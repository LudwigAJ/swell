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

/// A step in a scripted scenario - either a text response or a tool call.
///
/// This is used by [`ScenarioMockLlm`] to define multi-turn conversations.
#[derive(Debug, Clone)]
pub enum ScenarioStep {
    /// A text response without any tool calls
    Text(String),
    /// A response with a tool call that should be executed
    ToolUse {
        /// The tool call ID
        id: String,
        /// The tool name
        name: String,
        /// The tool arguments as JSON
        arguments: serde_json::Value,
        /// The result to return when this tool is called
        result: String,
        /// Whether the tool execution should be marked as successful
        success: bool,
    },
    /// A response with both text and a tool call
    TextWithToolUse {
        /// The text content
        text: String,
        /// The tool call ID
        id: String,
        /// The tool name
        name: String,
        /// The tool arguments as JSON
        arguments: serde_json::Value,
        /// The result to return when this tool is called
        result: String,
        /// Whether the tool execution should be marked as successful
        success: bool,
    },
}

impl ScenarioStep {
    /// Create a text-only step
    pub fn text(content: impl Into<String>) -> Self {
        ScenarioStep::Text(content.into())
    }

    /// Create a tool_use-only step
    pub fn tool_use(
        id: impl Into<String>,
        name: impl Into<String>,
        arguments: serde_json::Value,
        result: impl Into<String>,
        success: bool,
    ) -> Self {
        ScenarioStep::ToolUse {
            id: id.into(),
            name: name.into(),
            arguments,
            result: result.into(),
            success,
        }
    }

    /// Create a step with both text and tool use
    pub fn text_with_tool_use(
        text: impl Into<String>,
        id: impl Into<String>,
        name: impl Into<String>,
        arguments: serde_json::Value,
        result: impl Into<String>,
        success: bool,
    ) -> Self {
        ScenarioStep::TextWithToolUse {
            text: text.into(),
            id: id.into(),
            name: name.into(),
            arguments,
            result: result.into(),
            success,
        }
    }
}

/// Error returned when a scenario is exhausted or invalid.
#[derive(Debug, thiserror::Error)]
pub enum ScenarioError {
    #[error("Scenario exhausted: all {0} steps have been consumed")]
    Exhausted(usize),

    #[error("Invalid step index {index} for scenario with {total} steps")]
    InvalidIndex { index: usize, total: usize },
}

/// ScenarioMockLlm - a mock LLM that returns pre-scripted responses in sequence.
///
/// This is useful for deterministic testing of multi-turn agent interactions.
/// Each call to [`LlmBackend::chat`] or [`LlmBackend::stream`] returns the next
/// response in the scenario sequence.
///
/// # Example
/// ```ignore
/// use swell_llm::mock::{ScenarioMockLlm, ScenarioStep};
///
/// let scenario = vec![
///     ScenarioStep::tool_use("call_1", "file_read", json!({"path": "/tmp/test.txt"}), "contents", true),
///     ScenarioStep::text("File contents retrieved successfully"),
/// ];
/// let mock = ScenarioMockLlm::new("claude", scenario);
/// ```
#[derive(Debug)]
pub struct ScenarioMockLlm {
    model: String,
    steps: Vec<ScenarioStep>,
    current_index: std::sync::atomic::AtomicUsize,
}

impl ScenarioMockLlm {
    /// Create a new ScenarioMockLlm with the given scripted steps.
    pub fn new(model: impl Into<String>, steps: Vec<ScenarioStep>) -> Self {
        Self {
            model: model.into(),
            steps,
            current_index: 0.into(),
        }
    }

    /// Create a ScenarioMockLlm from a vector of text responses.
    ///
    /// Each string becomes a [`ScenarioStep::Text`] step.
    pub fn with_text_responses(model: impl Into<String>, responses: Vec<String>) -> Self {
        let steps = responses.into_iter().map(ScenarioStep::Text).collect();
        Self::new(model, steps)
    }

    /// Get the number of steps in this scenario.
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    /// Check if this scenario has no steps.
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// Get the current step index.
    pub fn current_index(&self) -> usize {
        self.current_index.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Reset the scenario to the beginning.
    pub fn reset(&self) {
        self.current_index
            .store(0, std::sync::atomic::Ordering::SeqCst);
    }

    /// Get the total number of steps.
    pub fn total_steps(&self) -> usize {
        self.steps.len()
    }
}

#[async_trait]
impl LlmBackend for ScenarioMockLlm {
    fn model(&self) -> &str {
        &self.model
    }

