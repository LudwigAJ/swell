//! Mock LLM backend for testing.

use crate::{LlmMessage, LlmResponse, LlmConfig, LlmToolDefinition, LlmBackend};
use swell_core::SwellError;
use async_trait::async_trait;
use std::sync::atomic::{AtomicU64, Ordering};

static CALL_COUNT: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
pub struct MockLlm {
    model: String,
    response: String,
    should_fail: bool,
    call_count: u64,
}

impl MockLlm {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            response: "Mock response".to_string(),
            should_fail: false,
            call_count: 0,
        }
    }

    pub fn with_response(model: impl Into<String>, response: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            response: response.into(),
            should_fail: false,
            call_count: 0,
        }
    }

    pub fn failing(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            response: String::new(),
            should_fail: true,
            call_count: 0,
        }
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
        
        if self.should_fail {
            return Err(SwellError::LlmError("Mock failure".to_string()));
        }

        // Echo back the user message for testing
        let user_message = messages
            .iter()
            .find(|m| m.role == crate::LlmRole::User)
            .map(|m| m.content.clone())
            .unwrap_or_else(|| "No user message".to_string());

        let input_tokens: u64 = messages
            .iter()
            .map(|m| m.content.len() as u64 / 4)
            .sum();

        Ok(LlmResponse {
            content: format!("{}: {}", self.response, user_message),
            tool_calls: None,
            usage: crate::LlmUsage {
                input_tokens,
                output_tokens: 50,
                total_tokens: input_tokens + 50,
            },
        })
    }

    async fn health_check(&self) -> bool {
        !self.should_fail
    }
}

pub fn get_call_count() -> u64 {
    CALL_COUNT.load(Ordering::SeqCst)
}

pub fn reset_call_count() {
    CALL_COUNT.store(0, Ordering::SeqCst);
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
            .chat(messages, None, LlmConfig {
                temperature: 0.7,
                max_tokens: 4096,
                stop_sequences: None,
            })
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
            .chat(messages, None, LlmConfig {
                temperature: 0.7,
                max_tokens: 4096,
                stop_sequences: None,
            })
            .await
            .unwrap();

        assert!(response.content.contains("Custom reply"));
        assert!(response.content.contains("Test"));
    }

    #[tokio::test]
    async fn test_mock_failure() {
        let mock = MockLlm::failing("gpt-4");
        let messages = vec![LlmMessage {
            role: LlmRole::User,
            content: "Hello".to_string(),
        }];

        let result = mock.chat(messages, None, LlmConfig {
            temperature: 0.7,
            max_tokens: 4096,
            stop_sequences: None,
        }).await;
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
