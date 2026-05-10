//! Anthropic Claude API backend, implemented as a thin adapter over the
//! community Anthropic Rust SDK (`anthropic-client`).
//!
//! This adapter leans on the SDK's richer surface where it produces
//! observable user value: typed errors (`SwellError::LlmApiError`), typed
//! `LlmStopReason`, surfaced extended-thinking text, cache-TTL choice,
//! per-request `RequestOptions` overrides, structured-output via
//! `chat_typed::<T>`, and `discover_models()` for startup validation.
//!
//! `LlmRetryConfig::base_delay_secs` and `max_delay_secs` are not honoured
//! by this backend — retry timing is owned by the SDK. Only `max_retries`
//! is forwarded.

use crate::{
    credential::validate_anthropic_key, LlmBackend, LlmCacheTtl, LlmConfig, LlmMessage,
    LlmRequestOverrides, LlmResponse, LlmRetryConfig, LlmRole, LlmStopReason, LlmThinkingBlock,
    LlmToolChoice, LlmToolDefinition, LlmUsage,
};
use anthropic_client::ApiErrorKind as SdkApiErrorKind;
use anthropic_client::{
    CacheControl, CacheControlTtl, Client, ContentBlock as SdkContentBlock, ContentBlockParam,
    Error as SdkError, Message as SdkMessage, MessageCountTokensParams, MessageCreateParams,
    MessageParam, MessageStreamEvent, Model as SdkModel, ModelInfo, RequestOptions,
    StopReason as SdkStopReason, SystemPromptBlock, Tool as SdkTool, ToolChoice as SdkToolChoice,
};
use anthropic_types::ContentBlockDelta;
use async_trait::async_trait;
use futures::Stream;
use futures_util::StreamExt;
use opentelemetry::trace::{Span, Tracer};
use opentelemetry::KeyValue;
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::pin::Pin;
use std::time::Duration;
use swell_core::{
    opentelemetry::{gen_ai, pricing, GenAiSpanExt, LatencyTracker},
    record_llm_cost, LlmErrorKind, LlmToolCall, StreamEvent, SwellError,
};

pub use crate::providers::{AnthropicProvider, ProviderCaps};

#[derive(Debug, Clone)]
pub struct AnthropicBackend {
    model: String,
    /// Underlying SDK client (cheap to clone — `Arc` internally).
    client: Client,
    /// Provider-specific capability profile derived from `base_url`,
    /// settings pin, or explicit constructor argument.
    provider: AnthropicProvider,
    /// Effective capability flags. Starts as `provider.caps()` and may
    /// be further modified by per-field overrides from `.swell` settings
    /// (`llm.models.<alias>.caps`). Read by `build_params` to decide
    /// which request fields to forward.
    caps: ProviderCaps,
    /// Kept for compatibility; only `max_retries` is forwarded to the SDK.
    #[allow(dead_code)]
    retry_config: LlmRetryConfig,
}

impl AnthropicBackend {
    /// Construct a backend with default retry config and the standard
    /// Anthropic base URL.
    pub fn new(model: impl Into<String>, api_key: impl Into<String>) -> Self {
        let api_key = api_key.into();
        if let Err(e) = validate_anthropic_key(&api_key) {
            tracing::warn!(error = %e, "Anthropic backend created with potentially mismatched API key format");
        }
        Self::with_retry_config(model, api_key, LlmRetryConfig::default())
    }

    /// Construct a backend with a custom retry config.
    pub fn with_retry_config(
        model: impl Into<String>,
        api_key: impl Into<String>,
        retry_config: LlmRetryConfig,
    ) -> Self {
        let model = model.into();
        let client = Client::builder()
            .api_key(api_key.into())
            .max_retries(retry_config.max_retries)
            .build()
            .expect("Anthropic SDK client build (default base URL) should not fail");
        Self {
            model,
            client,
            provider: AnthropicProvider::Anthropic,
            caps: ProviderCaps::FULL,
            retry_config,
        }
    }

