//! Mock LLM backend for testing.

use crate::{LlmBackend, LlmConfig, LlmMessage, LlmResponse, LlmToolDefinition};
use async_trait::async_trait;
use futures::Stream;
use opentelemetry::trace::{Span, Tracer};
use opentelemetry::{global, KeyValue};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use swell_core::{
    opentelemetry::gen_ai, opentelemetry::pricing, opentelemetry::GenAiSpanExt,
    opentelemetry::LatencyTracker, LlmToolCall, StreamEvent, SwellError,
};
use tokio::sync::mpsc;

static CALL_COUNT: AtomicU64 = AtomicU64::new(0);

/// Configuration for a mock tool call that MockLlm should emit during streaming.
#[derive(Debug, Clone)]
pub struct MockToolCall {
    /// The tool call ID
    pub id: String,
    /// The tool name
    pub name: String,
    /// The tool arguments as JSON
    pub arguments: serde_json::Value,
    /// The result to return when this tool is called
    pub result: String,
    /// Whether the tool execution should be marked as successful
    pub success: bool,
}

impl MockToolCall {
    /// Create a new mock tool call.
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        arguments: serde_json::Value,
        result: impl Into<String>,
        success: bool,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            arguments,
            result: result.into(),
            success,
        }
    }

    /// Convert to an LlmToolCall for stream emission.
    pub fn to_llm_tool_call(&self) -> LlmToolCall {
        LlmToolCall {
            id: self.id.clone(),
            name: self.name.clone(),
            arguments: self.arguments.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct MockLlm {
    model: String,
    response: String,
    should_fail: bool,
    #[allow(dead_code)]
    call_count: u64,
    /// If true, echoes back user message content in the response
    echo_user: bool,
    /// Optional tool call to emit during streaming
    tool_call: Option<MockToolCall>,
}

impl MockLlm {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            response: "Mock response".to_string(),
            should_fail: false,
            call_count: 0,
            echo_user: true,
            tool_call: None,
        }
    }

    pub fn with_response(model: impl Into<String>, response: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            response: response.into(),
            should_fail: false,
            call_count: 0,
            echo_user: false,
            tool_call: None,
        }
    }

    pub fn failing(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            response: String::new(),
            should_fail: true,
            call_count: 0,
            echo_user: false,
            tool_call: None,
        }
    }

    /// Configure this mock to emit a tool call during streaming.
    ///
    /// When configured, the stream() method will emit:
    /// 1. ToolUse event with the configured tool call
    /// 2. ToolResult event with the configured result
    /// 3. Usage and MessageStop events
    ///
    /// # Example
    /// ```ignore
    /// let mock = MockLlm::with_response("claude", "Tool executed")
    ///     .with_tool_call(MockToolCall::new(
    ///         "call_123",
    ///         "file_read",
    ///         serde_json::json!({"path": "/tmp/test.txt"}),
    ///         "file contents",
    ///         true,
    ///     ));
    /// ```
    pub fn with_tool_call(mut self, tool_call: MockToolCall) -> Self {
        self.tool_call = Some(tool_call);
        self
    }

    /// Configure this mock to emit a tool call using the builder pattern.
    ///
    /// Convenience method that creates a MockToolCall from the provided arguments.
    ///
    /// # Example
    /// ```ignore
    /// let mock = MockLlm::with_response("claude", "Done")
    ///     .with_tool_use("call_123", "file_read",
    ///         serde_json::json!({"path": "/tmp/test.txt"}),
    ///         "file contents", true);
    /// ```
    pub fn with_tool_use(
        self,
        id: impl Into<String>,
        name: impl Into<String>,
        arguments: serde_json::Value,
        result: impl Into<String>,
        success: bool,
    ) -> Self {
        self.with_tool_call(MockToolCall::new(id, name, arguments, result, success))
    }

    /// Get the tracer for OpenTelemetry
    fn tracer(&self) -> impl Tracer {
        global::tracer("swell-llm-mock")
    }
}

#[async_trait]
impl LlmBackend for MockLlm {
    fn model(&self) -> &str {
        &self.model
    }

