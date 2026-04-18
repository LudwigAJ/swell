//! LLM Backend implementations for SWELL.
//!
//! This crate provides the [`LlmBackend`] trait and implementations
//! for various LLM providers (Anthropic, OpenAI, etc.).
//!
//! # Usage
//!
//! ```rust,ignore
//! use swell_llm::{AnthropicBackend, LlmConfig, LlmMessage, LlmRole};
//!
//! let backend = AnthropicBackend::new("claude-sonnet-4-20250514", "sk-ant-api03-...");
//! let config = LlmConfig::default();
//! let messages = vec![
//!     LlmMessage { role: LlmRole::User, content: "Hello".to_string() }
//! ];
//! let response = backend.chat(messages, None, config).await?;
//! ```

pub mod anthropic;
pub mod credential;
pub mod mock;
pub mod openai;
pub mod retry;
pub mod router;
pub mod traits;
pub mod wiring;

pub use anthropic::AnthropicBackend;
pub use mock::{MockLlm, ScenarioMockLlm, ScenarioStep};
pub use openai::OpenAIBackend;
pub use retry::{calculate_backoff, is_retryable_status, LlmRetryConfig};
pub use router::{
    CostOptimizer, ModelRoute, ModelRouter, ModelRouterBuilder, RouteConfig, TaskType,
};
pub use swell_core::wiring::WiringReport;
pub use traits::*;

use std::sync::Arc;
use swell_core::{LlmBackend as CoreLlmBackend, SwellError};

/// Type alias for boxed LLM backend
pub type BoxLlmBackend = Arc<dyn CoreLlmBackend>;

/// Create a backend from a URL scheme
pub fn create_backend(url: &str, model: &str, api_key: &str) -> Result<BoxLlmBackend, SwellError> {
    if url.contains("anthropic") || url.contains("anthropic.com") {
        Ok(Arc::new(AnthropicBackend::new(model, api_key)))
    } else if url.contains("openai") || url.contains("openai.com") || url.contains("azure") {
        Ok(Arc::new(OpenAIBackend::new(model, api_key)?))
    } else {
        Err(SwellError::ConfigError(format!(
            "Unknown LLM provider: {}",
            url
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_backend() {
        let mock = MockLlm::new("gpt-4");
        let config = LlmConfig {
            temperature: 0.7,
            max_tokens: 4096,
            stop_sequences: None,
        };
        let messages = vec![LlmMessage {
            role: LlmRole::User,
            content: "Say hello".to_string(),
            ..Default::default()
        }];

        let response = mock.chat(messages, None, config).await.unwrap();
        assert!(!response.content.is_empty());
    }

    #[tokio::test]
    async fn test_anthropic_backend_creation() {
        let backend = AnthropicBackend::new("claude-sonnet-4-20250514", "test-key");
        assert_eq!(backend.model(), "claude-sonnet-4-20250514");
    }

    #[tokio::test]
    async fn test_openai_backend_creation() {
        let backend = OpenAIBackend::new("gpt-4", "test-key").unwrap();
        assert_eq!(backend.model(), "gpt-4");
    }
}
