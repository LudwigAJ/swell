//! Anthropic Claude API backend with OpenTelemetry instrumentation.
//!
//! # Prompt Caching
//!
//! This backend supports Anthropic's prompt caching feature. When system messages
//! are provided, they are sent with `cache_control: { type: "ephemeral" }` to enable
//! provider-managed cache creation. The API response includes `cache_creation_input_tokens`
//! (tokens written to cache) and `cache_read_input_tokens` (tokens read from cache).

use crate::{
    calculate_backoff, credential::validate_anthropic_key, is_retryable_status, LlmBackend,
    LlmConfig, LlmMessage, LlmResponse, LlmRetryConfig, LlmRole, LlmToolDefinition, LlmUsage,
};
use async_trait::async_trait;
use futures::{Stream, StreamExt};
use opentelemetry::trace::{Span, Tracer};
use opentelemetry::KeyValue;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::time::Duration;
use swell_core::{
    opentelemetry::{gen_ai, pricing, GenAiSpanExt, LatencyTracker},
    record_llm_cost, LlmToolCall, StreamEvent, SwellError,
};
use tokio::sync::mpsc;
use tokio::time::sleep;
use tracing::{debug, warn};

/// Default base URL for Anthropic API
const DEFAULT_ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";
/// Beta header for Anthropic prompt caching feature
const PROMPT_CACHING_BETA: &str = "prompt-caching-2024-07-31";

// ============================================================================
// API Request/Response Types
// ============================================================================

/// API message with support for prompt caching via cache_control.
/// For system messages, content is sent as a content block array with cache_control.
#[derive(Serialize)]
#[serde(untagged)]
enum ApiMessage {
    Simple {
        role: &'static str,
        content: String,
    },
    SystemWithCache {
        role: &'static str,
        content: Vec<SystemContentBlock>,
    },
}

/// System content block with cache control for prompt caching.
#[derive(Serialize)]
struct SystemContentBlock {
    r#type: String,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<CacheControl>,
}

#[derive(Serialize)]
struct CacheControl {
    r#type: String,
}

#[derive(Serialize)]
struct ApiTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct Response {
    content: Vec<ResponseContent>,
    usage: ResponseUsage,
    #[serde(default)]
    #[allow(dead_code)]
    id: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ResponseContent {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        name: String,
        input: serde_json::Value,
        id: String,
    },
}

/// Response usage with four-dimensional token tracking (Anthropic).
#[derive(Debug, Deserialize)]
struct ResponseUsage {
    input_tokens: u64,
    output_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: Option<u64>,
    #[serde(default)]
    cache_read_input_tokens: Option<u64>,
}

// ============================================================================
// Backend Implementation
// ============================================================================

#[derive(Debug, Clone)]
pub struct AnthropicBackend {
    model: String,
    api_key: String,
    base_url: String,
    client: Client,
    retry_config: LlmRetryConfig,
}

impl AnthropicBackend {
    pub fn new(model: impl Into<String>, api_key: impl Into<String>) -> Self {
        let api_key = api_key.into();
        // Validate API key format to detect OpenAI keys being used with Anthropic backend
        if let Err(e) = validate_anthropic_key(&api_key) {
            tracing::warn!(error = %e, "Anthropic backend created with potentially mismatched API key format");
        }
        Self::with_retry_config(model, api_key, LlmRetryConfig::default())
    }

    pub fn with_retry_config(
        model: impl Into<String>,
        api_key: impl Into<String>,
        retry_config: LlmRetryConfig,
    ) -> Self {
        Self {
            model: model.into(),
            api_key: api_key.into(),
            base_url: DEFAULT_ANTHROPIC_BASE_URL.to_string(),
            client: Client::new(),
            retry_config,
        }
    }

    /// Create a new backend with a custom base URL.
    /// This is useful for proxy services that implement the Anthropic API protocol.
    pub fn with_base_url(
        model: impl Into<String>,
        api_key: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Self {
        Self::with_base_url_and_retry(model, api_key, base_url, LlmRetryConfig::default())
    }

    /// Create a new backend with custom base URL and retry config.
    pub fn with_base_url_and_retry(
        model: impl Into<String>,
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        retry_config: LlmRetryConfig,
    ) -> Self {
        Self {
            model: model.into(),
            api_key: api_key.into(),
            base_url: base_url.into(),
            client: Client::new(),
            retry_config,
        }
    }

    fn convert_role(role: LlmRole) -> &'static str {
        match role {
            LlmRole::System => "system",
            LlmRole::User => "user",
            LlmRole::Assistant => "assistant",
        }
    }

    /// Get the tracer for OpenTelemetry
    fn tracer(&self) -> impl Tracer {
        opentelemetry::global::tracer("swell-llm")
    }