    /// Construct a backend pointed at a custom base URL (Anthropic-compatible
    /// gateways such as MiniMax). Default retry config.
    pub fn with_base_url(
        model: impl Into<String>,
        api_key: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Self {
        Self::with_base_url_and_retry(model, api_key, base_url, LlmRetryConfig::default())
    }

    /// Construct a backend with a custom base URL and a custom retry config.
    pub fn with_base_url_and_retry(
        model: impl Into<String>,
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        retry_config: LlmRetryConfig,
    ) -> Self {
        let model = model.into();
        let base_url = base_url.into();
        let provider = AnthropicProvider::detect(Some(&base_url));
        if provider != AnthropicProvider::Anthropic {
            tracing::info!(
                base_url = %base_url,
                provider = ?provider,
                "Detected Anthropic-compatible provider; some request fields will be filtered"
            );
        }
        let client = Client::builder()
            .api_key(api_key.into())
            .base_url(&base_url)
            .expect("Anthropic SDK rejected base URL")
            .max_retries(retry_config.max_retries)
            .build()
            .expect("Anthropic SDK client build should not fail");
        let caps = provider.caps();
        Self {
            model,
            client,
            provider,
            caps,
            retry_config,
        }
    }

    /// Which provider profile this backend is bound to. Set explicitly
    /// via `with_provider*` or auto-detected from `base_url`.
    pub fn provider(&self) -> &AnthropicProvider {
        &self.provider
    }

    /// Construct a backend with an explicit provider profile. Use this
    /// when `.swell` settings pin a provider — capability gating is
    /// driven by the explicit choice instead of URL substring matching.
    pub fn with_provider(
        model: impl Into<String>,
        api_key: impl Into<String>,
        base_url: Option<String>,
        provider: AnthropicProvider,
    ) -> Self {
        Self::with_provider_and_retry(
            model,
            api_key,
            base_url,
            provider,
            LlmRetryConfig::default(),
        )
    }

    /// Same as [`Self::with_provider`] with a custom retry config.
    pub fn with_provider_and_retry(
        model: impl Into<String>,
        api_key: impl Into<String>,
        base_url: Option<String>,
        provider: AnthropicProvider,
        retry_config: LlmRetryConfig,
    ) -> Self {
        let model = model.into();
        let mut builder = Client::builder()
            .api_key(api_key.into())
            .max_retries(retry_config.max_retries);
        if let Some(url) = base_url.as_deref() {
            builder = builder
                .base_url(url)
                .expect("Anthropic SDK rejected base URL");
        }
        let client = builder
            .build()
            .expect("Anthropic SDK client build should not fail");
        if provider != AnthropicProvider::Anthropic {
            tracing::info!(
                provider = provider.name(),
                base_url = ?base_url,
                "AnthropicBackend pinned to non-Anthropic provider profile"
            );
        }
        if provider == AnthropicProvider::Custom {
            tracing::warn!(
                base_url = ?base_url,
                "AnthropicBackend using Custom provider profile (full Anthropic surface). Pin a known provider in .swell `llm.models.<alias>.provider` for accurate capability gating."
            );
        }
        let caps = provider.caps();
        Self {
            model,
            client,
            provider,
            caps,
            retry_config,
        }
    }

    /// Construct a backend with an explicit provider profile **and**
    /// per-field capability overrides from `.swell` settings.
    ///
    /// Override semantics: each `Some(v)` in `caps_override` replaces
    /// the corresponding field in the provider's built-in profile;
    /// `None` keeps the built-in default. This is the path the daemon
    /// takes when a user writes `[llm.models.<alias>.caps]` in their
    /// settings — it lets unknown gateways opt into (or out of) features
    /// without us shipping a code change.
    pub fn with_provider_caps_and_retry(
        model: impl Into<String>,
        api_key: impl Into<String>,
        base_url: Option<String>,
        provider: AnthropicProvider,
        caps_override: &swell_core::llm_config::ProviderCapsOverride,
        retry_config: LlmRetryConfig,
    ) -> Self {
        let mut backend =
            Self::with_provider_and_retry(model, api_key, base_url, provider, retry_config);
        if !caps_override.is_empty() {
            backend.caps = backend.caps.with_override(caps_override);
            tracing::info!(
                provider = backend.provider.name(),
                "AnthropicBackend caps modified by user overrides from .swell settings"
            );
        }
        backend
    }

    /// The effective capability set this backend will use when building
    /// requests (provider profile + any user overrides).
    pub fn caps(&self) -> ProviderCaps {
        self.caps
    }

    /// Expose the retry config for diagnostics / tests.
    #[allow(dead_code)]
    pub(crate) fn retry_config(&self) -> &LlmRetryConfig {
        &self.retry_config
    }

    fn tracer(&self) -> impl Tracer {
        opentelemetry::global::tracer("swell-llm")
    }

    fn build_params(
        &self,
        messages: Vec<LlmMessage>,
        tools: Option<Vec<LlmToolDefinition>>,
        config: &LlmConfig,
    ) -> Result<MessageCreateParams, SwellError> {
        let model = SdkModel::from(self.model.clone());
        let caps = self.caps;

        let (system_text, conversation) = split_system(messages);
        let sdk_messages = convert_conversation(conversation)?;

        // MiniMax rejects temperature outside (0, 1]; clamp before sending.
        let temperature = if caps.clamp_temperature_unit {
            clamp_temperature_unit(config.temperature)
        } else {
            config.temperature as f64
        };

        let mut builder = MessageCreateParams::builder()
            .model(model)
            .max_tokens(saturating_u32(config.max_tokens))
            .messages(sdk_messages)
            .temperature(temperature)
            .map_err(|e| SwellError::LlmError(format!("invalid temperature: {e}")))?;

        if let Some(top_p) = config.top_p {
            builder = builder
                .top_p(top_p as f64)
                .map_err(|e| SwellError::LlmError(format!("invalid top_p: {e}")))?;
        }
        if let Some(top_k) = config.top_k {
            if caps.supports_top_k {
                builder = builder
                    .top_k(top_k)
                    .map_err(|e| SwellError::LlmError(format!("invalid top_k: {e}")))?;
            } else {
                tracing::debug!(provider = ?self.provider, "Dropping unsupported top_k");
            }
        }
        if let Some(stops) = config.stop_sequences.clone() {
            if caps.supports_stop_sequences {
                builder = builder.stop_sequences(stops);
            } else {
                tracing::debug!(provider = ?self.provider, "Dropping unsupported stop_sequences");
            }
        }
        if let Some(text) = system_text {
            if caps.supports_cache_control {
                // Cache the joined system prompt with the requested TTL.
                // Defaults to the API default (5-minute ephemeral).
                let cache_control = match config.cache_ttl {
                    Some(LlmCacheTtl::OneHour) => {
                        CacheControl::ephemeral_with_ttl(CacheControlTtl::OneHour)
                    }
                    Some(LlmCacheTtl::Ephemeral) | None => CacheControl::ephemeral(),
                };
                builder = builder.system_block(SystemPromptBlock::text_with_cache_control(
                    text,
                    cache_control,
                ));
            } else {
                // Compatible gateways drop cache_control silently. Send a
                // plain text system block so we don't waste a cache write
                // the upstream will ignore.
                builder = builder.system_block(SystemPromptBlock::text(text));
            }
        }
        if let Some(tools) = tools {
            if caps.supports_tool_use {
                for t in tools {
                    let tool = SdkTool::new(t.name, t.input_schema)
                        .map_err(|e| SwellError::LlmError(format!("invalid tool: {e}")))?
                        .description(t.description);
                    builder = builder.tool(tool);
                }
            } else {
                tracing::debug!(
                    provider = ?self.provider,
                    tool_count = tools.len(),
                    "Dropping tool definitions for provider that does not support tool_use"
                );
            }
        }
        if let Some(choice) = config.tool_choice.as_ref() {
            if caps.supports_tool_use {
                let sdk_choice = match choice {
                    LlmToolChoice::Auto => SdkToolChoice::auto(),
                    LlmToolChoice::Any => SdkToolChoice::any(),
                    LlmToolChoice::Tool { name } => SdkToolChoice::tool(name.clone())
                        .map_err(|e| SwellError::LlmError(format!("invalid tool name: {e}")))?,
                    LlmToolChoice::None => SdkToolChoice::none(),
                };
                builder = builder.tool_choice(sdk_choice);
            }
        }
        if let Some(budget) = config.thinking_budget_tokens {
            if caps.supports_thinking {
                builder = builder
                    .thinking_enabled(budget)
                    .map_err(|e| SwellError::LlmError(format!("invalid thinking budget: {e}")))?;
            } else {
                tracing::debug!(provider = ?self.provider, "Dropping unsupported thinking_budget");
            }
        }
        if let Some(user_id) = config.metadata_user_id.as_ref() {
            builder = builder.metadata(serde_json::json!({ "user_id": user_id }));
        }

        builder
            .build()
            .map_err(|e| SwellError::LlmError(format!("invalid Anthropic request: {e}")))
    }

    /// Translate `LlmRequestOverrides` into the SDK's `RequestOptions`.
    fn build_request_options(
        overrides: Option<&LlmRequestOverrides>,
    ) -> Result<RequestOptions, SwellError> {
        let Some(o) = overrides else {
            return Ok(RequestOptions::new());
        };
        let mut b = RequestOptions::builder();
        if let Some(ms) = o.timeout_ms {
            b = b.timeout(Duration::from_millis(ms));
        }
        if let Some(r) = o.max_retries {
            b = b.max_retries(r);
        }
        for beta in &o.betas {
            b = b.header("anthropic-beta", beta);
        }
        b.build()
            .map_err(|e| SwellError::LlmError(format!("invalid request overrides: {e}")))
    }

    /// List models the configured backend exposes via `/v1/models`. Used by
    /// startup wiring to warn on unknown configured models. Returns an empty
    /// vec on transport error rather than failing — Anthropic-compatible
    /// gateways may not implement the endpoint.
    pub async fn discover_models(&self) -> Result<Vec<ModelInfo>, SwellError> {
        let mut out = Vec::new();
        let mut stream = self.client.models().list_auto_paging();
        while let Some(item) = stream.next().await {
            match item {
                Ok(info) => out.push(info),
                Err(e) => return Err(map_sdk_error(e)),
            }
        }
        Ok(out)
    }

    /// Compare the configured model against `discover_models()` and emit a
    /// startup `tracing::warn!` if it isn't listed. Best-effort — gateways
    /// without `/v1/models` (or transient outages) silently no-op.
    pub async fn warn_if_unknown_model(&self) {
        if !self.caps.supports_models_listing {
            tracing::info!(
                provider = %self.provider.name(),
                "skipping /v1/models listing — provider declared listing unsupported"
            );
            return;
        }
        let models = match self.discover_models().await {
            Ok(m) if m.is_empty() => return,
            Ok(m) => m,
            Err(e) => {
                tracing::debug!(error = %e, "discover_models unavailable; skipping startup model check");
                return;
            }
        };
        let configured = self.model.as_str();
        let known = models.iter().any(|m| m.id == configured);
        if !known {
            let available: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
            tracing::warn!(
                configured_model = %configured,
                available = ?available,
                "Configured Anthropic model is not in the API's /v1/models listing — typo or deprecated?"
            );
        }
    }

    /// Structured-output convenience wrapping the SDK's `create_and_parse`.
    /// Caller is responsible for shaping `messages`/`config` so the response
    /// is JSON parseable into `T`.
    pub async fn chat_typed<T>(
        &self,
        messages: Vec<LlmMessage>,
        tools: Option<Vec<LlmToolDefinition>>,
        config: LlmConfig,
    ) -> Result<T, SwellError>
    where
        T: DeserializeOwned,
    {
        let params = self.build_params(messages, tools, &config)?;
        let options = Self::build_request_options(config.request_overrides.as_ref())?;
        self.client
            .messages()
            .create_and_parse_with::<T>(params, options)
            .await
            .map_err(map_sdk_error)
    }
}

#[async_trait]
impl LlmBackend for AnthropicBackend {
    fn model(&self) -> &str {
        &self.model
    }

