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
    opentelemetry::LatencyTracker, StreamEvent, SwellError,
};
use tokio::sync::mpsc;

static CALL_COUNT: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
pub struct MockLlm {
    model: String,
    response: String,
    should_fail: bool,
    #[allow(dead_code)]
    call_count: u64,
    /// If true, echoes back user message content in the response
    echo_user: bool,
}

impl MockLlm {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            response: "Mock response".to_string(),
            should_fail: false,
            call_count: 0,
            echo_user: true,
        }
    }

    pub fn with_response(model: impl Into<String>, response: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            response: response.into(),
            should_fail: false,
            call_count: 0,
            echo_user: false,
        }
    }

    pub fn failing(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            response: String::new(),
            should_fail: true,
            call_count: 0,
            echo_user: false,
        }
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
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, SwellError>> + Send>>, SwellError> {
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

        let (tx, rx) = mpsc::channel::<Result<StreamEvent, SwellError>>(100);

        // Spawn a task to emit the stream events
        tokio::spawn(async move {
            // Emit text delta with the full content as both text and delta
            // Since this is a mock, we emit the entire response as one delta
            let _ = tx.send(Ok(StreamEvent::TextDelta {
                text: content.clone(),
                delta: content,
            })).await;

            // Emit usage
            let _ = tx.send(Ok(StreamEvent::Usage {
                input_tokens,
                output_tokens,
                cache_creation_input_tokens: None,
                cache_read_input_tokens: None,
            })).await;

            // Emit message stop
            let _ = tx.send(Ok(StreamEvent::MessageStop { stop_reason: None })).await;
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

    #[tokio::test]
    async fn test_mock_response() {
        let mock = MockLlm::new("gpt-4");
        let messages = vec![LlmMessage {
            role: LlmRole::User,
            content: "Hello".to_string(),
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
}