    /// Execute a streaming chat completion request.
    ///
    /// Returns a stream of [`StreamEvent`]s that can be used to process
    /// the response in real-time as tokens are generated.
    ///
    /// # SSE Format
    ///
    /// Anthropic returns events in SSE format:
    /// ```ignore
    /// event: content_block_start\ndata: {"type":"text","index":0}\n\n
    /// event: content_block_delta\ndata: {"type":"text_delta","text":"Hello"}\n\n
    /// event: message_delta\ndata: {"type":"message_delta","usage":{"output_tokens":10,"stop_sequence":null,"stop_reason":"end_turn"}}\n\n
    /// event: message_stop\ndata: {"type":"message_stop"}\n\n
    /// ```
    ///
    /// For tool calls:
    /// ```ignore
    /// event: content_block_start\ndata: {"type":"tool_use","index":0,"name":"get_weather","id":"toolu_...","input":{}}\n\n
    /// event: content_block_delta\ndata: {"type":"input_json_delta","partial_json":"{\"location\": \"Los"}\n\n
    /// event: content_block_delta\ndata: {"type":"input_json_delta","partial_json":" Angeles\"}"}\n\n
    /// event: message_stop\ndata: {"type":"message_stop"}\n\n
    /// ```
    pub async fn stream(
        &self,
        messages: Vec<LlmMessage>,
        tools: Option<Vec<LlmToolDefinition>>,
        config: LlmConfig,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, SwellError>> + Send>>, SwellError>
    {
        #[derive(Serialize)]
        struct Request<'a> {
            model: &'a str,
            messages: Vec<ApiMessage>,
            #[serde(skip_serializing_if = "Option::is_none")]
            tools: Option<Vec<ApiTool>>,
            max_tokens: u64,
            temperature: f32,
            #[serde(skip_serializing_if = "Option::is_none")]
            stop_sequences: Option<Vec<String>>,
            stream: bool,
        }

        // Prepare request data - convert messages with cache support for system messages
        let api_messages: Vec<ApiMessage> = messages
            .into_iter()
            .map(|m| {
                if m.role == LlmRole::System && !m.content.is_empty() {
                    // System messages use cache_control for prompt caching
                    ApiMessage::SystemWithCache {
                        role: "system",
                        content: vec![SystemContentBlock {
                            r#type: "text".to_string(),
                            text: m.content,
                            cache_control: Some(CacheControl {
                                r#type: "ephemeral".to_string(),
                            }),
                        }],
                    }
                } else {
                    ApiMessage::Simple {
                        role: Self::convert_role(m.role),
                        content: m.content,
                    }
                }
            })
            .collect();

        let api_tools = tools.map(|tools| {
            tools
                .into_iter()
                .map(|t| ApiTool {
                    name: t.name,
                    description: t.description,
                    input_schema: t.input_schema,
                })
                .collect()
        });

        let request = Request {
            model: &self.model,
            messages: api_messages,
            tools: api_tools,
            max_tokens: config.max_tokens,
            temperature: config.temperature,
            stop_sequences: config.stop_sequences,
            stream: true,
        };

        debug!(model = %self.model, "Anthropic streaming request");

        let url = format!("{}/v1/messages", self.base_url);

        // Make HTTP request with streaming
        let response = match self
            .client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("anthropic-beta", PROMPT_CACHING_BETA)
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                return Err(SwellError::LlmError(format!("Request failed: {}", e)));
            }
        };

        // Check status
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            warn!(status = %status, body = %body, "Anthropic API error");
            return Err(SwellError::LlmError(format!(
                "API error {}: {}",
                status, body
            )));
        }

        let (tx, rx) = mpsc::channel::<Result<StreamEvent, SwellError>>(100);

        // Spawn a task to process the SSE stream
        tokio::spawn(async move {
            let mut parser = AnthropicSseParser::new();

            // Use reqwest's bytes_stream and convert to strings line by line
            let mut byte_stream = response.bytes_stream();

            // Accumulator for incomplete SSE lines
            let mut line_buffer = String::new();

            while let Some(chunk_result) = byte_stream.next().await {
                match chunk_result {
                    Ok(bytes) => {
                        // Convert bytes to string, handling UTF-8 boundaries
                        let chunk_str = match String::from_utf8(bytes.to_vec()) {
                            Ok(s) => s,
                            Err(e) => {
                                let _ = tx
                                    .send(Err(SwellError::LlmError(format!(
                                        "Invalid UTF-8 in stream: {}",
                                        e
                                    ))))
                                    .await;
                                break;
                            }
                        };

                        // Process character by character to handle line boundaries
                        for c in chunk_str.chars() {
                            if c == '\n' {
                                // Process complete line
                                let line = line_buffer.trim();
                                if !line.is_empty() {
                                    // Parse the data line - event type is encoded in JSON via #[serde(tag)]
                                    // Note: we ignore the SSE "event: " line type since JSON has the real type
                                    if let Some(data) = line.strip_prefix("data: ") {
                                        match parser.parse_data_line(data.trim(), None) {
                                            Ok(Some(event)) => {
                                                if tx.send(Ok(event)).await.is_err() {
                                                    // Receiver dropped, stop processing
                                                    return;
                                                }
                                            }
                                            Ok(None) => {
                                                // Line processed but no event
                                            }
                                            Err(e) => {
                                                let _ = tx.send(Err(e)).await;
                                                return;
                                            }
                                        }
                                    }
                                }
                                line_buffer.clear();
                            } else if c != '\r' {
                                // Ignore carriage returns, accumulate other characters
                                line_buffer.push(c);
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx
                            .send(Err(SwellError::LlmError(format!(
                                "Stream read error: {}",
                                e
                            ))))
                            .await;
                        break;
                    }
                }
            }

            // Send message stop event
            let _ = tx
                .send(Ok(StreamEvent::MessageStop { stop_reason: None }))
                .await;
        });

        Ok(Box::pin(AnthropicReceiverStreamAdapter { rx }))
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
        #[derive(Serialize)]
        struct Request<'a> {
            model: &'a str,
            messages: Vec<ApiMessage>,
            #[serde(skip_serializing_if = "Option::is_none")]
            tools: Option<Vec<ApiTool>>,
            max_tokens: u64,
            temperature: f32,
            #[serde(skip_serializing_if = "Option::is_none")]
            stop_sequences: Option<Vec<String>>,
        }

        // Prepare request data - convert messages with cache support for system messages
        let api_messages: Vec<ApiMessage> = messages
            .into_iter()
            .map(|m| {
                if m.role == LlmRole::System && !m.content.is_empty() {
                    // System messages use cache_control for prompt caching
                    ApiMessage::SystemWithCache {
                        role: "system",
                        content: vec![SystemContentBlock {
                            r#type: "text".to_string(),
                            text: m.content,
                            cache_control: Some(CacheControl {
                                r#type: "ephemeral".to_string(),
                            }),
                        }],
                    }
                } else {
                    ApiMessage::Simple {
                        role: Self::convert_role(m.role),
                        content: m.content,
                    }
                }
            })
            .collect();

        let api_tools = tools.map(|tools| {
            tools
                .into_iter()
                .map(|t| ApiTool {
                    name: t.name,
                    description: t.description,
                    input_schema: t.input_schema,
                })
                .collect()
        });

        let request = Request {
            model: &self.model,
            messages: api_messages,
            tools: api_tools,
            max_tokens: config.max_tokens,
            temperature: config.temperature,
            stop_sequences: config.stop_sequences,
        };

        let latency = LatencyTracker::new();

        debug!(model = %self.model, "Anthropic API request");

        // Retry loop for retryable errors (429, 5xx)
        let max_attempts = self.retry_config.max_retries + 1; // +1 for initial attempt
        let mut attempt = 0;

        let response = loop {
            attempt += 1;

            // Make HTTP request with prompt caching beta header
            let url = format!("{}/v1/messages", self.base_url);
            let resp = match self
                .client
                .post(&url)
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", "2023-06-01")
                .header("anthropic-beta", PROMPT_CACHING_BETA)
                .header("content-type", "application/json")
                .json(&request)
                .send()
                .await
            {
                Ok(resp) => resp,
                Err(e) => {
                    // Network error - retry if attempts remaining
                    if attempt < max_attempts {
                        let delay_secs = calculate_backoff(attempt, &self.retry_config);
                        warn!(
                            attempt = attempt,
                            error = %e,
                            delay_secs = delay_secs,
                            "Anthropic request failed, retrying"
                        );
                        sleep(Duration::from_secs_f64(delay_secs)).await;
                        continue;
                    }
                    return Err(SwellError::LlmError(format!("Request failed: {}", e)));
                }
            };

            let status = resp.status();

            // Check if we should retry
            if status.is_success() {
                // Success - break with the response
                break resp;
            } else if is_retryable_status(status) && attempt < max_attempts {
                // Retryable error (429, 5xx) - retry with backoff
                let body = resp.text().await.unwrap_or_default();
                let delay_secs = calculate_backoff(attempt, &self.retry_config);
                warn!(
                    attempt = attempt,
                    status = %status,
                    body = %body,
                    delay_secs = delay_secs,
                    "Anthropic API error, retrying"
                );
                sleep(Duration::from_secs_f64(delay_secs)).await;
                continue;
            } else if is_retryable_status(status) {
                // Retryable error but no attempts left
                let body = resp.text().await.unwrap_or_default();
                warn!(
                    status = %status,
                    body = %body,
                    "Anthropic API error: max retries exceeded"
                );
                return Err(SwellError::LlmError(format!(
                    "API error {} after {} attempts: {}",
                    status, attempt, body
                )));
            } else {
                // Non-retryable error (400, 401, 403) - fail immediately
                let body = resp.text().await.unwrap_or_default();
                warn!(
                    status = %status,
                    body = %body,
                    "Anthropic API non-retryable error"
                );
                return Err(SwellError::LlmError(format!(
                    "API error {}: {}",
                    status, body
                )));
            }
        };

        // Parse response
        let api_response: Response = match response.json().await {
            Ok(resp) => resp,
            Err(e) => {
                return Err(SwellError::LlmError(format!(
                    "Failed to parse response: {}",
                    e
                )));
            }
        };

        let latency_ms = latency.elapsed_ms();

        let mut content = String::new();
        let mut tool_calls = Vec::new();

        for item in api_response.content {
            match item {
                ResponseContent::Text { text } => {
                    content.push_str(&text);
                }
                ResponseContent::ToolUse { name, input, id } => {
                    tool_calls.push(LlmToolCall {
                        id,
                        name,
                        arguments: input,
                    });
                }
            }
        }

        let input_tokens = api_response.usage.input_tokens;
        let output_tokens = api_response.usage.output_tokens;

        // Get the tracer and record the span
        let tracer = self.tracer();
        let span_name = format!("Anthropic chat {}", self.model);
        let mut span_builder = tracer.span_builder(span_name);
        span_builder.attributes = Some(vec![
            KeyValue::new(gen_ai::OPERATION_NAME, "chat".to_string()),
            KeyValue::new(gen_ai::PROVIDER_NAME, "anthropic".to_string()),
            KeyValue::new(gen_ai::REQUEST_MODEL, self.model.clone()),
        ]);

        // Build and end the span with the recorded attributes
        let mut span = tracer.build(span_builder);
        span.record_prompt_tokens(input_tokens);
        span.record_completion_tokens(output_tokens);
        span.record_latency_ms(latency_ms);
        span.record_response_model(api_response.model.as_deref().unwrap_or(&self.model));

        // Calculate and record cost
        let pricing = pricing::for_model(&self.model);
        let cost = pricing.calculate_cost(input_tokens, output_tokens);
        span.record_cost_usd(cost);

        span.end();

        // Record cost to global tracker for dashboard integration
        record_llm_cost(input_tokens + output_tokens, &self.model);

        Ok(LlmResponse {
            content,
            tool_calls: if tool_calls.is_empty() {
                None
            } else {
                Some(tool_calls)
            },
            usage: LlmUsage {
                input_tokens,
                output_tokens,
                total_tokens: input_tokens + output_tokens,
                cache_creation_input_tokens: api_response.usage.cache_creation_input_tokens,
                cache_read_input_tokens: api_response.usage.cache_read_input_tokens,
            },
        })
    }

    async fn health_check(&self) -> bool {
        // Simple check - just verify client can connect
        let url = format!("{}/v1/messages", self.base_url);
        self.client.head(&url).send().await.is_ok()
    }

    async fn stream(
        &self,
        messages: Vec<LlmMessage>,
        tools: Option<Vec<LlmToolDefinition>>,
        config: LlmConfig,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, SwellError>> + Send>>, SwellError>
    {
        self.stream(messages, tools, config).await
    }
}