    async fn chat(
        &self,
        messages: Vec<LlmMessage>,
        tools: Option<Vec<LlmToolDefinition>>,
        config: LlmConfig,
    ) -> Result<LlmResponse, SwellError> {
        let params = self.build_params(messages, tools, &config)?;
        let options = Self::build_request_options(config.request_overrides.as_ref())?;

        let latency = LatencyTracker::new();
        let message: SdkMessage = self
            .client
            .messages()
            .create_with(params, options)
            .await
            .map_err(map_sdk_error)?;
        let latency_ms = latency.elapsed_ms();

        let (content, tool_calls, thinking, thinking_blocks) = collect_response_content(&message);
        let usage = build_usage(&message.usage);

        let tracer = self.tracer();
        let mut span_builder = tracer.span_builder(format!("Anthropic chat {}", self.model));
        span_builder.attributes = Some(vec![
            KeyValue::new(gen_ai::OPERATION_NAME, "chat".to_string()),
            KeyValue::new(gen_ai::PROVIDER_NAME, "anthropic".to_string()),
            KeyValue::new(gen_ai::REQUEST_MODEL, self.model.clone()),
        ]);
        let mut span = tracer.build(span_builder);
        span.record_prompt_tokens(usage.input_tokens);
        span.record_completion_tokens(usage.output_tokens);
        span.record_latency_ms(latency_ms);
        span.record_response_model(message.model.as_str());
        let pricing = pricing::for_model(&self.model);
        let cost = pricing.calculate_cost(usage.input_tokens, usage.output_tokens);
        span.record_cost_usd(cost);
        span.end();

        record_llm_cost(usage.input_tokens + usage.output_tokens, &self.model);

        Ok(LlmResponse {
            content,
            tool_calls: if tool_calls.is_empty() {
                None
            } else {
                Some(tool_calls)
            },
            usage,
            stop_reason: message.stop_reason.as_ref().map(convert_stop_reason),
            thinking,
            thinking_blocks,
        })
    }