    async fn chat(
        &self,
        messages: Vec<LlmMessage>,
        _tools: Option<Vec<LlmToolDefinition>>,
        _config: LlmConfig,
    ) -> Result<LlmResponse, SwellError> {
        CALL_COUNT.fetch_add(1, Ordering::SeqCst);

        // Start OpenTelemetry span for the mock LLM call
        let tracer = self.tracer();
        let span_name = format!("Mock chat {}", self.model);
        let mut span_builder = tracer.span_builder(span_name);
        span_builder.attributes = Some(vec![
            KeyValue::new(gen_ai::OPERATION_NAME, "chat".to_string()),
            KeyValue::new(gen_ai::PROVIDER_NAME, "mock".to_string()),
            KeyValue::new(gen_ai::REQUEST_MODEL, self.model.clone()),
            KeyValue::new("mock", true),
        ]);
        let mut span = tracer.build(span_builder);

        let latency = LatencyTracker::new();

        if self.should_fail {
            span.set_status(opentelemetry::trace::Status::error("Mock failure"));
            span.end();
            return Err(SwellError::LlmError("Mock failure".to_string()));
        }

        // Return the configured response, optionally echoing user message content
        let content = if self.echo_user {
            let user_content: String = messages
                .iter()
                .filter(|m| m.role == crate::LlmRole::User)
                .map(|m| m.content.clone())
                .collect::<Vec<_>>()
                .join(" ");

            if user_content.is_empty() {
                self.response.clone()
            } else {
                format!("{}: {}", self.response, user_content)
            }
        } else {
            self.response.clone()
        };

        let input_tokens: u64 = messages.iter().map(|m| m.content.len() as u64 / 4).sum();
        let output_tokens = 50u64;

        // Record GenAI attributes on the span
        span.record_prompt_tokens(input_tokens);
        span.record_completion_tokens(output_tokens);
        span.record_latency_ms(latency.elapsed_ms());
        span.record_response_model(&self.model);

        // Calculate and record cost
        let pricing = pricing::for_model(&self.model);
        let cost = pricing.calculate_cost(input_tokens, output_tokens);
        span.record_cost_usd(cost);

        span.end();

        Ok(LlmResponse {
            content,
            tool_calls: None,
            usage: crate::LlmUsage {
                input_tokens,
                output_tokens,
                total_tokens: input_tokens + output_tokens,
                cache_creation_input_tokens: None,
                cache_read_input_tokens: None,
            },
        })
    }

    async fn health_check(&self) -> bool {
        !self.should_fail
    }

    async fn stream(
        &self,
        messages: Vec<LlmMessage>,
        _tools: Option<Vec<LlmToolDefinition>>,
        _config: LlmConfig,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, SwellError>> + Send>>, SwellError>
    {
        CALL_COUNT.fetch_add(1, Ordering::SeqCst);

        if self.should_fail {
            return Err(SwellError::LlmError("Mock failure".to_string()));
        }

        // Return the configured response, optionally echoing user message content
        let content = if self.echo_user {
            let user_content: String = messages
                .iter()
                .filter(|m| m.role == crate::LlmRole::User)
                .map(|m| m.content.clone())
                .collect::<Vec<_>>()
                .join(" ");

            if user_content.is_empty() {
                self.response.clone()
            } else {
                format!("{}: {}", self.response, user_content)
            }
        } else {
            self.response.clone()
        };

        let input_tokens: u64 = messages.iter().map(|m| m.content.len() as u64 / 4).sum();
        let output_tokens = 50u64;

        // Clone tool_call for use in the spawned task
        let tool_call = self.tool_call.clone();

        let (tx, rx) = mpsc::channel::<Result<StreamEvent, SwellError>>(100);

        // Spawn a task to emit the stream events
        tokio::spawn(async move {
            // If configured with a tool call, emit ToolUse and ToolResult events
            if let Some(tc) = tool_call {
                // Emit ToolUse event
                let _ = tx
                    .send(Ok(StreamEvent::ToolUse {
                        tool_call: tc.to_llm_tool_call(),
                    }))
                    .await;

                // Emit ToolResult event
                let _ = tx
                    .send(Ok(StreamEvent::ToolResult {
                        tool_call_id: tc.id.clone(),
                        result: tc.result.clone(),
                        success: tc.success,
                    }))
                    .await;
            }

            // Emit text delta with the full content as both text and delta
            // Since this is a mock, we emit the entire response as one delta
            let _ = tx
                .send(Ok(StreamEvent::TextDelta {
                    text: content.clone(),
                    delta: content,
                }))
                .await;

            // Emit usage
            let _ = tx
                .send(Ok(StreamEvent::Usage {
                    input_tokens,
                    output_tokens,
                    cache_creation_input_tokens: None,
                    cache_read_input_tokens: None,
                }))
                .await;

            // Emit message stop
            let _ = tx
                .send(Ok(StreamEvent::MessageStop { stop_reason: None }))
                .await;
        });

        Ok(Box::pin(MockStreamAdapter { rx }))
    }
}

pub fn get_call_count() -> u64 {
    CALL_COUNT.load(Ordering::SeqCst)
}

pub fn reset_call_count() {
    CALL_COUNT.store(0, Ordering::SeqCst);
}

/// Stream adapter for MockLlm streaming responses
struct MockStreamAdapter {
    rx: mpsc::Receiver<Result<StreamEvent, SwellError>>,
}

impl Stream for MockStreamAdapter {
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
    use futures::StreamExt;