// ============================================================================
// SSE Parser for Anthropic Streaming Responses
// ============================================================================

/// Parser for Server-Sent Events (SSE) from Anthropic's Messages API.
///
/// Anthropic sends events in the format:
/// ```ignore
/// event: content_block_start
/// data: {"type":"text","index":0}
///
/// event: content_block_delta
/// data: {"type":"text_delta","text":"Hello","index":0}
///
/// event: message_delta
/// data: {"type":"message_delta","usage":{"output_tokens":10}}
///
/// event: message_stop
/// data: {"type":"message_stop"}
/// ```
struct AnthropicSseParser {
    /// Accumulated text content
    text_accumulator: String,
    /// Current tool use being accumulated
    current_tool_use: Option<ToolUseAccumulator>,
    /// Usage stats from message_delta
    usage: Option<AnthropicStreamUsage>,
    /// Index of current content block
    current_block_index: Option<u32>,
}

#[derive(Debug, Clone)]
struct ToolUseAccumulator {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone)]
struct AnthropicStreamUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_creation_input_tokens: Option<u64>,
    pub cache_read_input_tokens: Option<u64>,
    pub stop_reason: Option<String>,
}

impl AnthropicSseParser {
    fn new() -> Self {
        Self {
            text_accumulator: String::new(),
            current_tool_use: None,
            usage: None,
            current_block_index: None,
        }
    }

