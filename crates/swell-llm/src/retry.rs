//! Retry logic with exponential backoff for LLM backend HTTP requests.
//!
//! # Retry Behavior
//!
//! - **Retryable errors**: HTTP 429 (rate limit) and 5xx (server error) responses
//! - **Non-retryable errors**: HTTP 400 (bad request), 401 (unauthorized), 403 (forbidden)
//! - **Backoff formula**: `base_delay * 2^attempt` seconds (exponential)
//! - **Configurable**: Retry count and base delay via [`LlmRetryConfig`]

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

/// Configuration for LLM retry behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmRetryConfig {
    /// Maximum number of retry attempts (default: 3)
    pub max_retries: u32,
    /// Base delay in seconds for exponential backoff (default: 1.0)
    pub base_delay_secs: f64,
    /// Maximum delay in seconds (default: 60.0)
    pub max_delay_secs: f64,
}

impl Default for LlmRetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay_secs: 1.0,
            max_delay_secs: 60.0,
        }
    }
}

impl LlmRetryConfig {
    /// Create a new retry config with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set maximum retry attempts.
    pub fn with_max_retries(mut self, retries: u32) -> Self {
        self.max_retries = retries;
        self
    }

    /// Set base delay in seconds.
    pub fn with_base_delay_secs(mut self, delay_secs: f64) -> Self {
        self.base_delay_secs = delay_secs;
        self
    }

    /// Set maximum delay in seconds.
    pub fn with_max_delay_secs(mut self, delay_secs: f64) -> Self {
        self.max_delay_secs = delay_secs;
        self
    }
}

/// Determines if an HTTP status code represents a retryable error.
///
/// Retryable:
/// - 429 Too Many Requests (rate limit)
/// - 500 Internal Server Error
/// - 502 Bad Gateway
/// - 503 Service Unavailable
/// - 504 Gateway Timeout
///
/// Non-retryable (fail immediately):
/// - 400 Bad Request
/// - 401 Unauthorized
/// - 403 Forbidden
/// - 404 Not Found
/// - 422 Unprocessable Entity
/// - Other 4xx errors
pub fn is_retryable_status(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS
        || status.as_u16() >= 500 && status.as_u16() < 600
}

/// Calculate exponential backoff delay in seconds.
///
/// Formula: `min(base_delay * 2^attempt, max_delay)`
///
/// # Arguments
///
/// * `attempt` - The current retry attempt (0-indexed, so first retry is attempt=1)
/// * `config` - The retry configuration
///
/// # Examples
///
/// ```
/// use swell_llm::retry::{calculate_backoff, LlmRetryConfig};
///
/// let config = LlmRetryConfig::default();
/// // attempt=0 (initial request): 1.0 * 2^0 = 1.0s
/// // attempt=1 (first retry): 1.0 * 2^1 = 2.0s
/// // attempt=2 (second retry): 1.0 * 2^2 = 4.0s
/// // attempt=3 (third retry): 1.0 * 2^3 = 8.0s
/// ```
pub fn calculate_backoff(attempt: u32, config: &LlmRetryConfig) -> f64 {
    let delay = config.base_delay_secs * 2.0_f64.powi(attempt as i32);
    delay.min(config.max_delay_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retry_config_defaults() {
        let config = LlmRetryConfig::default();
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.base_delay_secs, 1.0);
        assert_eq!(config.max_delay_secs, 60.0);
    }

    #[test]
    fn test_retry_config_builder() {
        let config = LlmRetryConfig::new()
            .with_max_retries(5)
            .with_base_delay_secs(2.0)
            .with_max_delay_secs(30.0);

        assert_eq!(config.max_retries, 5);
        assert_eq!(config.base_delay_secs, 2.0);
        assert_eq!(config.max_delay_secs, 30.0);
    }

    #[test]
    fn test_is_retryable_status_rate_limit() {
        assert!(is_retryable_status(StatusCode::TOO_MANY_REQUESTS));
    }

    #[test]
    fn test_is_retryable_status_server_errors() {
        assert!(is_retryable_status(StatusCode::INTERNAL_SERVER_ERROR));
        assert!(is_retryable_status(StatusCode::BAD_GATEWAY));
        assert!(is_retryable_status(StatusCode::SERVICE_UNAVAILABLE));
        assert!(is_retryable_status(StatusCode::GATEWAY_TIMEOUT));
    }

    #[test]
    fn test_is_not_retryable_status_client_errors() {
        assert!(!is_retryable_status(StatusCode::BAD_REQUEST));
        assert!(!is_retryable_status(StatusCode::UNAUTHORIZED));
        assert!(!is_retryable_status(StatusCode::FORBIDDEN));
        assert!(!is_retryable_status(StatusCode::NOT_FOUND));
        assert!(!is_retryable_status(StatusCode::UNPROCESSABLE_ENTITY));
    }

    #[test]
    fn test_is_not_retryable_status_success() {
        assert!(!is_retryable_status(StatusCode::OK));
        assert!(!is_retryable_status(StatusCode::CREATED));
    }

    #[test]
    fn test_calculate_backoff_default() {
        let config = LlmRetryConfig::default();

        // attempt 0 (initial): 1.0 * 2^0 = 1.0
        assert!((calculate_backoff(0, &config) - 1.0).abs() < 0.001);

        // attempt 1 (first retry): 1.0 * 2^1 = 2.0
        assert!((calculate_backoff(1, &config) - 2.0).abs() < 0.001);

        // attempt 2 (second retry): 1.0 * 2^2 = 4.0
        assert!((calculate_backoff(2, &config) - 4.0).abs() < 0.001);

        // attempt 3 (third retry): 1.0 * 2^3 = 8.0
        assert!((calculate_backoff(3, &config) - 8.0).abs() < 0.001);
    }

    #[test]
    fn test_calculate_backoff_caps_at_max() {
        let config = LlmRetryConfig::new()
            .with_base_delay_secs(10.0)
            .with_max_delay_secs(30.0);

        // attempt 0: 10.0 * 2^0 = 10.0
        assert!((calculate_backoff(0, &config) - 10.0).abs() < 0.001);

        // attempt 1: 10.0 * 2^1 = 20.0
        assert!((calculate_backoff(1, &config) - 20.0).abs() < 0.001);

        // attempt 2: 10.0 * 2^2 = 40.0, but capped at 30.0
        assert!((calculate_backoff(2, &config) - 30.0).abs() < 0.001);

        // attempt 3: also capped at 30.0
        assert!((calculate_backoff(3, &config) - 30.0).abs() < 0.001);
    }

    #[test]
    fn test_calculate_backoff_custom_base() {
        let config = LlmRetryConfig::new()
            .with_base_delay_secs(0.5);

        // attempt 0: 0.5 * 2^0 = 0.5
        assert!((calculate_backoff(0, &config) - 0.5).abs() < 0.001);

        // attempt 1: 0.5 * 2^1 = 1.0
        assert!((calculate_backoff(1, &config) - 1.0).abs() < 0.001);

        // attempt 2: 0.5 * 2^2 = 2.0
        assert!((calculate_backoff(2, &config) - 2.0).abs() < 0.001);
    }
}