    #[tokio::test]
    async fn test_mock_response() {
        let mock = MockLlm::new("gpt-4");
        let messages = vec![LlmMessage {
            role: LlmRole::User,
            content: "Hello".to_string(),
            ..Default::default()
        }];

        let response = mock
            .chat(
                messages,
                None,
                LlmConfig {
                    temperature: 0.7,
                    max_tokens: 4096,
                    stop_sequences: None,
                },
            )
            .await
            .unwrap();

        assert!(response.content.contains("Mock response"));
        assert!(response.content.contains("Hello"));
        // Call count is global across tests, so we just verify it's been called
        assert!(get_call_count() >= 1);
    }

    #[tokio::test]
    async fn test_mock_custom_response() {
        let mock = MockLlm::with_response("gpt-4", "Custom reply");
        let messages = vec![LlmMessage {
            role: LlmRole::User,
            content: "Test".to_string(),
            ..Default::default()
        }];

        let response = mock
            .chat(
                messages,
                None,
                LlmConfig {
                    temperature: 0.7,
                    max_tokens: 4096,
                    stop_sequences: None,
                },
            )
            .await
            .unwrap();

        // with_response returns exact response without echoing user message
        assert!(response.content.contains("Custom reply"));
        // Verify the custom response is returned verbatim
        assert_eq!(response.content, "Custom reply");
    }