    /// Parse a data line and return any resulting StreamEvent.
    ///
    /// Returns `Ok(None)` if the line doesn't produce an event.
    /// Returns `Ok(Some(StreamEvent))` if an event should be emitted.
    fn parse_data_line(
        &mut self,
        data: &str,
        _event_type: Option<&str>,
    ) -> Result<Option<StreamEvent>, SwellError> {
        // Handle empty data
        if data.is_empty() {
            return Ok(None);
        }

        // Parse the JSON
        let event: AnthropicSseEvent = serde_json::from_str(data).map_err(|e| {
            SwellError::LlmError(format!("Failed to parse SSE event: {} - data: {}", e, data))
        })?;

        match event {
            // Text block start
            AnthropicSseEvent::TextBlockStart { index } => {
                self.current_block_index = index;
                // Text block - accumulate will happen via TextDelta
                Ok(None)
            }

            // Tool use block start
            AnthropicSseEvent::ToolUseBlockStart {
                name,
                input: _,
                id,
                index,
            } => {
                self.current_block_index = index;
                // Start accumulating tool use
                self.current_tool_use = Some(ToolUseAccumulator {
                    id: id.unwrap_or_default(),
                    name: name.unwrap_or_default(),
                    arguments: String::new(),
                });
                Ok(None)
            }

            // Text delta - accumulate and emit
            AnthropicSseEvent::TextDelta { text, index: _ } => {
                if let Some(t) = text {
                    self.text_accumulator.push_str(&t);
                    return Ok(Some(StreamEvent::TextDelta {
                        text: self.text_accumulator.clone(),
                        delta: t,
                    }));
                }
                Ok(None)
            }

            // Input JSON delta - accumulate tool arguments
            AnthropicSseEvent::InputJsonDelta {
                partial_json,
                index: _,
            } => {
                if let Some(partial) = partial_json {
                    if let Some(ref mut tool) = self.current_tool_use {
                        tool.arguments.push_str(&partial);
                    }
                }
                Ok(None)
            }

            // Message delta - usage and stop_reason
            AnthropicSseEvent::MessageDelta { delta, usage } => {
                // Extract stop_reason from delta if present
                let stop_reason = delta.as_ref().and_then(|d| d.stop_reason.clone());

                if let Some(usage) = usage {
                    self.usage = Some(AnthropicStreamUsage {
                        input_tokens: usage.input_tokens,
                        output_tokens: usage.output_tokens,
                        cache_creation_input_tokens: usage.cache_creation_input_tokens,
                        cache_read_input_tokens: usage.cache_read_input_tokens,
                        stop_reason,
                    });
                }

                // Emit usage event if we have output tokens
                if let Some(ref u) = self.usage {
                    if u.output_tokens.is_some() {
                        return Ok(Some(StreamEvent::Usage {
                            input_tokens: u.input_tokens.unwrap_or(0),
                            output_tokens: u.output_tokens.unwrap_or(0),
                            cache_creation_input_tokens: u.cache_creation_input_tokens,
                            cache_read_input_tokens: u.cache_read_input_tokens,
                        }));
                    }
                }
                Ok(None)
            }

            // Message stop - stream is complete
            AnthropicSseEvent::MessageStop => {
                // Before sending MessageStop, emit any pending tool use
                if let Some(tool) = self.current_tool_use.take() {
                    // Only emit if we have a name (valid tool use)
                    if !tool.name.is_empty() {
                        let arguments: serde_json::Value = serde_json::from_str(&tool.arguments)
                            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

                        return Ok(Some(StreamEvent::ToolUse {
                            tool_call: LlmToolCall {
                                id: tool.id,
                                name: tool.name,
                                arguments,
                            },
                        }));
                    }
                }

                let stop_reason = self.usage.as_ref().and_then(|u| u.stop_reason.clone());
                Ok(Some(StreamEvent::MessageStop { stop_reason }))
            }
        }
    }
}

