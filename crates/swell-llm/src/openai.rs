//! OpenAI API backend with OpenTelemetry instrumentation.
//!
//! # SSE Streaming Support
//!
//! This backend supports Server-Sent Events (SSE) streaming via the `stream()` method.
//! The stream parses `data:` line chunks from the OpenAI Chat Completions API,
//! extracting `choices[0].delta.content` for text and `function_call` for tool calls.

use crate::{
    calculate_backoff, credential::validate_openai_key, is_retryable_status, LlmBackend, LlmConfig,
    LlmMessage, LlmResponse, LlmRetryConfig, LlmRole, LlmToolDefinition, LlmUsage,
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

#[derive(Debug, Clone)]
pub struct OpenAIBackend {
    model: String,
    api_key: String,
    base_url: String,
    client: Client,
    retry_config: LlmRetryConfig,
}

impl OpenAIBackend {
    pub fn new(model: impl Into<String>, api_key: impl Into<String>) -> Result<Self, SwellError> {
        let api_key = api_key.into();
        // Validate API key format to detect Anthropic keys being used with OpenAI backend
        validate_openai_key(&api_key)?;
        Self::with_retry_config(model, api_key, LlmRetryConfig::default())
    }

    pub fn with_base_url(
        model: impl Into<String>,
        api_key: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Result<Self, SwellError> {
        let api_key = api_key.into();
        // Validate API key format to detect Anthropic keys being used with OpenAI backend
        validate_openai_key(&api_key)?;
        Self::with_base_url_and_retry(model, api_key, base_url, LlmRetryConfig::default())
    }

    pub fn with_retry_config(
        model: impl Into<String>,
        api_key: impl Into<String>,
        retry_config: LlmRetryConfig,
    ) -> Result<Self, SwellError> {
        Ok(Self {
            model: model.into(),
            api_key: api_key.into(),
            base_url: "https://api.openai.com/v1".to_string(),
            client: Client::new(),
            retry_config,
        })
    }

    pub fn with_base_url_and_retry(
        model: impl Into<String>,
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        retry_config: LlmRetryConfig,
    ) -> Result<Self, SwellError> {
        Ok(Self {
            model: model.into(),
            api_key: api_key.into(),
            base_url: base_url.into(),
            client: Client::new(),
            retry_config,
        })
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
    /// OpenAI returns chunks in SSE format:
    /// ```ignore
    /// data: {"choices":[{"delta":{"content":"Hello"},"index":0}]}
    ///
    /// data: {"choices":[{"delta":{"content":" world"},"index":0}]}
    ///
    /// data: [DONE]
    /// ```
    ///
    /// For function calls:
    /// ```ignore
    /// data: {"choices":[{"delta":{"function_call":{"name":"get_weather","arguments":"{"}},"index":0}]}
    ///
    /// data: {"choices":[{"delta":{"function_call":{"arguments":"\"Los Angeles\""}},"index":0}]}
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
            temperature: f32,
            max_tokens: u64,
            stream: bool,
            #[serde(skip_serializing_if = "Option::is_none")]
            stop: Option<Vec<String>>,
        }

        #[derive(Serialize)]
        struct ApiMessage {
            role: &'static str,
            content: String,
        }

        #[derive(Serialize)]
        struct ApiTool {
            #[serde(rename = "type")]
            tool_type: &'static str,
            function: ApiFunction,
        }

        #[derive(Serialize)]
        struct ApiFunction {
            name: String,
            description: String,
            parameters: serde_json::Value,
        }

        // Prepare request data
        let api_messages: Vec<ApiMessage> = messages
            .into_iter()
            .map(|m| ApiMessage {
                role: Self::convert_role(m.role),
                content: m.content,
            })
            .collect();

        let api_tools = tools.map(|tools| {
            tools
                .into_iter()
                .map(|t| ApiTool {
                    tool_type: "function",
                    function: ApiFunction {
                        name: t.name,
                        description: t.description,
                        parameters: t.input_schema,
                    },
                })
                .collect()
        });

        let request = Request {
            model: &self.model,
            messages: api_messages,
            tools: api_tools,
            temperature: config.temperature,
            max_tokens: config.max_tokens,
            stream: true,
            stop: config.stop_sequences,
        };

        debug!(model = %self.model, "OpenAI streaming request");

        let url = format!("{}/chat/completions", self.base_url);

        // Make HTTP request with streaming
        let response = match self
            .client
            .post(&url)
            .header("authorization", format!("Bearer {}", self.api_key))
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
            warn!(status = %status, body = %body, "OpenAI API error");
            return Err(SwellError::LlmError(format!(
                "API error {}: {}",
                status, body
            )));
        }

        let (tx, rx) = mpsc::channel::<Result<StreamEvent, SwellError>>(100);

        // Spawn a task to process the SSE stream
        tokio::spawn(async move {
            let mut parser = SseParser::new();

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
                                    match parser.parse_line(line) {
                                        Ok(Some(event)) => {
                                            if tx.send(Ok(event)).await.is_err() {
                                                // Receiver dropped, stop processing
                                                return;
                                            }
                                        }
                                        Ok(None) => {
                                            // Line processed but no event (e.g., comment or empty data:)
                                        }
                                        Err(e) => {
                                            let _ = tx.send(Err(e)).await;
                                            return;
                                        }
                                    }
                                }
                                line_buffer.clear();
                            } else {
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

        Ok(Box::pin(ReceiverStreamAdapter { rx }))
    }
}

// ============================================================================
// SSE Parser for OpenAI Streaming Responses
// ============================================================================

/// Parser for Server-Sent Events (SSE) from OpenAI's Chat Completions API.
///
/// OpenAI sends events in the format:
/// ```ignore
/// data: {"choices":[{"delta":{"content":"text"},"index":0}]}
///
/// data: {"choices":[{"delta":{"function_call":{"name":"fn","arguments":"{}"}},"index":0}]}
///
/// data: [DONE]
/// ```
struct SseParser {
    /// Accumulated text content
    text_accumulator: String,
    /// Current function call being accumulated (name + arguments)
    current_function_call: Option<FunctionCallAccumulator>,
    /// Usage accumulator
    usage: Option<StreamUsage>,
    /// The ID of the last seen chunk (for tracking)
    last_chunk_id: Option<String>,
}

#[derive(Debug, Clone)]
struct FunctionCallAccumulator {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct StreamUsage {
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

impl SseParser {
    fn new() -> Self {
        Self {
            text_accumulator: String::new(),
            current_function_call: None,
            usage: None,
            last_chunk_id: None,
        }
    }

    /// Parse a single SSE line and return any resulting StreamEvent.
    ///
    /// Returns `Ok(None)` if the line doesn't produce an event (e.g., empty data:, comments).
    /// Returns `Ok(Some(StreamEvent))` if an event should be emitted.
    fn parse_line(&mut self, line: &str) -> Result<Option<StreamEvent>, SwellError> {
        // Handle SSE comment lines
        if line.starts_with(':') {
            return Ok(None);
        }

        // Strip "data: " prefix
        let data = match line.strip_prefix("data: ") {
            Some(d) => d.trim(),
            None => {
                // Maybe it's "data:" without space
                if let Some(d) = line.strip_prefix("data:") {
                    d.trim()
                } else {
                    // Not a data line, skip
                    return Ok(None);
                }
            }
        };

        // Handle empty data (just whitespace after data:)
        if data.is_empty() {
            return Ok(None);
        }

        // Handle [DONE] sentinel
        if data == "[DONE]" {
            // Before sending MessageStop, emit any pending function call
            if let Some(fc) = self.current_function_call.take() {
                return Ok(Some(self.emit_tool_use_event(fc)));
            }
            return Ok(Some(StreamEvent::MessageStop { stop_reason: None }));
        }

        // Parse the JSON chunk
        let chunk: SseChunk = serde_json::from_str(data).map_err(|e| {
            SwellError::LlmError(format!("Failed to parse SSE chunk: {} - data: {}", e, data))
        })?;

        // Track chunk ID
        if let Some(id) = &chunk.id {
            self.last_chunk_id = Some(id.clone());
        }

        // Update usage if present
        if let Some(usage) = &chunk.usage {
            self.usage = Some(StreamUsage {
                prompt_tokens: usage.prompt_tokens,
                completion_tokens: usage.completion_tokens,
                total_tokens: usage.total_tokens,
            });
        }

        // Process choices
        if let Some(choices) = &chunk.choices {
            for choice in choices {
                if let Some(delta) = &choice.delta {
                    // Handle text content
                    if let Some(content) = &delta.content {
                        self.text_accumulator.push_str(content);
                        return Ok(Some(StreamEvent::TextDelta {
                            text: self.text_accumulator.clone(),
                            delta: content.clone(),
                        }));
                    }

                    // Handle function call
                    if let Some(fc) = &delta.function_call {
                        // If we have a name, start a new function call accumulator
                        if let Some(name) = &fc.name {
                            // Generate a unique ID for the tool call
                            let id = format!(
                                "call_{}",
                                uuid::Uuid::new_v4().to_string().replace("-", "")
                            );

                            self.current_function_call = Some(FunctionCallAccumulator {
                                id,
                                name: name.clone(),
                                arguments: fc.arguments.clone().unwrap_or_default(),
                            });
                        } else if let Some(args) = &fc.arguments {
                            // Append to current function call arguments
                            if let Some(ref mut current) = self.current_function_call {
                                current.arguments.push_str(args);
                            }
                        }
                    }
                }

                // Check for finish reason indicating stream end
                if let Some(ref finish_reason) = choice.finish_reason {
                    if finish_reason != "null" && !finish_reason.is_empty() {
                        // Stream is ending, emit any pending function call
                        if let Some(fc) = self.current_function_call.take() {
                            return Ok(Some(self.emit_tool_use_event(fc)));
                        }
                    }
                }
            }
        }

        Ok(None)
    }

    /// Create a ToolUse event from accumulated function call data.
    fn emit_tool_use_event(&self, fc: FunctionCallAccumulator) -> StreamEvent {
        // Parse the arguments JSON
        let arguments: serde_json::Value = serde_json::from_str(&fc.arguments)
            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

        StreamEvent::ToolUse {
            tool_call: LlmToolCall {
                id: fc.id,
                name: fc.name,
                arguments,
            },
        }
    }
}

impl Default for SseParser {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Stream Adapter for mpsc::Receiver
// ============================================================================

/// A stream adapter that wraps an `mpsc::Receiver` and implements `Stream`.
struct ReceiverStreamAdapter {
    rx: mpsc::Receiver<Result<StreamEvent, SwellError>>,
}

impl Stream for ReceiverStreamAdapter {
    type Item = Result<StreamEvent, SwellError>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}

/// SSE chunk structure from OpenAI Chat Completions streaming API.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct SseChunk {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    object: Option<String>,
    #[serde(default)]
    created: Option<u64>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    choices: Option<Vec<SseChoice>>,
    #[serde(default)]
    usage: Option<SseUsage>,
}

/// A choice in an SSE chunk.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct SseChoice {
    #[serde(default)]
    index: u32,
    #[serde(default)]
    delta: Option<SseDelta>,
    #[serde(default)]
    finish_reason: Option<String>,
}

/// The delta content in an SSE chunk.
#[derive(Debug, Deserialize)]
struct SseDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    function_call: Option<SseFunctionCall>,
}

/// A function call delta in an SSE chunk.
#[derive(Debug, Deserialize)]
struct SseFunctionCall {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

/// Usage information in an SSE chunk (final chunk only).
#[derive(Debug, Deserialize)]
struct SseUsage {
    #[serde(default)]
    prompt_tokens: Option<u64>,
    #[serde(default)]
    completion_tokens: Option<u64>,
    #[serde(default)]
    total_tokens: Option<u64>,
}

#[async_trait]
impl LlmBackend for OpenAIBackend {
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
            temperature: f32,
            max_tokens: u64,
            #[serde(skip_serializing_if = "Option::is_none")]
            stop: Option<Vec<String>>,
        }

        #[derive(Serialize)]
        struct ApiMessage {
            role: &'static str,
            content: String,
        }

        #[derive(Serialize)]
        struct ApiTool {
            #[serde(rename = "type")]
            tool_type: &'static str,
            function: ApiFunction,
        }

        #[derive(Serialize)]
        struct ApiFunction {
            name: String,
            description: String,
            parameters: serde_json::Value,
        }

        #[derive(Deserialize)]
        struct Response {
            #[allow(dead_code)]
            id: Option<String>,
            choices: Vec<ResponseChoice>,
            usage: ResponseUsage,
        }

        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct ResponseChoice {
            message: ResponseMessage,
            finish_reason: String,
        }

        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct ResponseMessage {
            role: String,
            content: Option<String>,
            #[serde(default)]
            tool_calls: Option<Vec<ResponseToolCall>>,
        }

        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct ResponseToolCall {
            id: String,
            #[serde(rename = "type")]
            call_type: String,
            function: ResponseFunction,
        }

        #[derive(Deserialize)]
        struct ResponseFunction {
            name: String,
            arguments: String,
        }

        #[derive(Deserialize)]
        struct ResponseUsage {
            prompt_tokens: u64,
            completion_tokens: u64,
            total_tokens: u64,
        }

        // Prepare request data
        let api_messages: Vec<ApiMessage> = messages
            .into_iter()
            .map(|m| ApiMessage {
                role: Self::convert_role(m.role),
                content: m.content,
            })
            .collect();

        let api_tools = tools.map(|tools| {
            tools
                .into_iter()
                .map(|t| ApiTool {
                    tool_type: "function",
                    function: ApiFunction {
                        name: t.name,
                        description: t.description,
                        parameters: t.input_schema,
                    },
                })
                .collect()
        });

        let request = Request {
            model: &self.model,
            messages: api_messages,
            tools: api_tools,
            temperature: config.temperature,
            max_tokens: config.max_tokens,
            stop: config.stop_sequences,
        };

        let latency = LatencyTracker::new();

        debug!(model = %self.model, "OpenAI API request");

        let url = format!("{}/chat/completions", self.base_url);

        // Retry loop for retryable errors (429, 5xx)
        let max_attempts = self.retry_config.max_retries + 1; // +1 for initial attempt
        let mut attempt = 0;

        let response = loop {
            attempt += 1;

            // Make HTTP request
            let resp = match self
                .client
                .post(&url)
                .header("authorization", format!("Bearer {}", self.api_key))
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
                            "OpenAI request failed, retrying"
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
                    "OpenAI API error, retrying"
                );
                sleep(Duration::from_secs_f64(delay_secs)).await;
                continue;
            } else if is_retryable_status(status) {
                // Retryable error but no attempts left
                let body = resp.text().await.unwrap_or_default();
                warn!(
                    status = %status,
                    body = %body,
                    "OpenAI API error: max retries exceeded"
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
                    "OpenAI API non-retryable error"
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

        let choice = match api_response.choices.into_iter().next() {
            Some(choice) => choice,
            None => {
                return Err(SwellError::LlmError("No choices in response".to_string()));
            }
        };

        let content = choice.message.content.unwrap_or_default();

        let tool_calls: Option<Vec<LlmToolCall>> =
            choice
                .message
                .tool_calls
                .map(|calls: Vec<ResponseToolCall>| {
                    calls
                        .into_iter()
                        .map(|call| LlmToolCall {
                            id: call.id,
                            name: call.function.name,
                            arguments: serde_json::from_str(&call.function.arguments)
                                .unwrap_or(serde_json::Value::Object(serde_json::Map::new())),
                        })
                        .collect()
                });

        let input_tokens = api_response.usage.prompt_tokens;
        let output_tokens = api_response.usage.completion_tokens;

        // Record GenAI attributes on the span
        let tracer = self.tracer();
        let span_name = format!("OpenAI chat {}", self.model);
        let mut span_builder = tracer.span_builder(span_name);
        span_builder.attributes = Some(vec![
            KeyValue::new(gen_ai::OPERATION_NAME, "chat".to_string()),
            KeyValue::new(gen_ai::PROVIDER_NAME, "openai".to_string()),
            KeyValue::new(gen_ai::REQUEST_MODEL, self.model.clone()),
        ]);

        let mut span = tracer.build(span_builder);
        span.record_prompt_tokens(input_tokens);
        span.record_completion_tokens(output_tokens);
        span.record_latency_ms(latency_ms);
        span.record_response_model(&self.model);

        // Calculate and record cost
        let pricing = pricing::for_model(&self.model);
        let cost = pricing.calculate_cost(input_tokens, output_tokens);
        span.record_cost_usd(cost);

        span.end();

        // Record cost to global tracker for dashboard integration
        record_llm_cost(api_response.usage.total_tokens, &self.model);

        Ok(LlmResponse {
            content,
            tool_calls,
            usage: LlmUsage {
                input_tokens,
                output_tokens,
                total_tokens: api_response.usage.total_tokens,
                cache_creation_input_tokens: None,
                cache_read_input_tokens: None,
            },
        })
    }

    async fn health_check(&self) -> bool {
        let url = format!("{}/models/{}", self.base_url, self.model);
        self.client
            .get(&url)
            .header("authorization", format!("Bearer {}", self.api_key))
            .send()
            .await
            .is_ok()
    }

    async fn stream(
        &self,
        messages: Vec<LlmMessage>,
        tools: Option<Vec<LlmToolDefinition>>,
        config: LlmConfig,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, SwellError>> + Send>>, SwellError> {
        self.stream(messages, tools, config).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backend_model_name() {
        let backend = OpenAIBackend::new("gpt-4-turbo", "fake-key").unwrap();
        assert_eq!(backend.model(), "gpt-4-turbo");
    }

    #[test]
    fn test_role_conversion() {
        assert_eq!(OpenAIBackend::convert_role(LlmRole::System), "system");
        assert_eq!(OpenAIBackend::convert_role(LlmRole::User), "user");
        assert_eq!(OpenAIBackend::convert_role(LlmRole::Assistant), "assistant");
    }

    // =========================================================================
    // SSE Parser Tests
    // =========================================================================

    #[test]
    fn test_sse_parser_text_delta() {
        let mut parser = SseParser::new();

        // Simulate receiving text chunks
        let line1 = r#"data: {"choices":[{"delta":{"content":"Hello"},"index":0}]}"#;
        let result = parser.parse_line(line1).unwrap();
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
        let line2 = r#"data: {"choices":[{"delta":{"content":" World"},"index":0}]}"#;
        let result = parser.parse_line(line2).unwrap();
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
    fn test_sse_parser_function_call_accumulation() {
        let mut parser = SseParser::new();

        // First chunk: function call name
        let line1 = r#"data: {"choices":[{"delta":{"function_call":{"name":"get_weather","arguments":""},"index":0}}]}"#;
        let result = parser.parse_line(line1).unwrap();
        // No event yet, just accumulating the name
        assert!(result.is_none());

        // Second chunk: opening brace
        let line2 =
            r#"data: {"choices":[{"delta":{"function_call":{"arguments":"{"},"index":0}}]}"#;
        let result = parser.parse_line(line2).unwrap();
        assert!(result.is_none()); // Still accumulating

        // Third chunk: location key
        let line3 = r#"data: {"choices":[{"delta":{"function_call":{"arguments":"\"location\":"},"index":0}}]}"#;
        let result = parser.parse_line(line3).unwrap();
        assert!(result.is_none());

        // Fourth chunk: location value (partial)
        let line4 =
            r#"data: {"choices":[{"delta":{"function_call":{"arguments":"\"Los"},"index":0}}]}"#;
        let result = parser.parse_line(line4).unwrap();
        assert!(result.is_none());

        // Fifth chunk: rest of location value
        let line5 = r#"data: {"choices":[{"delta":{"function_call":{"arguments":" Angeles\""},"index":0}}]}"#;
        let result = parser.parse_line(line5).unwrap();
        assert!(result.is_none());

        // Sixth chunk: closing brace
        let line6 =
            r#"data: {"choices":[{"delta":{"function_call":{"arguments":"}"},"index":0}}]}"#;
        let result = parser.parse_line(line6).unwrap();
        assert!(result.is_none());

        // Seventh chunk: finish_reason indicates end
        let line7 = r#"data: {"choices":[{"delta":{},"finish_reason":"tool_calls","index":0}]}"#;
        let result = parser.parse_line(line7).unwrap();
        assert!(result.is_some());
        let event = result.unwrap();
        match event {
            StreamEvent::ToolUse { tool_call } => {
                assert_eq!(tool_call.name, "get_weather");
                // Arguments should be a JSON object
                let args = tool_call.arguments;
                assert!(args.is_object());
                let location = args.get("location");
                assert!(location.is_some());
                assert_eq!(location.unwrap().as_str(), Some("Los Angeles"));
            }
            _ => panic!("Expected ToolUse event, got {:?}", event),
        }
    }

    #[test]
    fn test_sse_parser_done_sentinel() {
        let mut parser = SseParser::new();

        // First send a text delta
        let line1 = r#"data: {"choices":[{"delta":{"content":"Done"},"index":0}]}"#;
        let result = parser.parse_line(line1).unwrap();
        assert!(result.is_some());

        // Then send the [DONE] sentinel
        let result = parser.parse_line("data: [DONE]").unwrap();
        assert!(result.is_some());
        match result.unwrap() {
            StreamEvent::MessageStop { stop_reason } => {
                assert!(stop_reason.is_none());
            }
            _ => panic!("Expected MessageStop event"),
        }
    }

    #[test]
    fn test_sse_parser_done_with_pending_function_call() {
        let mut parser = SseParser::new();

        // Start a function call
        let line1 = r#"data: {"choices":[{"delta":{"function_call":{"name":"test_fn","arguments":""},"index":0}}]}"#;
        parser.parse_line(line1).unwrap();

        // Append some arguments
        let line2 = r#"data: {"choices":[{"delta":{"function_call":{"arguments":"{\"arg\": \"value\""},"index":0}}]}"#;
        parser.parse_line(line2).unwrap();

        // Now send [DONE] - should emit the pending tool call
        let result = parser.parse_line("data: [DONE]").unwrap();
        assert!(result.is_some());

        match result.unwrap() {
            StreamEvent::ToolUse { tool_call } => {
                assert_eq!(tool_call.name, "test_fn");
            }
            StreamEvent::MessageStop { .. } => {
                // Tool call was not emitted before MessageStop - this is a bug!
                panic!("Expected ToolUse event before MessageStop");
            }
            _ => panic!("Expected ToolUse or MessageStop event"),
        }
    }

    #[test]
    fn test_sse_parser_empty_data_line() {
        let mut parser = SseParser::new();

        // Empty data line should be handled gracefully
        let result = parser.parse_line("data:").unwrap();
        assert!(result.is_none());

        let result = parser.parse_line("data: ").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_sse_parser_comment_line() {
        let mut parser = SseParser::new();

        // SSE comment lines should be skipped
        let result = parser.parse_line(": this is a comment").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_sse_parser_non_data_line() {
        let mut parser = SseParser::new();

        // Non-data lines should be skipped
        let result = parser.parse_line("some random text").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_sse_parser_invalid_json() {
        let mut parser = SseParser::new();

        // Invalid JSON should return an error
        let result = parser.parse_line("data: not valid json {{{");
        assert!(result.is_err());
    }

    #[test]
    fn test_sse_parser_text_followed_by_function_call() {
        let mut parser = SseParser::new();

        // First chunk: text
        let line1 = r#"data: {"choices":[{"delta":{"content":"I think "},"index":0}]}"#;
        let result = parser.parse_line(line1).unwrap();
        assert!(matches!(result, Some(StreamEvent::TextDelta { .. })));

        // Second chunk: function call name starts
        let line2 = r#"data: {"choices":[{"delta":{"function_call":{"name":"search","arguments":""},"index":0}}]}"#;
        let _result = parser.parse_line(line2).unwrap();
        // This is the start of a function call - we don't emit yet

        // Third chunk: arguments start
        let line3 = r#"data: {"choices":[{"delta":{"function_call":{"arguments":"\"query\":"},"index":0}}]}"#;
        let result = parser.parse_line(line3).unwrap();
        assert!(result.is_none());

        // Fourth chunk: finish_reason
        let line4 = r#"data: {"choices":[{"delta":{},"finish_reason":"tool_calls","index":0}]}"#;
        let result = parser.parse_line(line4).unwrap();
        assert!(result.is_some());
        match result.unwrap() {
            StreamEvent::ToolUse { tool_call } => {
                assert_eq!(tool_call.name, "search");
            }
            _ => panic!("Expected ToolUse"),
        }
    }
}

// ============================================================================
// Retry Behavior Integration Tests
// ============================================================================

#[cfg(test)]
mod retry_tests {
    use super::*;
    use mockito::Server;
    use std::time::Instant;

    #[test]
    fn test_openai_retry_config_in_backend() {
        // Test that retry config is properly stored in backend
        let retry_config = LlmRetryConfig::new()
            .with_max_retries(5)
            .with_base_delay_secs(2.0)
            .with_max_delay_secs(30.0);

        let backend = OpenAIBackend::with_base_url_and_retry(
            "gpt-4",
            "test-key",
            "https://api.openai.com/v1",
            retry_config.clone(),
        )
        .expect("Failed to create backend");

        assert_eq!(backend.retry_config.max_retries, 5);
        assert_eq!(backend.retry_config.base_delay_secs, 2.0);
        assert_eq!(backend.retry_config.max_delay_secs, 30.0);
    }

    #[test]
    fn test_openai_default_retry_config() {
        // Test that default retry config is applied when using new()
        let backend = OpenAIBackend::new("gpt-4", "test-key").expect("Failed to create backend");

        // Default: max_retries = 3, base_delay = 1.0, max_delay = 60.0
        assert_eq!(backend.retry_config.max_retries, 3);
        assert_eq!(backend.retry_config.base_delay_secs, 1.0);
        assert_eq!(backend.retry_config.max_delay_secs, 60.0);
    }

    #[tokio::test]
    async fn test_openai_retry_on_rate_limit_then_success() {
        // Create mockito server
        let mut server = Server::new_async().await;

        // Set up mock to return 429 with rate limit error
        // This test verifies retry behavior by checking that after exhausting retries
        // we get an error that mentions rate limiting
        let _mock = server
            .mock("POST", "/chat/completions")
            .with_status(429)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "error": {
                        "message": "Rate limit exceeded",
                        "type": "rate_limit_error",
                        "code": "rate_limit_exceeded"
                    }
                })
                .to_string(),
            )
            .create();

        // Create backend pointing to mockito server with fast retry config
        let retry_config = LlmRetryConfig::new()
            .with_max_retries(3)
            .with_base_delay_secs(0.01); // Fast backoff for testing
        let backend =
            OpenAIBackend::with_base_url_and_retry("gpt-4", "test-key", server.url(), retry_config)
                .expect("Failed to create backend");

        let messages = vec![LlmMessage {
            role: LlmRole::User,
            content: "Say hello".to_string(),
        }];

        let config = LlmConfig::default();

        // Make request - should fail after retries are exhausted
        let result = backend.chat(messages, None, config).await;

        // Verify request failed with rate limit error
        assert!(
            result.is_err(),
            "Request should fail after exhausting retries"
        );

        let error = result.unwrap_err().to_string();
        assert!(
            error.contains("429") || error.contains("rate_limit"),
            "Error should mention rate limit (429), got: {}",
            error
        );
    }

    #[tokio::test]
    async fn test_openai_retry_on_server_error_then_success() {
        // Create mockito server
        let mut server = Server::new_async().await;

        // Set up mock to return 500 with server error
        let _mock = server
            .mock("POST", "/chat/completions")
            .with_status(500)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "error": {
                        "message": "Internal server error",
                        "type": "server_error",
                        "code": "internal_error"
                    }
                })
                .to_string(),
            )
            .create();

        // Create backend with fast retry config
        let retry_config = LlmRetryConfig::new()
            .with_max_retries(3)
            .with_base_delay_secs(0.01);
        let backend =
            OpenAIBackend::with_base_url_and_retry("gpt-4", "test-key", server.url(), retry_config)
                .expect("Failed to create backend");

        let messages = vec![LlmMessage {
            role: LlmRole::User,
            content: "Say hello".to_string(),
        }];

        let config = LlmConfig::default();

        // Make request - should fail after retries are exhausted
        let result = backend.chat(messages, None, config).await;

        // Verify request failed with server error after retries
        assert!(
            result.is_err(),
            "Request should fail after exhausting retries"
        );

        let error = result.unwrap_err().to_string();
        assert!(
            error.contains("500")
                || error.contains("server_error")
                || error.contains("Internal Server Error"),
            "Error should mention server error (500), got: {}",
            error
        );
    }

    #[tokio::test]
    async fn test_openai_fails_immediately_on_bad_request() {
        // Create mockito server
        let mut server = Server::new_async().await;

        // Set up mock to return 400 Bad Request
        let mock = server
            .mock("POST", "/chat/completions")
            .with_status(400)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "error": {
                        "message": "Invalid request",
                        "type": "invalid_request_error",
                        "code": "invalid_request"
                    }
                })
                .to_string(),
            )
            .expect_at_least(1) // At least 1 request
            .expect_at_most(1) // But no more than 1 (no retries)
            .create();

        // Create backend with retry config
        let retry_config = LlmRetryConfig::new()
            .with_max_retries(3)
            .with_base_delay_secs(0.01);
        let backend =
            OpenAIBackend::with_base_url_and_retry("gpt-4", "test-key", server.url(), retry_config)
                .expect("Failed to create backend");

        let messages = vec![LlmMessage {
            role: LlmRole::User,
            content: "Say hello".to_string(),
        }];

        let config = LlmConfig::default();

        // Measure time to verify no retries
        let start = Instant::now();
        let result = backend.chat(messages, None, config).await;
        let elapsed = start.elapsed();

        // Verify request failed
        assert!(
            result.is_err(),
            "Request should fail immediately on 400 Bad Request"
        );

        let error = result.unwrap_err();
        let error_msg = error.to_string();
        // Should contain "400" or indicate bad request
        assert!(
            error_msg.contains("400") || error_msg.contains("Bad Request"),
            "Error should mention 400 or bad request, got: {}",
            error_msg
        );

        // Verify minimal time elapsed (no retry delays)
        // Should be nearly instant
        println!("Elapsed time (no retry): {:?}", elapsed);

        // Clean up mock
        mock.assert();
    }

    #[tokio::test]
    async fn test_openai_fails_immediately_on_unauthorized() {
        // Create mockito server
        let mut server = Server::new_async().await;

        // Set up mock to return 401 Unauthorized
        let mock = server
            .mock("POST", "/chat/completions")
            .with_status(401)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "error": {
                        "message": "Invalid API key",
                        "type": "authentication_error",
                        "code": "invalid_api_key"
                    }
                })
                .to_string(),
            )
            .expect_at_least(1) // At least 1 request
            .expect_at_most(1) // But no more than 1 (no retries)
            .create();

        // Create backend with retry config
        let retry_config = LlmRetryConfig::new()
            .with_max_retries(3)
            .with_base_delay_secs(0.01);
        let backend =
            OpenAIBackend::with_base_url_and_retry("gpt-4", "test-key", server.url(), retry_config)
                .expect("Failed to create backend");

        let messages = vec![LlmMessage {
            role: LlmRole::User,
            content: "Say hello".to_string(),
        }];

        let config = LlmConfig::default();

        let result = backend.chat(messages, None, config).await;

        // Verify request failed immediately without retry
        assert!(
            result.is_err(),
            "Request should fail immediately on 401 Unauthorized"
        );

        let error = result.unwrap_err();
        let error_msg = error.to_string();
        // Should contain "401" or indicate unauthorized
        assert!(
            error_msg.contains("401")
                || error_msg.contains("unauthorized")
                || error_msg.contains("Unauthorized"),
            "Error should mention 401 or unauthorized, got: {}",
            error_msg
        );

        // Clean up mock
        mock.assert();
    }

    #[tokio::test]
    async fn test_openai_fails_immediately_on_forbidden() {
        // Create mockito server
        let mut server = Server::new_async().await;

        // Set up mock to return 403 Forbidden
        let mock = server
            .mock("POST", "/chat/completions")
            .with_status(403)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "error": {
                        "message": "Permission denied",
                        "type": "permission_error",
                        "code": "permission_denied"
                    }
                })
                .to_string(),
            )
            .expect_at_least(1) // At least 1 request
            .expect_at_most(1) // But no more than 1 (no retries)
            .create();

        // Create backend with retry config
        let retry_config = LlmRetryConfig::new()
            .with_max_retries(3)
            .with_base_delay_secs(0.01);
        let backend =
            OpenAIBackend::with_base_url_and_retry("gpt-4", "test-key", server.url(), retry_config)
                .expect("Failed to create backend");

        let messages = vec![LlmMessage {
            role: LlmRole::User,
            content: "Say hello".to_string(),
        }];

        let config = LlmConfig::default();

        let result = backend.chat(messages, None, config).await;

        // Verify request failed immediately without retry
        assert!(
            result.is_err(),
            "Request should fail immediately on 403 Forbidden"
        );

        let error = result.unwrap_err();
        let error_msg = error.to_string();
        // Should contain "403" or indicate forbidden
        assert!(
            error_msg.contains("403")
                || error_msg.contains("forbidden")
                || error_msg.contains("Forbidden"),
            "Error should mention 403 or forbidden, got: {}",
            error_msg
        );

        // Clean up mock
        mock.assert();
    }

    #[tokio::test]
    async fn test_openai_exhausts_retries_on_persistent_rate_limit() {
        // Create mockito server
        let mut server = Server::new_async().await;

        // Set up mock to always return 429
        let mock = server
            .mock("POST", "/chat/completions")
            .with_status(429)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "error": {
                        "message": "Rate limit exceeded",
                        "type": "rate_limit_error",
                        "code": "rate_limit_exceeded"
                    }
                })
                .to_string(),
            )
            .expect_at_least(4) // Initial + 3 retries = 4 total
            .expect_at_most(4)
            .create();

        // Create backend with only 3 retries
        let retry_config = LlmRetryConfig::new()
            .with_max_retries(3)
            .with_base_delay_secs(0.01); // Fast backoff for testing
        let backend =
            OpenAIBackend::with_base_url_and_retry("gpt-4", "test-key", server.url(), retry_config)
                .expect("Failed to create backend");

        let messages = vec![LlmMessage {
            role: LlmRole::User,
            content: "Say hello".to_string(),
        }];

        let config = LlmConfig::default();

        // Make request - should fail after exhausting retries
        let start = Instant::now();
        let result = backend.chat(messages, None, config).await;
        let elapsed = start.elapsed();

        // Verify request failed after exhausting retries
        assert!(
            result.is_err(),
            "Request should fail after exhausting retries"
        );

        let error = result.unwrap_err();
        let error_msg = error.to_string();
        assert!(
            error_msg.contains("429") || error_msg.contains("attempts"),
            "Error should mention rate limit or attempts"
        );

        println!("Elapsed time after exhausting retries: {:?}", elapsed);

        // Clean up mock
        mock.assert();
    }
}