    async fn chat(
        &self,
        messages: Vec<LlmMessage>,
        _tools: Option<Vec<LlmToolDefinition>>,
        _config: LlmConfig,
    ) -> Result<LlmResponse, SwellError> {
        let index = self
            .current_index
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        if index >= self.steps.len() {
            return Err(SwellError::LlmError(format!(
                "Scenario exhausted: step {} requested but scenario only has {} steps",
                index,
                self.steps.len()
            )));
        }

        let step = &self.steps[index];
        let input_tokens: u64 = messages.iter().map(|m| m.content.len() as u64 / 4).sum();
        let output_tokens = 50u64;

        match step {
            ScenarioStep::Text(content) => Ok(LlmResponse {
                content: content.clone(),
                tool_calls: None,
                usage: crate::LlmUsage {
                    input_tokens,
                    output_tokens,
                    total_tokens: input_tokens + output_tokens,
                    cache_creation_input_tokens: None,
                    cache_read_input_tokens: None,
                },
            }),
            ScenarioStep::ToolUse {
                id,
                name,
                arguments,
                ..
            } => Ok(LlmResponse {
                content: String::new(),
                tool_calls: Some(vec![LlmToolCall {
                    id: id.clone(),
                    name: name.clone(),
                    arguments: arguments.clone(),
                }]),
                usage: crate::LlmUsage {
                    input_tokens,
                    output_tokens,
                    total_tokens: input_tokens + output_tokens,
                    cache_creation_input_tokens: None,
                    cache_read_input_tokens: None,
                },
            }),
            ScenarioStep::TextWithToolUse {
                text,
                id,
                name,
                arguments,
                ..
            } => Ok(LlmResponse {
                content: text.clone(),
                tool_calls: Some(vec![LlmToolCall {
                    id: id.clone(),
                    name: name.clone(),
                    arguments: arguments.clone(),
                }]),
                usage: crate::LlmUsage {
                    input_tokens,
                    output_tokens,
                    total_tokens: input_tokens + output_tokens,
                    cache_creation_input_tokens: None,
                    cache_read_input_tokens: None,
                },
            }),
        }
    }

    async fn health_check(&self) -> bool {
        true
    }