impl Default for AnthropicSseParser {
    fn default() -> Self {
        Self::new()
    }
}

/// SSE event types from Anthropic Messages API streaming.
///
/// Note: These events come in two stages:
/// 1. "event: content_block_start" line sets the event type
/// 2. "data: {...}" line contains JSON with inner "type" field that further specifies the event
///
/// The inner "type" values for content_block_start can be "text" or "tool_use".
/// The inner "type" values for content_block_delta can be "text_delta" or "input_json_delta".
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum AnthropicSseEvent {
    #[serde(rename = "text")]
    TextBlockStart {
        #[serde(default)]
        index: Option<u32>,
    },
    #[serde(rename = "tool_use")]
    ToolUseBlockStart {
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        #[allow(dead_code)]
        input: Option<serde_json::Value>,
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        index: Option<u32>,
    },
    #[serde(rename = "text_delta")]
    TextDelta {
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        #[allow(dead_code)]
        index: Option<u32>,
    },
    #[serde(rename = "input_json_delta")]
    InputJsonDelta {
        #[serde(default)]
        partial_json: Option<String>,
        #[serde(default)]
        #[allow(dead_code)]
        index: Option<u32>,
    },
    #[serde(rename = "message_delta")]
    MessageDelta {
        #[serde(default)]
        delta: Option<MessageDeltaInner>,
        #[serde(default)]
        usage: Option<MessageDeltaUsage>,
    },
    #[serde(rename = "message_stop")]
    MessageStop,
}

/// Inner delta for message_delta events containing stop_reason.
#[derive(Debug, Deserialize)]
struct MessageDeltaInner {
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    stop_sequence: Option<serde_json::Value>,
}

/// Usage in a message_delta event.
#[derive(Debug, Deserialize)]
struct MessageDeltaUsage {
    #[serde(default)]
    input_tokens: Option<u64>,
    #[serde(default)]
    output_tokens: Option<u64>,
    #[serde(default)]
    cache_creation_input_tokens: Option<u64>,
    #[serde(default)]
    cache_read_input_tokens: Option<u64>,
}

// ============================================================================
// Stream Adapter for mpsc::Receiver
// ============================================================================

/// A stream adapter that wraps an `mpsc::Receiver` and implements `Stream`.
struct AnthropicReceiverStreamAdapter {
    rx: mpsc::Receiver<Result<StreamEvent, SwellError>>,
}

impl Stream for AnthropicReceiverStreamAdapter {
    type Item = Result<StreamEvent, SwellError>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LlmRole;

    #[test]
    fn test_backend_model_name() {
        let backend = AnthropicBackend::new("claude-opus-4-5", "fake-key");
        assert_eq!(backend.model(), "claude-opus-4-5");
    }