    async fn health_check(&self) -> bool {
        let params = MessageCountTokensParams::builder()
            .model(SdkModel::from(self.model.clone()))
            .message(MessageParam::user("ping"))
            .build();
        let params = match params {
            Ok(p) => p,
            Err(_) => return false,
        };
        self.client.messages().count_tokens(params).await.is_ok()
    }

    async fn stream(
        &self,
        messages: Vec<LlmMessage>,
        tools: Option<Vec<LlmToolDefinition>>,
        config: LlmConfig,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, SwellError>> + Send>>, SwellError>
    {
        let params = self.build_params(messages, tools, &config)?;
        let options = Self::build_request_options(config.request_overrides.as_ref())?;
        let sdk_stream = self
            .client
            .messages()
            .create_stream_with(params, options)
            .await
            .map_err(map_sdk_error)?;

        Ok(Box::pin(StreamAdapter::new(sdk_stream)))
    }
}

// ============================================================================
// Conversion: swell -> SDK request types
// ============================================================================

fn split_system(messages: Vec<LlmMessage>) -> (Option<String>, Vec<LlmMessage>) {
    let mut system_texts: Vec<String> = Vec::new();
    let mut convo: Vec<LlmMessage> = Vec::new();
    for m in messages {
        if m.role == LlmRole::System {
            if !m.content.is_empty() {
                system_texts.push(m.content);
            }
        } else {
            convo.push(m);
        }
    }
    let system = if system_texts.is_empty() {
        None
    } else {
        Some(system_texts.join("\n\n"))
    };
    (system, convo)
}

fn convert_conversation(messages: Vec<LlmMessage>) -> Result<Vec<MessageParam>, SwellError> {
    messages
        .into_iter()
        .map(convert_message)
        .collect::<Result<Vec<_>, _>>()
}

fn convert_message(m: LlmMessage) -> Result<MessageParam, SwellError> {
    match m.role {
        // Assistant turn that needs to round-trip thinking blocks and/or
        // tool_use blocks. Per Anthropic's contract — and explicitly per
        // MiniMax's docs — thinking blocks must come FIRST in the block
        // list to keep the reasoning chain coherent. Order: thinking →
        // text → tool_use.
        LlmRole::Assistant
            if !m.thinking_blocks.is_empty()
                || m.tool_calls.as_ref().is_some_and(|tc| !tc.is_empty()) =>
        {
            let mut blocks: Vec<ContentBlockParam> = Vec::new();
            for tb in m.thinking_blocks {
                let block = match tb.signature {
                    Some(sig) => ContentBlockParam::thinking_with_signature(tb.thinking, sig),
                    None => ContentBlockParam::thinking(tb.thinking),
                };
                blocks.push(block);
            }
            if !m.content.is_empty() {
                blocks.push(ContentBlockParam::text(m.content));
            }
            for tc in m.tool_calls.unwrap_or_default() {
                let block = ContentBlockParam::tool_use(tc.id, tc.name, tc.arguments)
                    .map_err(|e| SwellError::LlmError(format!("invalid tool_use block: {e}")))?;
                blocks.push(block);
            }
            Ok(MessageParam::assistant_blocks(blocks))
        }
        LlmRole::Assistant => Ok(MessageParam::assistant(m.content)),
        LlmRole::User if m.tool_call_id.is_some() => {
            let tool_use_id = m.tool_call_id.expect("checked Some above");
            if m.tool_result_is_error {
                Ok(MessageParam::tool_result_error(tool_use_id, m.content))
            } else {
                Ok(MessageParam::tool_result(tool_use_id, m.content))
            }
        }
        LlmRole::User => Ok(MessageParam::user(m.content)),
        LlmRole::System => unreachable!("system messages are split out before conversion"),
    }
}

/// MiniMax constrains `temperature` to (0, 1]; values outside that range
/// return an error from the API. Snap to the nearest valid value: `0.0`
/// (or NaN/negative) becomes a tiny epsilon, anything > 1 becomes `1.0`.
fn clamp_temperature_unit(t: f32) -> f64 {
    let v = t as f64;
    if !v.is_finite() || v <= 0.0 {
        f64::EPSILON
    } else if v > 1.0 {
        1.0
    } else {
        v
    }
}

fn saturating_u32(value: u64) -> u32 {
    if value > u32::MAX as u64 {
        u32::MAX
    } else {
        value as u32
    }
}

// ============================================================================
// Conversion: SDK response -> swell types
// ============================================================================

fn collect_response_content(
    message: &SdkMessage,
) -> (
    String,
    Vec<LlmToolCall>,
    Option<String>,
    Vec<LlmThinkingBlock>,
) {
    let mut content = String::new();
    let mut tool_calls = Vec::new();
    let mut thinking_concat = String::new();
    let mut thinking_blocks: Vec<LlmThinkingBlock> = Vec::new();
    for block in &message.content {
        match block {
            SdkContentBlock::Text { text, .. } => content.push_str(text),
            SdkContentBlock::ToolUse { id, name, input } => {
                tool_calls.push(LlmToolCall {
                    id: id.clone(),
                    name: name.clone(),
                    arguments: input.clone(),
                });
            }
            SdkContentBlock::Thinking {
                thinking: t,
                signature,
            } => {
                thinking_concat.push_str(t);
                thinking_blocks.push(LlmThinkingBlock {
                    thinking: t.clone(),
                    signature: signature.clone(),
                });
            }
            // RedactedThinking and server-tool blocks are not surfaced today.
            _ => {}
        }
    }
    let thinking = if thinking_concat.is_empty() {
        None
    } else {
        Some(thinking_concat)
    };
    (content, tool_calls, thinking, thinking_blocks)
}

fn convert_stop_reason(r: &SdkStopReason) -> LlmStopReason {
    match r {
        SdkStopReason::EndTurn => LlmStopReason::EndTurn,
        SdkStopReason::MaxTokens => LlmStopReason::MaxTokens,
        SdkStopReason::StopSequence => LlmStopReason::StopSequence,
        SdkStopReason::ToolUse => LlmStopReason::ToolUse,
        SdkStopReason::PauseTurn => LlmStopReason::PauseTurn,
        SdkStopReason::Refusal => LlmStopReason::Refusal,
        SdkStopReason::Other(value) => LlmStopReason::Other(value.clone()),
    }
}

fn build_usage(u: &anthropic_client::Usage) -> LlmUsage {
    let server_tool_use_count = u
        .server_tool_use
        .as_ref()
        .map(|s| u64::from(s.web_fetch_requests).saturating_add(u64::from(s.web_search_requests)));
    let (cache_5m, cache_1h) = u
        .cache_creation
        .as_ref()
        .map(|c| {
            (
                Some(u64::from(c.ephemeral_5m_input_tokens)),
                Some(u64::from(c.ephemeral_1h_input_tokens)),
            )
        })
        .unwrap_or((None, None));
    LlmUsage {
        input_tokens: u.input_tokens as u64,
        output_tokens: u.output_tokens as u64,
        total_tokens: (u.input_tokens + u.output_tokens) as u64,
        cache_creation_input_tokens: u.cache_creation_input_tokens.map(|v| v as u64),
        cache_read_input_tokens: u.cache_read_input_tokens.map(|v| v as u64),
        cache_creation_5m_input_tokens: cache_5m,
        cache_creation_1h_input_tokens: cache_1h,
        server_tool_use_count,
        service_tier: u.service_tier.as_ref().map(|t| t.as_str().to_owned()),
    }
}

fn map_sdk_error(e: SdkError) -> SwellError {
    match e {
        SdkError::Api(api) => SwellError::LlmApiError {
            kind: convert_api_error_kind(&api.kind),
            status: api.status.as_u16(),
            request_id: api.request_id().map(|s| s.to_owned()),
            message: api.message.clone(),
        },
        other => SwellError::LlmError(format!("Anthropic SDK error: {other}")),
    }
}

fn convert_api_error_kind(k: &SdkApiErrorKind) -> LlmErrorKind {
    match k {
        SdkApiErrorKind::InvalidRequest => LlmErrorKind::InvalidRequest,
        SdkApiErrorKind::Authentication => LlmErrorKind::Authentication,
        SdkApiErrorKind::Permission => LlmErrorKind::Permission,
        SdkApiErrorKind::NotFound => LlmErrorKind::NotFound,
        SdkApiErrorKind::Conflict => LlmErrorKind::Conflict,
        SdkApiErrorKind::UnprocessableEntity => LlmErrorKind::UnprocessableEntity,
        SdkApiErrorKind::RateLimit => LlmErrorKind::RateLimit,
        SdkApiErrorKind::InternalServer => LlmErrorKind::InternalServer,
        SdkApiErrorKind::Overloaded => LlmErrorKind::Overloaded,
        SdkApiErrorKind::Unknown(value) => LlmErrorKind::Unknown(value.clone()),
    }
}

// ============================================================================
// Streaming adapter: MessageStreamEvent -> StreamEvent
// ============================================================================

struct ToolUseAccumulator {
    id: String,
    name: String,
    args: String,
}

#[derive(Default)]
struct ThinkingAccumulator {
    text: String,
    signature: Option<String>,
}

struct StreamAdapter<S> {
    inner: S,
    accumulated_text: String,
    tool_uses: HashMap<u32, ToolUseAccumulator>,
    thinking_blocks: HashMap<u32, ThinkingAccumulator>,
    pending: std::collections::VecDeque<Result<StreamEvent, SwellError>>,
    last_stop_reason: Option<LlmStopReason>,
    finished: bool,
}

impl<S> StreamAdapter<S> {
    fn new(inner: S) -> Self {
        Self {
            inner,
            accumulated_text: String::new(),
            tool_uses: HashMap::new(),
            thinking_blocks: HashMap::new(),
            pending: std::collections::VecDeque::new(),
            last_stop_reason: None,
            finished: false,
        }
    }