    #[tokio::test]
    async fn test_mock_failure() {
        let mock = MockLlm::failing("gpt-4");
        let messages = vec![LlmMessage {
            role: LlmRole::User,
            content: "Hello".to_string(),
            ..Default::default()
        }];

        let result = mock
            .chat(
                messages,
                None,
                LlmConfig {
                    temperature: 0.7,
                    max_tokens: 4096,
                    stop_sequences: None,
                },
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_health_check() {
        let healthy = MockLlm::new("gpt-4");
        assert!(healthy.health_check().await);

        let unhealthy = MockLlm::failing("gpt-4");
        assert!(!unhealthy.health_check().await);
    }

    #[tokio::test]
    async fn test_stream_with_tool_call() {
        // Create a mock LLM that returns a tool call
        let mock = MockLlm::with_response("claude", "Tool executed").with_tool_use(
            "call_123",
            "file_read",
            serde_json::json!({"path": "/tmp/test.txt"}),
            "file contents here",
            true,
        );

        let messages = vec![LlmMessage {
            role: LlmRole::User,
            content: "Read the file".to_string(),
            ..Default::default()
        }];

        let stream = mock
            .stream(
                messages,
                None,
                LlmConfig {
                    temperature: 0.7,
                    max_tokens: 4096,
                    stop_sequences: None,
                },
            )
            .await
            .unwrap();

        // Collect all events from the stream
        let events: Vec<_> = stream.collect().await;

        // Should have: ToolUse, ToolResult, TextDelta, Usage, MessageStop
        assert!(
            events.len() >= 5,
            "Expected at least 5 events, got {}",
            events.len()
        );

        // First event should be ToolUse
        let tool_use = match &events[0] {
            Ok(StreamEvent::ToolUse { tool_call }) => tool_call,
            other => panic!("Expected ToolUse as first event, got {:?}", other),
        };
        assert_eq!(tool_use.id, "call_123");
        assert_eq!(tool_use.name, "file_read");
        assert_eq!(tool_use.arguments["path"], "/tmp/test.txt");

        // Second event should be ToolResult
        let _tool_result = match &events[1] {
            Ok(StreamEvent::ToolResult {
                tool_call_id,
                result,
                success,
            }) => {
                assert_eq!(tool_call_id, "call_123");
                assert_eq!(result, "file contents here");
                assert!(success);
            }
            other => panic!("Expected ToolResult as second event, got {:?}", other),
        };

        // Find the TextDelta event
        let text_delta = events
            .iter()
            .find_map(|e| match e {
                Ok(StreamEvent::TextDelta { text, .. }) => Some(text.clone()),
                _ => None,
            })
            .expect("Should have a TextDelta event");
        assert!(text_delta.contains("Tool executed"));
    }

    #[tokio::test]
    async fn test_stream_without_tool_call() {
        // Create a mock LLM without tool call (text-only)
        let mock = MockLlm::with_response("claude", "Hello, world!");

        let messages = vec![LlmMessage {
            role: LlmRole::User,
            content: "Say hello".to_string(),
            ..Default::default()
        }];

        let stream = mock
            .stream(
                messages,
                None,
                LlmConfig {
                    temperature: 0.7,
                    max_tokens: 4096,
                    stop_sequences: None,
                },
            )
            .await
            .unwrap();

        // Collect all events
        let events: Vec<_> = stream.collect().await;

        // Should have: TextDelta, Usage, MessageStop (no ToolUse or ToolResult)
        assert!(
            events.len() >= 3,
            "Expected at least 3 events, got {}",
            events.len()
        );

        // Verify no ToolUse or ToolResult events
        for event in &events {
            match event {
                Ok(StreamEvent::ToolUse { .. }) | Ok(StreamEvent::ToolResult { .. }) => {
                    panic!("Should not have tool events when tool_call is not configured");
                }
                _ => {}
            }
        }

        // Find the TextDelta event
        let text_delta = events
            .iter()
            .find_map(|e| match e {
                Ok(StreamEvent::TextDelta { text, .. }) => Some(text.clone()),
                _ => None,
            })
            .expect("Should have a TextDelta event");
        assert!(text_delta.contains("Hello, world!"));
    }

    #[tokio::test]
    async fn test_stream_with_failed_tool() {
        // Create a mock LLM that returns a failed tool call
        let mock = MockLlm::with_response("claude", "Tool failed").with_tool_use(
            "call_456",
            "shell",
            serde_json::json!({"command": "exit 1"}),
            "Command failed: exit code 1",
            false, // Tool execution failed
        );

        let messages = vec![LlmMessage {
            role: LlmRole::User,
            content: "Run command".to_string(),
            ..Default::default()
        }];

        let stream = mock
            .stream(
                messages,
                None,
                LlmConfig {
                    temperature: 0.7,
                    max_tokens: 4096,
                    stop_sequences: None,
                },
            )
            .await
            .unwrap();

        let events: Vec<_> = stream.collect().await;

        // Should have ToolUse, ToolResult, TextDelta, Usage, MessageStop
        assert!(events.len() >= 5);

        // Check ToolResult has success=false
        let _tool_result = match &events[1] {
            Ok(StreamEvent::ToolResult {
                tool_call_id,
                result,
                success,
            }) => {
                assert_eq!(tool_call_id, "call_456");
                assert!(!success);
                assert!(result.contains("failed"));
            }
            other => panic!("Expected ToolResult as second event, got {:?}", other),
        };
    }

    #[tokio::test]
    async fn test_mock_tool_call_builder() {
        // Test the MockToolCall builder
        let tool_call = MockToolCall::new(
            "test_id",
            "test_tool",
            serde_json::json!({"arg1": "value1"}),
            "result",
            true,
        );

        assert_eq!(tool_call.id, "test_id");
        assert_eq!(tool_call.name, "test_tool");
        assert_eq!(tool_call.arguments["arg1"], "value1");
        assert_eq!(tool_call.result, "result");
        assert!(tool_call.success);

        // Test conversion to LlmToolCall
        let llm_tool_call = tool_call.to_llm_tool_call();
        assert_eq!(llm_tool_call.id, "test_id");
        assert_eq!(llm_tool_call.name, "test_tool");
        assert_eq!(llm_tool_call.arguments["arg1"], "value1");
    }
}