    #[test]
    fn test_role_conversion() {
        assert_eq!(AnthropicBackend::convert_role(LlmRole::System), "system");
        assert_eq!(AnthropicBackend::convert_role(LlmRole::User), "user");
        assert_eq!(
            AnthropicBackend::convert_role(LlmRole::Assistant),
            "assistant"
        );
    }

    #[test]
    fn test_prompt_caching_beta_header() {
        // Verify the beta header constant is correct
        assert_eq!(PROMPT_CACHING_BETA, "prompt-caching-2024-07-31");
    }

    #[test]
    fn test_system_message_cache_control_structure() {
        // Test that system messages with cache control serialize correctly
        let cache_control = CacheControl {
            r#type: "ephemeral".to_string(),
        };
        let block = SystemContentBlock {
            r#type: "text".to_string(),
            text: "You are a helpful assistant".to_string(),
            cache_control: Some(cache_control),
        };

        let json = serde_json::to_string(&block).unwrap();
        assert!(json.contains("\"cache_control\""));
        assert!(json.contains("\"ephemeral\""));
        assert!(json.contains("\"type\":\"text\""));
    }

    #[test]
    fn test_api_message_system_variant_with_cache() {
        // Test that system messages use the SystemWithCache variant
        let msg = ApiMessage::SystemWithCache {
            role: "system",
            content: vec![SystemContentBlock {
                r#type: "text".to_string(),
                text: "System prompt".to_string(),
                cache_control: Some(CacheControl {
                    r#type: "ephemeral".to_string(),
                }),
            }],
        };

        let json = serde_json::to_string(&msg).unwrap();
        // Should contain role and content with cache_control
        assert!(json.contains("\"role\":\"system\""));
        assert!(json.contains("\"cache_control\""));
        assert!(json.contains("\"ephemeral\""));
    }

    #[test]
    fn test_api_message_simple_variant() {
        // Test that non-system messages use the Simple variant
        let msg = ApiMessage::Simple {
            role: "user",
            content: "Hello".to_string(),
        };

        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"role\":\"user\""));
        assert!(json.contains("\"content\":\"Hello\""));
        // Simple variant should not have cache_control
        assert!(!json.contains("\"cache_control\""));
    }

    #[test]
    fn test_response_usage_with_cache_tokens() {
        // Test deserialization of ResponseUsage with cache fields
        let json = r#"{"input_tokens":100,"output_tokens":50,"cache_creation_input_tokens":200,"cache_read_input_tokens":300}"#;
        let usage: ResponseUsage = serde_json::from_str(json).unwrap();

        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 50);
        assert_eq!(usage.cache_creation_input_tokens, Some(200));
        assert_eq!(usage.cache_read_input_tokens, Some(300));
    }

    #[test]
    fn test_response_usage_without_cache_tokens() {
        // Test deserialization of ResponseUsage without cache fields (backward compat)
        let json = r#"{"input_tokens":100,"output_tokens":50}"#;
        let usage: ResponseUsage = serde_json::from_str(json).unwrap();

        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 50);
        assert_eq!(usage.cache_creation_input_tokens, None);
        assert_eq!(usage.cache_read_input_tokens, None);
    }

    #[test]
    fn test_llm_usage_four_dimensional_tracking() {
        // Test LlmUsage struct with all four token dimensions
        let usage = LlmUsage {
            input_tokens: 100,
            output_tokens: 50,
            total_tokens: 150,
            cache_creation_input_tokens: Some(200),
            cache_read_input_tokens: Some(300),
        };

        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 50);
        assert_eq!(usage.total_tokens, 150);
        assert_eq!(usage.cache_creation_input_tokens, Some(200));
        assert_eq!(usage.cache_read_input_tokens, Some(300));
    }

    #[test]
    fn test_llm_usage_without_cache_tokens() {
        // Test LlmUsage without cache tokens (backward compatible)
        let usage = LlmUsage {
            input_tokens: 100,
            output_tokens: 50,
            total_tokens: 150,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
        };

        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 50);
        assert_eq!(usage.cache_creation_input_tokens, None);
        assert_eq!(usage.cache_read_input_tokens, None);
    }

    #[test]
    fn test_response_usage_partial_cache_tokens() {
        // Test that partial cache token data works (only cache_read)
        let json = r#"{"input_tokens":100,"output_tokens":50,"cache_read_input_tokens":300}"#;
        let usage: ResponseUsage = serde_json::from_str(json).unwrap();

        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 50);
        assert_eq!(usage.cache_creation_input_tokens, None);
        assert_eq!(usage.cache_read_input_tokens, Some(300));
    }
}

// ============================================================================
// Retry Behavior Integration Tests
// ============================================================================

#[cfg(test)]
mod retry_tests {
    use super::*;
    use reqwest::StatusCode;

    #[test]
    fn test_anthropic_fails_immediately_on_bad_request() {
        // Verify 400 is not retryable
        let status_400 = StatusCode::from_u16(400).unwrap();
        assert!(!is_retryable_status(status_400));
    }

    #[test]
    fn test_anthropic_fails_immediately_on_unauthorized() {
        // Verify 401 is not retryable
        let status_401 = StatusCode::from_u16(401).unwrap();
        assert!(!is_retryable_status(status_401));
    }