    fn translate(&mut self, event: MessageStreamEvent) {
        match event {
            MessageStreamEvent::ContentBlockStart {
                index,
                content_block,
            } => match content_block {
                SdkContentBlock::ToolUse { id, name, .. } => {
                    self.tool_uses.insert(
                        index,
                        ToolUseAccumulator {
                            id,
                            name,
                            args: String::new(),
                        },
                    );
                }
                SdkContentBlock::Thinking {
                    thinking,
                    signature,
                } => {
                    // ContentBlockStart for thinking may carry an initial
                    // text+signature payload. Seed the accumulator.
                    self.thinking_blocks.insert(
                        index,
                        ThinkingAccumulator {
                            text: thinking,
                            signature,
                        },
                    );
                }
                _ => {}
            },
            MessageStreamEvent::ContentBlockDelta { index, delta } => match delta {
                ContentBlockDelta::Text { text } => {
                    self.accumulated_text.push_str(&text);
                    self.pending.push_back(Ok(StreamEvent::TextDelta {
                        text: self.accumulated_text.clone(),
                        delta: text,
                    }));
                }
                ContentBlockDelta::InputJson { partial_json } => {
                    if let Some(acc) = self.tool_uses.get_mut(&index) {
                        acc.args.push_str(&partial_json);
                    }
                }
                ContentBlockDelta::Thinking { thinking } => {
                    self.thinking_blocks
                        .entry(index)
                        .or_default()
                        .text
                        .push_str(&thinking);
                    self.pending
                        .push_back(Ok(StreamEvent::ThinkingDelta { text: thinking }));
                }
                ContentBlockDelta::Signature { signature } => {
                    self.thinking_blocks.entry(index).or_default().signature = Some(signature);
                }
                _ => {}
            },
            MessageStreamEvent::ContentBlockStop { index } => {
                if let Some(acc) = self.tool_uses.remove(&index) {
                    let arguments = if acc.args.is_empty() {
                        serde_json::json!({})
                    } else {
                        serde_json::from_str(&acc.args).unwrap_or(serde_json::json!({}))
                    };
                    self.pending.push_back(Ok(StreamEvent::ToolUse {
                        tool_call: LlmToolCall {
                            id: acc.id,
                            name: acc.name,
                            arguments,
                        },
                    }));
                }
                if let Some(acc) = self.thinking_blocks.remove(&index) {
                    self.pending
                        .push_back(Ok(StreamEvent::ThinkingBlockComplete {
                            thinking: acc.text,
                            signature: acc.signature,
                        }));
                }
            }
            MessageStreamEvent::MessageDelta { delta, usage } => {
                if let Some(reason) = delta.stop_reason {
                    self.last_stop_reason = Some(convert_stop_reason(&reason));
                }
                if let Some(u) = usage {
                    self.pending.push_back(Ok(StreamEvent::Usage {
                        input_tokens: u.input_tokens.unwrap_or(0) as u64,
                        output_tokens: u.output_tokens as u64,
                        cache_creation_input_tokens: u
                            .cache_creation_input_tokens
                            .map(|v| v as u64),
                        cache_read_input_tokens: u.cache_read_input_tokens.map(|v| v as u64),
                    }));
                }
            }
            MessageStreamEvent::MessageStart { message } => {
                let u = &message.usage;
                if u.input_tokens != 0
                    || u.cache_creation_input_tokens.is_some()
                    || u.cache_read_input_tokens.is_some()
                {
                    self.pending.push_back(Ok(StreamEvent::Usage {
                        input_tokens: u.input_tokens as u64,
                        output_tokens: u.output_tokens as u64,
                        cache_creation_input_tokens: u
                            .cache_creation_input_tokens
                            .map(|v| v as u64),
                        cache_read_input_tokens: u.cache_read_input_tokens.map(|v| v as u64),
                    }));
                }
            }
            MessageStreamEvent::MessageStop => {
                let mut leftover_tools: Vec<_> = self.tool_uses.drain().collect();
                leftover_tools.sort_by_key(|(idx, _)| *idx);
                for (_, acc) in leftover_tools {
                    let arguments = if acc.args.is_empty() {
                        serde_json::json!({})
                    } else {
                        serde_json::from_str(&acc.args).unwrap_or(serde_json::json!({}))
                    };
                    self.pending.push_back(Ok(StreamEvent::ToolUse {
                        tool_call: LlmToolCall {
                            id: acc.id,
                            name: acc.name,
                            arguments,
                        },
                    }));
                }
                // Defensive: flush any thinking blocks that never received a
                // ContentBlockStop (some compatible gateways batch them).
                let mut leftover_thinking: Vec<_> = self.thinking_blocks.drain().collect();
                leftover_thinking.sort_by_key(|(idx, _)| *idx);
                for (_, acc) in leftover_thinking {
                    self.pending
                        .push_back(Ok(StreamEvent::ThinkingBlockComplete {
                            thinking: acc.text,
                            signature: acc.signature,
                        }));
                }
                self.pending.push_back(Ok(StreamEvent::MessageStop {
                    stop_reason: self.last_stop_reason.take(),
                }));
                self.finished = true;
            }
            MessageStreamEvent::Error { error } => {
                self.pending.push_back(Err(SwellError::LlmError(format!(
                    "Anthropic stream error: {} ({})",
                    error.message,
                    error.error_type.as_str()
                ))));
            }
            MessageStreamEvent::Ping | MessageStreamEvent::Other { .. } => {}
        }
    }
}

impl<S> Stream for StreamAdapter<S>
where
    S: Stream<Item = Result<MessageStreamEvent, SdkError>> + Send + Unpin,
{
    type Item = Result<StreamEvent, SwellError>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        loop {
            if let Some(event) = self.pending.pop_front() {
                return std::task::Poll::Ready(Some(event));
            }
            if self.finished {
                return std::task::Poll::Ready(None);
            }
            match self.inner.poll_next_unpin(cx) {
                std::task::Poll::Pending => return std::task::Poll::Pending,
                std::task::Poll::Ready(None) => {
                    self.finished = true;
                    return std::task::Poll::Ready(None);
                }
                std::task::Poll::Ready(Some(Err(e))) => {
                    return std::task::Poll::Ready(Some(Err(map_sdk_error(e))));
                }
                std::task::Poll::Ready(Some(Ok(event))) => {
                    self.translate(event);
                }
            }
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_records_model_name() {
        let backend = AnthropicBackend::new("claude-sonnet-4-5", "sk-ant-test");
        assert_eq!(backend.model(), "claude-sonnet-4-5");
    }

    #[test]
    fn with_base_url_uses_custom_endpoint() {
        let backend = AnthropicBackend::with_base_url(
            "claude-sonnet-4-5",
            "sk-ant-test",
            "https://example.test",
        );
        assert_eq!(backend.model(), "claude-sonnet-4-5");
    }

    #[test]
    fn retry_config_is_stored() {
        let cfg = LlmRetryConfig {
            max_retries: 5,
            base_delay_secs: 1.0,
            max_delay_secs: 60.0,
        };
        let backend =
            AnthropicBackend::with_retry_config("claude-sonnet-4-5", "sk-ant-test", cfg.clone());
        assert_eq!(backend.retry_config().max_retries, 5);
    }

    #[test]
    fn split_system_pulls_system_messages_out() {
        let msgs = vec![
            LlmMessage {
                role: LlmRole::System,
                content: "you are helpful".into(),
                ..Default::default()
            },
            LlmMessage {
                role: LlmRole::User,
                content: "hi".into(),
                ..Default::default()
            },
            LlmMessage {
                role: LlmRole::System,
                content: "be terse".into(),
                ..Default::default()
            },
        ];
        let (system, convo) = split_system(msgs);
        assert_eq!(system.as_deref(), Some("you are helpful\n\nbe terse"));
        assert_eq!(convo.len(), 1);
    }

    #[test]
    fn convert_message_handles_tool_results() {
        let m = LlmMessage {
            role: LlmRole::User,
            content: "result body".into(),
            tool_call_id: Some("toolu_abc".into()),
            ..Default::default()
        };
        let param = convert_message(m).expect("convert");
        let value = serde_json::to_value(&param).unwrap();
        assert_eq!(value["role"], "user");
        assert_eq!(value["content"][0]["type"], "tool_result");
        assert_eq!(value["content"][0]["tool_use_id"], "toolu_abc");
    }

    #[test]
    fn convert_message_handles_assistant_with_tool_calls() {
        let m = LlmMessage {
            role: LlmRole::Assistant,
            content: "I'll check the weather.".into(),
            tool_calls: Some(vec![LlmToolCall {
                id: "toolu_1".into(),
                name: "get_weather".into(),
                arguments: serde_json::json!({ "city": "Paris" }),
            }]),
            ..Default::default()
        };
        let param = convert_message(m).expect("convert");
        let value = serde_json::to_value(&param).unwrap();
        assert_eq!(value["role"], "assistant");
        assert_eq!(value["content"][0]["type"], "text");
        assert_eq!(value["content"][1]["type"], "tool_use");
        assert_eq!(value["content"][1]["name"], "get_weather");
        assert_eq!(value["content"][1]["input"]["city"], "Paris");
    }

    #[test]
    fn saturating_u32_clamps_max_tokens() {
        assert_eq!(saturating_u32(100), 100);
        assert_eq!(saturating_u32(u64::MAX), u32::MAX);
    }

    #[test]
    fn provider_detection_picks_minimax_from_base_url() {
        let backend = AnthropicBackend::with_base_url(
            "MiniMax-M2.7",
            "sk-test",
            "https://api.minimax.io/anthropic",
        );
        assert_eq!(backend.provider(), &AnthropicProvider::MiniMax);
        let caps = backend.provider().caps();
        assert!(!caps.supports_top_k);
        assert!(!caps.supports_stop_sequences);
        assert!(!caps.supports_cache_control);
        assert!(caps.clamp_temperature_unit);
    }

    #[test]
    fn provider_defaults_to_anthropic_for_default_base_url() {
        let backend = AnthropicBackend::new("claude-sonnet-4-5", "sk-ant-test");
        assert_eq!(backend.provider(), &AnthropicProvider::Anthropic);
    }

    #[test]
    fn provider_detects_anthropic_from_anthropic_url() {
        // URLs containing "anthropic" route to the native profile.
        let backend = AnthropicBackend::with_base_url(
            "claude-sonnet-4-5",
            "sk-test",
            "https://api.anthropic.com",
        );
        assert_eq!(backend.provider(), &AnthropicProvider::Anthropic);
    }

    #[test]
    fn provider_unknown_gateway_falls_back_to_custom() {
        let backend =
            AnthropicBackend::with_base_url("some-model", "sk-test", "https://example.com/v1");
        assert_eq!(backend.provider(), &AnthropicProvider::Custom);
    }

    #[test]
    fn provider_explicit_override_wins_over_url() {
        let backend = AnthropicBackend::with_provider(
            "kimi-k2",
            "sk-test",
            Some("https://api.example.com".to_string()),
            AnthropicProvider::Moonshot,
        );
        assert_eq!(backend.provider(), &AnthropicProvider::Moonshot);
    }

    #[test]
    fn clamp_temperature_unit_snaps_to_valid_range() {
        assert!(clamp_temperature_unit(0.0) > 0.0);
        assert!(clamp_temperature_unit(-0.5) > 0.0);
        assert_eq!(clamp_temperature_unit(0.5), 0.5);
        assert_eq!(clamp_temperature_unit(1.0), 1.0);
        assert_eq!(clamp_temperature_unit(1.7), 1.0);
        assert_eq!(clamp_temperature_unit(f32::NAN), f64::EPSILON);
    }

    #[test]
    fn convert_stop_reason_round_trips_other() {
        let r = SdkStopReason::Other("provider_custom".to_owned());
        match convert_stop_reason(&r) {
            LlmStopReason::Other(v) => assert_eq!(v, "provider_custom"),
            _ => panic!("expected Other variant"),
        }
    }
}