    async fn stream(
        &self,
        messages: Vec<LlmMessage>,
        _tools: Option<Vec<LlmToolDefinition>>,
        _config: LlmConfig,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, SwellError>> + Send>>, SwellError>
    {
        let index = self
            .current_index
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        if index >= self.steps.len() {
            return Err(SwellError::LlmError(format!(
                "Scenario exhausted: step {} requested but scenario only has {} steps",
                index,
                self.steps.len()
            )));
        }

        let step = &self.steps[index];
        let input_tokens: u64 = messages.iter().map(|m| m.content.len() as u64 / 4).sum();
        let output_tokens = 50u64;

        let (content, tool_call_opt) = match step {
            ScenarioStep::Text(text) => (text.clone(), None),
            ScenarioStep::ToolUse {
                id,
                name,
                arguments,
                result,
                success,
            } => {
                let tc = (
                    id.clone(),
                    name.clone(),
                    arguments.clone(),
                    result.clone(),
                    *success,
                );
                (String::new(), Some(tc))
            }
            ScenarioStep::TextWithToolUse {
                text,
                id,
                name,
                arguments,
                result,
                success,
            } => {
                let tc = (
                    id.clone(),
                    name.clone(),
                    arguments.clone(),
                    result.clone(),
                    *success,
                );
                (text.clone(), Some(tc))
            }
        };

        let (tx, rx) = mpsc::channel::<Result<StreamEvent, SwellError>>(100);

        tokio::spawn(async move {
            // If there's a tool call, emit ToolUse and ToolResult events first
            if let Some((id, name, arguments, result, success)) = tool_call_opt {
                let tool_call = LlmToolCall {
                    id: id.clone(),
                    name: name.clone(),
                    arguments: arguments.clone(),
                };

                let _ = tx.send(Ok(StreamEvent::ToolUse { tool_call })).await;

                let _ = tx
                    .send(Ok(StreamEvent::ToolResult {
                        tool_call_id: id,
                        result,
                        success,
                    }))
                    .await;
            }

            // Emit text delta if there's content
            if !content.is_empty() {
                let _ = tx
                    .send(Ok(StreamEvent::TextDelta {
                        text: content.clone(),
                        delta: content,
                    }))
                    .await;
            }

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
        match &events[1] {
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
        match &events[1] {
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

    // =========================================================================
    // ScenarioMockLlm Tests
    // =========================================================================

    #[tokio::test]
    async fn test_scenario_mock_basic_multi_turn() {
        // VAL-PROMPT-005: ScenarioMockLlm supports multi-turn scripted scenarios
        let scenario = vec![
            ScenarioStep::text("Response 1"),
            ScenarioStep::text("Response 2"),
            ScenarioStep::text("Response 3"),
        ];
        let mock = ScenarioMockLlm::new("claude", scenario);

        let messages = vec![LlmMessage {
            role: LlmRole::User,
            content: "Turn 1".to_string(),
            ..Default::default()
        }];

        // First call returns first response
        let response = mock
            .chat(messages.clone(), None, LlmConfig::default())
            .await
            .unwrap();
        assert_eq!(response.content, "Response 1");

        // Second call returns second response
        let response = mock
            .chat(messages.clone(), None, LlmConfig::default())
            .await
            .unwrap();
        assert_eq!(response.content, "Response 2");

        // Third call returns third response
        let response = mock
            .chat(messages.clone(), None, LlmConfig::default())
            .await
            .unwrap();
        assert_eq!(response.content, "Response 3");
    }

    #[tokio::test]
    async fn test_scenario_mock_exhaustion_error() {
        // Scenario exhaustion produces a clear error
        let scenario = vec![ScenarioStep::text("Only one")];
        let mock = ScenarioMockLlm::new("claude", scenario);

        let messages = vec![LlmMessage {
            role: LlmRole::User,
            content: "Turn 1".to_string(),
            ..Default::default()
        }];

        // First call succeeds
        let response = mock
            .chat(messages.clone(), None, LlmConfig::default())
            .await
            .unwrap();
        assert_eq!(response.content, "Only one");

        // Second call should fail with scenario exhausted error
        let err = mock
            .chat(messages.clone(), None, LlmConfig::default())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Scenario exhausted"));
    }

    #[tokio::test]
    async fn test_scenario_mock_with_tool_use() {
        // VAL-PROMPT-006: ScenarioMockLlm can script tool_use responses
        let scenario = vec![
            ScenarioStep::tool_use(
                "call_1",
                "file_read",
                serde_json::json!({"path": "/tmp/test.txt"}),
                "file contents",
                true,
            ),
            ScenarioStep::text("Done reading the file"),
        ];
        let mock = ScenarioMockLlm::new("claude", scenario);

        let messages = vec![LlmMessage {
            role: LlmRole::User,
            content: "Read the file".to_string(),
            ..Default::default()
        }];

        // First response should have tool_calls
        let response = mock
            .chat(messages.clone(), None, LlmConfig::default())
            .await
            .unwrap();
        assert!(response.content.is_empty());
        assert!(response.tool_calls.is_some());
        let tool_calls = response.tool_calls.unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id, "call_1");
        assert_eq!(tool_calls[0].name, "file_read");
        assert_eq!(tool_calls[0].arguments["path"], "/tmp/test.txt");

        // Second response should be text
        let response = mock
            .chat(messages.clone(), None, LlmConfig::default())
            .await
            .unwrap();
        assert_eq!(response.content, "Done reading the file");
        assert!(response.tool_calls.is_none());
    }

    #[tokio::test]
    async fn test_scenario_mock_stream_multi_turn() {
        // Test streaming across multiple turns
        let scenario = vec![
            ScenarioStep::text("First response"),
            ScenarioStep::text("Second response"),
        ];
        let mock = ScenarioMockLlm::new("claude", scenario);

        let messages = vec![LlmMessage {
            role: LlmRole::User,
            content: "Turn 1".to_string(),
            ..Default::default()
        }];

        // First stream
        let stream = mock
            .stream(messages.clone(), None, LlmConfig::default())
            .await
            .unwrap();
        let events: Vec<_> = stream.collect().await;
        let text1 = events
            .iter()
            .find_map(|e| match e {
                Ok(StreamEvent::TextDelta { text, .. }) => Some(text.clone()),
                _ => None,
            })
            .expect("Should have text");
        assert_eq!(text1, "First response");

        // Second stream
        let stream = mock
            .stream(messages.clone(), None, LlmConfig::default())
            .await
            .unwrap();
        let events: Vec<_> = stream.collect().await;
        let text2 = events
            .iter()
            .find_map(|e| match e {
                Ok(StreamEvent::TextDelta { text, .. }) => Some(text.clone()),
                _ => None,
            })
            .expect("Should have text");
        assert_eq!(text2, "Second response");
    }

    #[tokio::test]
    async fn test_scenario_mock_stream_with_tool_use() {
        // Test streaming with tool_use events
        let scenario = vec![ScenarioStep::text_with_tool_use(
            "File has been read",
            "call_1",
            "file_read",
            serde_json::json!({"path": "/tmp/test.txt"}),
            "file contents here",
            true,
        )];
        let mock = ScenarioMockLlm::new("claude", scenario);

        let messages = vec![LlmMessage {
            role: LlmRole::User,
            content: "Read the file".to_string(),
            ..Default::default()
        }];

        let stream = mock
            .stream(messages.clone(), None, LlmConfig::default())
            .await
            .unwrap();
        let events: Vec<_> = stream.collect().await;

        // Should have: ToolUse, ToolResult, TextDelta, Usage, MessageStop
        assert!(
            events.len() >= 5,
            "Expected at least 5 events, got {}",
            events.len()
        );

        // Find ToolUse event
        let tool_use = events
            .iter()
            .find_map(|e| match e {
                Ok(StreamEvent::ToolUse { tool_call }) => Some(tool_call.clone()),
                _ => None,
            })
            .expect("Should have ToolUse event");
        assert_eq!(tool_use.id, "call_1");
        assert_eq!(tool_use.name, "file_read");

        // Find ToolResult event
        let tool_result = events
            .iter()
            .find_map(|e| match e {
                Ok(StreamEvent::ToolResult {
                    tool_call_id,
                    result,
                    success,
                }) => Some((tool_call_id.clone(), result.clone(), *success)),
                _ => None,
            })
            .expect("Should have ToolResult event");
        assert_eq!(tool_result.0, "call_1");
        assert_eq!(tool_result.1, "file contents here");
        assert!(tool_result.2);

        // Find TextDelta event
        let text = events
            .iter()
            .find_map(|e| match e {
                Ok(StreamEvent::TextDelta { text, .. }) => Some(text.clone()),
                _ => None,
            })
            .expect("Should have TextDelta event");
        assert_eq!(text, "File has been read");
    }

    #[tokio::test]
    async fn test_scenario_mock_with_text_responses_helper() {
        // Test the with_text_responses helper
        let mock = ScenarioMockLlm::with_text_responses(
            "claude",
            vec!["A".to_string(), "B".to_string(), "C".to_string()],
        );

        let messages = vec![LlmMessage {
            role: LlmRole::User,
            content: "Test".to_string(),
            ..Default::default()
        }];

        assert_eq!(
            mock.chat(messages.clone(), None, LlmConfig::default())
                .await
                .unwrap()
                .content,
            "A"
        );
        assert_eq!(
            mock.chat(messages.clone(), None, LlmConfig::default())
                .await
                .unwrap()
                .content,
            "B"
        );
        assert_eq!(
            mock.chat(messages.clone(), None, LlmConfig::default())
                .await
                .unwrap()
                .content,
            "C"
        );
    }

    #[tokio::test]
    async fn test_scenario_mock_reset() {
        // Test the reset functionality
        let scenario = vec![ScenarioStep::text("First"), ScenarioStep::text("Second")];
        let mock = ScenarioMockLlm::new("claude", scenario);

        let messages = vec![LlmMessage {
            role: LlmRole::User,
            content: "Test".to_string(),
            ..Default::default()
        }];

        // Consume first response
        assert_eq!(
            mock.chat(messages.clone(), None, LlmConfig::default())
                .await
                .unwrap()
                .content,
            "First"
        );

        // Reset and consume again
        mock.reset();
        assert_eq!(
            mock.chat(messages.clone(), None, LlmConfig::default())
                .await
                .unwrap()
                .content,
            "First"
        );
        assert_eq!(
            mock.chat(messages.clone(), None, LlmConfig::default())
                .await
                .unwrap()
                .content,
            "Second"
        );
    }

    #[tokio::test]
    async fn test_scenario_mock_empty_scenario() {
        // Test behavior with empty scenario
        let scenario: Vec<ScenarioStep> = vec![];
        let mock = ScenarioMockLlm::new("claude", scenario);

        let messages = vec![LlmMessage {
            role: LlmRole::User,
            content: "Test".to_string(),
            ..Default::default()
        }];

        // Any call should fail
        let err = mock
            .chat(messages.clone(), None, LlmConfig::default())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Scenario exhausted"));
    }

    #[tokio::test]
    async fn test_scenario_mock_failed_tool() {
        // Test script for a failed tool execution
        let scenario = vec![ScenarioStep::tool_use(
            "call_fail",
            "shell",
            serde_json::json!({"command": "exit 1"}),
            "Command failed with exit code 1",
            false, // Tool failed
        )];
        let mock = ScenarioMockLlm::new("claude", scenario);

        let messages = vec![LlmMessage {
            role: LlmRole::User,
            content: "Run command".to_string(),
            ..Default::default()
        }];

        let response = mock
            .chat(messages.clone(), None, LlmConfig::default())
            .await
            .unwrap();
        assert!(response.tool_calls.is_some());
        assert_eq!(response.tool_calls.as_ref().unwrap()[0].name, "shell");
    }

    #[tokio::test]
    async fn test_scenario_mock_stream_exhaustion() {
        // Test streaming exhaustion error
        let scenario = vec![ScenarioStep::text("Only one")];
        let mock = ScenarioMockLlm::new("claude", scenario);

        let messages = vec![LlmMessage {
            role: LlmRole::User,
            content: "Test".to_string(),
            ..Default::default()
        }];

        // First stream succeeds
        let stream = mock
            .stream(messages.clone(), None, LlmConfig::default())
            .await
            .unwrap();
        let events: Vec<_> = stream.collect().await;
        assert!(!events.is_empty());

        // Second stream fails
        match mock
            .stream(messages.clone(), None, LlmConfig::default())
            .await
        {
            Ok(_) => panic!("Expected error but got Ok"),
            Err(err) => {
                assert!(err.to_string().contains("Scenario exhausted"));
            }
        }
    }

    #[tokio::test]
    async fn test_scenario_mock_model_name() {
        // Verify model name is returned correctly
        let scenario = vec![ScenarioStep::text("Response")];
        let mock = ScenarioMockLlm::new("gpt-5", scenario);
        assert_eq!(mock.model(), "gpt-5");
    }

    #[tokio::test]
    async fn test_scenario_mock_health_check() {
        // Verify health check returns true
        let scenario = vec![ScenarioStep::text("Response")];
        let mock = ScenarioMockLlm::new("claude", scenario);
        assert!(mock.health_check().await);
    }

    #[tokio::test]
    async fn test_scenario_step_helpers() {
        // Test the ScenarioStep helper methods
        let text_step = ScenarioStep::text("Hello");
        assert!(matches!(text_step, ScenarioStep::Text(s) if s == "Hello"));

        let tool_step = ScenarioStep::tool_use(
            "id1",
            "tool_name",
            serde_json::json!({"key": "value"}),
            "result",
            true,
        );
        assert!(matches!(
            tool_step,
            ScenarioStep::ToolUse { id, name, arguments: _, result: _, success }
            if id == "id1" && name == "tool_name" && success
        ));

        let combined = ScenarioStep::text_with_tool_use(
            "text content",
            "id2",
            "another_tool",
            serde_json::json!({"arg": 123}),
            "tool result",
            false,
        );
        assert!(matches!(
            combined,
            ScenarioStep::TextWithToolUse { text, id, name: _, arguments: _, result: _, success }
            if text == "text content" && id == "id2" && !success
        ));
    }

    #[tokio::test]
    async fn test_scenario_mock_info_methods() {
        // Test len, is_empty, current_index, total_steps
        let scenario = vec![
            ScenarioStep::text("1"),
            ScenarioStep::text("2"),
            ScenarioStep::text("3"),
        ];
        let mock = ScenarioMockLlm::new("claude", scenario);

        assert_eq!(mock.len(), 3);
        assert!(!mock.is_empty());
        assert_eq!(mock.current_index(), 0);
        assert_eq!(mock.total_steps(), 3);

        // Consume one
        let messages = vec![LlmMessage {
            role: LlmRole::User,
            content: "Test".to_string(),
            ..Default::default()
        }];
        mock.chat(messages.clone(), None, LlmConfig::default())
            .await
            .unwrap();

        assert_eq!(mock.current_index(), 1);
    }
}