    #[test]
    fn test_anthropic_fails_immediately_on_forbidden() {
        // Verify 403 is not retryable
        let status_403 = StatusCode::from_u16(403).unwrap();
        assert!(!is_retryable_status(status_403));
    }

    #[test]
    fn test_anthropic_retry_on_server_error() {
        // Verify 500 is retryable
        let status_500 = StatusCode::from_u16(500).unwrap();
        assert!(is_retryable_status(status_500));

        // Verify 502 is retryable
        let status_502 = StatusCode::from_u16(502).unwrap();
        assert!(is_retryable_status(status_502));

        // Verify 503 is retryable
        let status_503 = StatusCode::from_u16(503).unwrap();
        assert!(is_retryable_status(status_503));

        // Verify 504 is retryable
        let status_504 = StatusCode::from_u16(504).unwrap();
        assert!(is_retryable_status(status_504));
    }

    #[test]
    fn test_anthropic_retry_config_in_backend() {
        // Test that retry config is properly stored in backend
        let retry_config = LlmRetryConfig::new()
            .with_max_retries(5)
            .with_base_delay_secs(2.0)
            .with_max_delay_secs(30.0);

        let backend = AnthropicBackend::with_retry_config(
            "claude-sonnet-4-20250514",
            "test-key",
            retry_config.clone(),
        );

        assert_eq!(backend.retry_config.max_retries, 5);
        assert_eq!(backend.retry_config.base_delay_secs, 2.0);
        assert_eq!(backend.retry_config.max_delay_secs, 30.0);
    }

    #[test]
    fn test_anthropic_default_retry_config() {
        // Test that default retry config is applied when using new()
        let backend = AnthropicBackend::new("claude-sonnet-4-20250514", "test-key");

        // Default: max_retries = 3, base_delay = 1.0, max_delay = 60.0
        assert_eq!(backend.retry_config.max_retries, 3);
        assert_eq!(backend.retry_config.base_delay_secs, 1.0);
        assert_eq!(backend.retry_config.max_delay_secs, 60.0);
    }

    #[tokio::test]
    async fn test_anthropic_retry_config_with_mockito_server() {
        // This test validates the retry config is properly stored when a server URL is needed
        use mockito::Server;

        let server = Server::new_async().await;

        // Create backend with only 3 retries
        // Note: We don't make actual HTTP requests in this test, just verify config is stored
        let retry_config = LlmRetryConfig::new()
            .with_max_retries(3)
            .with_base_delay_secs(0.01);
        let backend = AnthropicBackend::with_retry_config(
            "claude-sonnet-4-20250514",
            "test-key",
            retry_config,
        );

        // Verify retry config
        assert_eq!(backend.retry_config.max_retries, 3);

        // Keep server alive for the test duration
        let _server_url = server.url();
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

// ============================================================================
// SSE Parser Tests
// ============================================================================

#[cfg(test)]
mod sse_parser_tests {
    use super::*;

    #[test]
    fn test_sse_parser_text_delta() {
        let mut parser = AnthropicSseParser::new();

        // Simulate receiving text chunks - use correct event types
        let line1 = r#"{"type":"text","index":0}"#;
        let result = parser
            .parse_data_line(line1, Some("content_block_start"))
            .unwrap();
        assert!(result.is_none()); // Just block start, no event yet

        // Text delta
        let line2 = r#"{"type":"text_delta","text":"Hello","index":0}"#;
        let result = parser
            .parse_data_line(line2, Some("content_block_delta"))
            .unwrap();
        assert!(result.is_some());
        let event = result.unwrap();
        match event {
            StreamEvent::TextDelta { text, delta } => {
                assert_eq!(text, "Hello");
                assert_eq!(delta, "Hello");
            }
            _ => panic!("Expected TextDelta event"),
        }

        // Second chunk
        let line3 = r#"{"type":"text_delta","text":" World","index":0}"#;
        let result = parser
            .parse_data_line(line3, Some("content_block_delta"))
            .unwrap();
        assert!(result.is_some());
        let event = result.unwrap();
        match event {
            StreamEvent::TextDelta { text, delta } => {
                assert_eq!(text, "Hello World");
                assert_eq!(delta, " World");
            }
            _ => panic!("Expected TextDelta event"),
        }
    }

    #[test]
    fn test_sse_parser_tool_use_accumulation() {
        let mut parser = AnthropicSseParser::new();

        // Tool use block start
        let line1 =
            r#"{"type":"tool_use","index":0,"name":"get_weather","id":"toolu_123","input":{}}"#;
        let result = parser
            .parse_data_line(line1, Some("content_block_start"))
            .unwrap();
        assert!(result.is_none()); // Just starting accumulation

        // First JSON delta
        let line2 = r#"{"type":"input_json_delta","partial_json":"{\"location\":","index":0}"#;
        let result = parser
            .parse_data_line(line2, Some("content_block_delta"))
            .unwrap();
        assert!(result.is_none()); // Still accumulating

        // Second JSON delta
        let line3 = r#"{"type":"input_json_delta","partial_json":" \"Los Angeles\"}","index":0}"#;
        let result = parser
            .parse_data_line(line3, Some("content_block_delta"))
            .unwrap();
        assert!(result.is_none()); // Still accumulating

        // Message stop - should emit the accumulated tool use
        let line4 = r#"{"type":"message_stop"}"#;
        let result = parser.parse_data_line(line4, Some("message_stop")).unwrap();
        assert!(result.is_some());
        let event = result.unwrap();
        match event {
            StreamEvent::ToolUse { tool_call } => {
                assert_eq!(tool_call.name, "get_weather");
                assert_eq!(tool_call.id, "toolu_123");
                // Arguments should be a JSON object
                let args = tool_call.arguments;
                assert!(args.is_object());
                let location = args.get("location");
                assert!(location.is_some());
                assert_eq!(location.unwrap().as_str(), Some("Los Angeles"));
            }
            _ => panic!("Expected ToolUse event"),
        }
    }

    #[test]
    fn test_sse_parser_message_stop() {
        let mut parser = AnthropicSseParser::new();

        // Text block start
        let line1 = r#"{"type":"text","index":0}"#;
        parser
            .parse_data_line(line1, Some("content_block_start"))
            .unwrap();

        // Text delta
        let line2 = r#"{"type":"text_delta","text":"Done","index":0}"#;
        parser
            .parse_data_line(line2, Some("content_block_delta"))
            .unwrap();

        // Message delta with stop_reason in delta object (real Anthropic API structure)
        let line3 = r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":5}}"#;
        let result = parser
            .parse_data_line(line3, Some("message_delta"))
            .unwrap();
        // Usage event may be emitted
        if let Some(StreamEvent::Usage { .. }) = result {
            // Expected
        }

        // Message stop
        let result = parser
            .parse_data_line(r#"{"type":"message_stop"}"#, Some("message_stop"))
            .unwrap();
        assert!(result.is_some());
        match result.unwrap() {
            StreamEvent::MessageStop { stop_reason } => {
                assert_eq!(stop_reason, Some("end_turn".to_string()));
            }
            _ => panic!("Expected MessageStop event"),
        }
    }

    #[test]
    fn test_sse_parser_usage_extraction() {
        let mut parser = AnthropicSseParser::new();

        // Text block start
        parser
            .parse_data_line(r#"{"type":"text","index":0}"#, Some("content_block_start"))
            .unwrap();

        // Text delta
        parser
            .parse_data_line(
                r#"{"type":"text_delta","text":"Hi","index":0}"#,
                Some("content_block_delta"),
            )
            .unwrap();

        // Message delta with all usage fields (stop_reason inside usage object)
        let result = parser.parse_data_line(
            r#"{"type":"message_delta","usage":{"input_tokens":100,"output_tokens":50,"cache_creation_input_tokens":200,"cache_read_input_tokens":300}}"#,
            Some("message_delta"),
        ).unwrap();

        match result {
            Some(StreamEvent::Usage {
                input_tokens,
                output_tokens,
                cache_creation_input_tokens,
                cache_read_input_tokens,
            }) => {
                assert_eq!(input_tokens, 100);
                assert_eq!(output_tokens, 50);
                assert_eq!(cache_creation_input_tokens, Some(200));
                assert_eq!(cache_read_input_tokens, Some(300));
            }
            _ => panic!("Expected Usage event"),
        }
    }

    #[test]
    fn test_sse_parser_empty_data_line() {
        let mut parser = AnthropicSseParser::new();

        // Empty data line should be handled gracefully
        let result = parser.parse_data_line("", None).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_sse_parser_invalid_json() {
        let mut parser = AnthropicSseParser::new();

        // Invalid JSON should return an error
        let result = parser.parse_data_line("not valid json {{{", None);
        assert!(result.is_err());
    }

    #[test]
    fn test_sse_parser_text_then_tool_use() {
        let mut parser = AnthropicSseParser::new();

        // Text block start
        parser
            .parse_data_line(r#"{"type":"text","index":0}"#, Some("content_block_start"))
            .unwrap();

        // Text delta - note: index is outside inner object in real API but we parse simplified
        let result = parser
            .parse_data_line(
                r#"{"type":"text_delta","text":"I think ","index":1}"#,
                Some("content_block_delta"),
            )
            .unwrap();
        assert!(matches!(result, Some(StreamEvent::TextDelta { .. })));

        // Tool use block starts (new block)
        parser
            .parse_data_line(
                r#"{"type":"tool_use","index":1,"name":"search","id":"toolu_456","input":{}}"#,
                Some("content_block_start"),
            )
            .unwrap();

        // JSON delta
        parser
            .parse_data_line(
                r#"{"type":"input_json_delta","partial_json":"{\"query\": \"rust\"}","index":1}"#,
                Some("content_block_delta"),
            )
            .unwrap();

        // Message stop - should emit pending tool use
        let result = parser
            .parse_data_line(r#"{"type":"message_stop"}"#, Some("message_stop"))
            .unwrap();
        match result {
            Some(StreamEvent::ToolUse { tool_call }) => {
                assert_eq!(tool_call.name, "search");
            }
            _ => panic!("Expected ToolUse event"),
        }
    }
}
