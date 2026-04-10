//! Langfuse integration for LLM observability.
//!
//! This module provides integration with [Langfuse](https://langfuse.com/),
//! an open-source LLM engineering platform for observability, metrics, and evals.
//!
//! # Overview
//!
//! Langfuse provides comprehensive LLM observability through:
//! - **Traces**: End-to-end visibility into LLM application workflows
//! - **Generations**: Token usage, cost tracking, and model performance
//! - **User Feedback**: Collect and analyze human feedback on LLM outputs
//!
//! # Getting Started
//!
//! ```rust,ignore
//! use swell_core::langfuse::{LangfuseClient, LangfuseConfig};
//!
//! let config = LangfuseConfig::from_env()?;
//! let client = LangfuseClient::new(config)?;
//!
//! // Create and send a generation trace
//! let trace = client.new_generation("chat")
//!     .with_model("claude-3-5-sonnet")
//!     .with_usage(100, 50, 0.00375)
//!     .with_input("Hello, world!")
//!     .with_output("Hi there!");
//!
//! client.send(trace).await?;
//! ```
//!
//! # Environment Variables
//!
//! - `LANGFUSE_HOST`: Base URL of Langfuse instance (defaults to cloud)
//! - `LANGFUSE_PUBLIC_KEY`: Your Langfuse public key
//! - `LANGFUSE_SECRET_KEY`: Your Langfuse secret key

use base64::Engine;
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use uuid::Uuid;

// ============================================================================
// Configuration
// ============================================================================

/// Langfuse client configuration
#[derive(Debug, Clone)]
pub struct LangfuseConfig {
    /// Base URL of the Langfuse instance
    host: String,
    /// Public key for authentication
    public_key: String,
    /// Secret key for authentication
    secret_key: String,
    /// Service name for tracing (reserved for future use)
    #[allow(dead_code)]
    service_name: String,
}

impl LangfuseConfig {
    /// Create configuration from environment variables
    ///
    /// Reads from:
    /// - `LANGFUSE_HOST` (defaults to `https://cloud.langfuse.com`)
    /// - `LANGFUSE_PUBLIC_KEY`
    /// - `LANGFUSE_SECRET_KEY`
    pub fn from_env() -> Result<Self, LangfuseError> {
        let host = std::env::var("LANGFUSE_HOST")
            .unwrap_or_else(|_| "https://cloud.langfuse.com".to_string());
        let public_key = std::env::var("LANGFUSE_PUBLIC_KEY")
            .map_err(|_| LangfuseError::MissingConfig("LANGFUSE_PUBLIC_KEY".to_string()))?;
        let secret_key = std::env::var("LANGFUSE_SECRET_KEY")
            .map_err(|_| LangfuseError::MissingConfig("LANGFUSE_SECRET_KEY".to_string()))?;

        Ok(Self {
            host,
            public_key,
            secret_key,
            service_name: std::env::var("LANGFUSE_SERVICE_NAME")
                .unwrap_or_else(|_| "swell".to_string()),
        })
    }

    /// Create configuration with explicit values
    pub fn new(host: &str, public_key: &str, secret_key: &str, service_name: &str) -> Self {
        Self {
            host: host.to_string(),
            public_key: public_key.to_string(),
            secret_key: secret_key.to_string(),
            service_name: service_name.to_string(),
        }
    }

    /// Get the OTLP endpoint URL for traces
    fn otlp_endpoint(&self) -> String {
        format!(
            "{}/api/public/otel/v1/traces",
            self.host.trim_end_matches('/')
        )
    }

    /// Get the ingestion endpoint URL
    fn ingestion_endpoint(&self) -> String {
        format!("{}/api/public/ingestion", self.host.trim_end_matches('/'))
    }

    /// Get the authorization header value (Basic auth)
    fn auth_header(&self) -> String {
        let credentials = format!("{}:{}", self.public_key, self.secret_key);
        let encoded = base64::engine::general_purpose::STANDARD.encode(credentials);
        format!("Basic {}", encoded)
    }
}

/// Langfuse errors
#[derive(Debug, thiserror::Error)]
pub enum LangfuseError {
    #[error("Missing configuration: {0}")]
    MissingConfig(String),

    #[error("HTTP request failed: {0}")]
    RequestFailed(#[from] reqwest::Error),

    #[error("Export failed: {0}")]
    ExportFailed(String),

    #[error("Serialization failed: {0}")]
    SerializationFailed(#[from] serde_json::Error),
}

// ============================================================================
// Trace and Observation Types
// ============================================================================

/// Langfuse observation type
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationType {
    /// A general span
    Span,
    /// An LLM generation (has model + usage)
    Generation,
    /// An event (point-in-time)
    Event,
}

/// Langfuse span level
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SpanLevel {
    Debug,
    Default,
    Warning,
    Error,
}

/// Token usage details for Langfuse
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    /// Number of input/prompt tokens
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    /// Number of output/completion tokens
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    /// Total number of tokens
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
    /// Cost in USD
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
}

impl Usage {
    /// Create usage from token counts
    pub fn new(input_tokens: u64, output_tokens: u64, cost_usd: f64) -> Self {
        Self {
            input_tokens: Some(input_tokens),
            output_tokens: Some(output_tokens),
            total_tokens: Some(input_tokens + output_tokens),
            cost: Some(cost_usd),
        }
    }
}

/// Model information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model {
    /// The model name (e.g., "claude-3-5-sonnet")
    pub name: String,
    /// The provider (e.g., "anthropic", "openai")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

/// An observation (span/generation/event) in Langfuse
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    /// Unique identifier
    id: String,
    /// Trace ID
    trace_id: String,
    /// Observation name
    name: String,
    /// Start time (ISO 8601)
    start_time: DateTime<Utc>,
    /// End time (ISO 8601)
    end_time: DateTime<Utc>,
    /// Observation type
    #[serde(skip_serializing_if = "Option::is_none")]
    observation_type: Option<ObservationType>,
    /// Span level
    #[serde(skip_serializing_if = "Option::is_none")]
    level: Option<SpanLevel>,
    /// Status message
    #[serde(skip_serializing_if = "Option::is_none")]
    status_message: Option<String>,
    /// Input content
    #[serde(skip_serializing_if = "Option::is_none")]
    input: Option<String>,
    /// Output content
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<String>,
    /// Token usage
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<Usage>,
    /// Model info
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<Model>,
    /// Parent observation ID
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_observation_id: Option<String>,
    /// User ID
    #[serde(skip_serializing_if = "Option::is_none")]
    user_id: Option<String>,
    /// Session ID
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
}

impl Observation {
    /// Create a new observation
    pub fn new(name: impl Into<String>, trace_id: &str) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            trace_id: trace_id.to_string(),
            name: name.into(),
            start_time: now,
            end_time: now,
            observation_type: None,
            level: None,
            status_message: None,
            input: None,
            output: None,
            usage: None,
            model: None,
            parent_observation_id: None,
            user_id: None,
            session_id: None,
        }
    }

    /// Set as a generation (LLM call)
    pub fn as_generation(mut self) -> Self {
        self.observation_type = Some(ObservationType::Generation);
        self
    }

    /// Set the model
    pub fn with_model(mut self, model_name: impl Into<String>) -> Self {
        self.model = Some(Model {
            name: model_name.into(),
            provider: None,
        });
        self
    }

    /// Set the provider
    pub fn with_provider(mut self, provider: impl Into<String>) -> Self {
        if let Some(ref mut m) = self.model {
            m.provider = Some(provider.into());
        }
        self
    }

    /// Set token usage
    pub fn with_usage(mut self, input_tokens: u64, output_tokens: u64, cost_usd: f64) -> Self {
        self.usage = Some(Usage::new(input_tokens, output_tokens, cost_usd));
        self
    }

    /// Set input content
    pub fn with_input(mut self, input: impl Into<String>) -> Self {
        self.input = Some(input.into());
        self
    }

    /// Set output content
    pub fn with_output(mut self, output: impl Into<String>) -> Self {
        self.output = Some(output.into());
        self
    }

    /// Set the end time (call after completion)
    pub fn ended_at(mut self, end_time: DateTime<Utc>) -> Self {
        self.end_time = end_time;
        self
    }

    /// Set error status
    pub fn with_error(mut self, message: impl Into<String>) -> Self {
        self.level = Some(SpanLevel::Error);
        self.status_message = Some(message.into());
        self
    }

    /// Set parent observation ID
    pub fn with_parent(mut self, parent_id: impl Into<String>) -> Self {
        self.parent_observation_id = Some(parent_id.into());
        self
    }

    /// Set user ID
    pub fn with_user_id(mut self, user_id: impl Into<String>) -> Self {
        self.user_id = Some(user_id.into());
        self
    }

    /// Set session ID
    pub fn with_session_id(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    /// Get the observation ID
    pub fn id(&self) -> &str {
        &self.id
    }
}

/// A Langfuse trace containing observations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trace {
    /// Trace ID
    id: String,
    /// Timestamp
    timestamp: DateTime<Utc>,
    /// Trace name
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    /// User ID
    #[serde(skip_serializing_if = "Option::is_none")]
    user_id: Option<String>,
    /// Session ID
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    /// Release version
    #[serde(skip_serializing_if = "Option::is_none")]
    release: Option<String>,
    /// Tags
    #[serde(skip_serializing_if = "Option::is_none")]
    tags: Option<Vec<String>>,
    /// Input
    #[serde(skip_serializing_if = "Option::is_none")]
    input: Option<String>,
    /// Output
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<String>,
    /// Metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<serde_json::Value>,
    /// Observations (spans)
    #[serde(default)]
    observations: Vec<Observation>,
}

impl Trace {
    /// Create a new trace
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string().replace("-", ""),
            timestamp: Utc::now(),
            name: Some(name.into()),
            user_id: None,
            session_id: None,
            release: None,
            tags: None,
            input: None,
            output: None,
            metadata: None,
            observations: Vec::new(),
        }
    }

    /// Create a new generation observation attached to this trace
    pub fn new_generation(&self, name: impl Into<String>) -> Observation {
        Observation::new(name, &self.id).as_generation()
    }

    /// Create a new span observation attached to this trace
    pub fn new_span(&self, name: impl Into<String>) -> Observation {
        Observation::new(name, &self.id)
    }

    /// Add an observation to the trace
    pub fn add_observation(mut self, observation: Observation) -> Self {
        self.observations.push(observation);
        self
    }

    /// Set user ID
    pub fn with_user_id(mut self, user_id: impl Into<String>) -> Self {
        self.user_id = Some(user_id.into());
        self
    }

    /// Set session ID
    pub fn with_session_id(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    /// Set input
    pub fn with_input(mut self, input: impl Into<String>) -> Self {
        self.input = Some(input.into());
        self
    }

    /// Set output
    pub fn with_output(mut self, output: impl Into<String>) -> Self {
        self.output = Some(output.into());
        self
    }

    /// Set metadata
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
        self
    }

    /// Get the trace ID
    pub fn id(&self) -> &str {
        &self.id
    }
}

/// Ingestion request body for Langfuse
#[derive(Debug, Serialize)]
struct IngestionBody {
    #[serde(rename = "batch")]
    traces: Vec<Trace>,
}

// ============================================================================
// Client
// ============================================================================

/// Client for sending traces to Langfuse
#[derive(Debug, Clone)]
pub struct LangfuseClient {
    client: Client,
    config: LangfuseConfig,
}

impl LangfuseClient {
    /// Create a new Langfuse client
    pub fn new(config: LangfuseConfig) -> Result<Self, LangfuseError> {
        let client = Client::builder().timeout(Duration::from_secs(30)).build()?;

        Ok(Self { client, config })
    }

    /// Create a new trace with the given name
    pub fn new_trace(&self, name: impl Into<String>) -> Trace {
        Trace::new(name)
    }

    /// Send a trace to Langfuse
    pub async fn send(&self, trace: Trace) -> Result<(), LangfuseError> {
        let body = IngestionBody {
            traces: vec![trace],
        };

        let response = self
            .client
            .post(self.config.otlp_endpoint())
            .header("Authorization", self.config.auth_header())
            .header("x-langfuse-ingestion-version", "4")
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if response.status().is_success() {
            Ok(())
        } else {
            let status = response.status();
            let body_text = response.text().await.unwrap_or_default();
            Err(LangfuseError::ExportFailed(format!(
                "Langfuse returned {}: {}",
                status, body_text
            )))
        }
    }

    /// Send multiple traces at once
    pub async fn send_batch(&self, traces: Vec<Trace>) -> Result<(), LangfuseError> {
        if traces.is_empty() {
            return Ok(());
        }

        let body = IngestionBody { traces };

        let response = self
            .client
            .post(self.config.otlp_endpoint())
            .header("Authorization", self.config.auth_header())
            .header("x-langfuse-ingestion-version", "4")
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if response.status().is_success() {
            Ok(())
        } else {
            let status = response.status();
            let body_text = response.text().await.unwrap_or_default();
            Err(LangfuseError::ExportFailed(format!(
                "Langfuse returned {}: {}",
                status, body_text
            )))
        }
    }
}

// ============================================================================
// User Feedback
// ============================================================================

/// User feedback for a trace or observation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Feedback {
    /// The ID to score (trace or observation)
    id: String,
    /// Feedback score (typically -1, 0, or 1 for thumbs down/neutral/up)
    score: i8,
    /// Feedback category
    #[serde(skip_serializing_if = "Option::is_none")]
    category: Option<String>,
    /// Feedback message
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    /// Metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<serde_json::Value>,
}

impl Feedback {
    /// Create feedback for a trace or observation
    pub fn new(id: impl Into<String>, score: i8) -> Self {
        Self {
            id: id.into(),
            score,
            category: None,
            message: None,
            metadata: None,
        }
    }

    /// Create positive feedback
    pub fn positive(id: impl Into<String>) -> Self {
        Self::new(id, 1)
    }

    /// Create negative feedback
    pub fn negative(id: impl Into<String>) -> Self {
        Self::new(id, -1)
    }

    /// Set the category
    pub fn with_category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    /// Set the message
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }
}

/// Feedback client for submitting user feedback
#[derive(Debug, Clone)]
pub struct FeedbackClient {
    client: Client,
    config: LangfuseConfig,
}

impl FeedbackClient {
    /// Create a new feedback client
    pub fn new(config: LangfuseConfig) -> Result<Self, LangfuseError> {
        let client = Client::builder().timeout(Duration::from_secs(10)).build()?;

        Ok(Self { client, config })
    }

    /// Submit feedback to Langfuse
    pub async fn submit(&self, feedback: Feedback) -> Result<(), LangfuseError> {
        #[derive(Serialize)]
        struct FeedbackRequest {
            #[serde(rename = "type")]
            feedback_type: String,
            id: String,
            score: i8,
            category: Option<String>,
            message: Option<String>,
            metadata: Option<serde_json::Value>,
        }

        let request = FeedbackRequest {
            feedback_type: "feedback".to_string(),
            id: feedback.id,
            score: feedback.score,
            category: feedback.category,
            message: feedback.message,
            metadata: feedback.metadata,
        };

        let response = self
            .client
            .post(self.config.ingestion_endpoint())
            .header("Authorization", self.config.auth_header())
            .header("x-langfuse-ingestion-version", "4")
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        if response.status().is_success() {
            Ok(())
        } else {
            let status = response.status();
            let body_text = response.text().await.unwrap_or_default();
            Err(LangfuseError::ExportFailed(format!(
                "Langfuse feedback failed with {}: {}",
                status, body_text
            )))
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
    fn test_config_auth_header() {
        let config = LangfuseConfig::new(
            "https://cloud.langfuse.com",
            "pk-test-123",
            "sk-test-456",
            "test-service",
        );

        let auth = config.auth_header();
        assert!(auth.starts_with("Basic "));
        // Verify it's valid base64
        let encoded = auth.trim_start_matches("Basic ");
        assert!(base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .is_ok());
    }

    #[test]
    fn test_trace_creation() {
        let trace = Trace::new("test-trace");
        assert!(!trace.id.is_empty());
        assert_eq!(trace.name, Some("test-trace".to_string()));
        assert!(trace.observations.is_empty());
    }

    #[test]
    fn test_observation_creation() {
        let trace = Trace::new("test");
        let obs = trace.new_generation("chat");
        assert_eq!(obs.trace_id, trace.id);
        assert_eq!(obs.name, "chat");
    }

    #[test]
    fn test_observation_with_usage() {
        let trace = Trace::new("test");
        let obs = trace
            .new_generation("chat")
            .with_model("claude-3-5-sonnet")
            .with_usage(100, 50, 0.00375);

        assert!(obs.usage.is_some());
        let usage = obs.usage.unwrap();
        assert_eq!(usage.input_tokens, Some(100));
        assert_eq!(usage.output_tokens, Some(50));
        assert_eq!(usage.total_tokens, Some(150));
        assert!((usage.cost.unwrap() - 0.00375).abs() < f64::EPSILON);
    }

    #[test]
    fn test_feedback_creation() {
        let feedback = Feedback::positive("trace-123")
            .with_category("quality")
            .with_message("Great output!");

        assert_eq!(feedback.id, "trace-123");
        assert_eq!(feedback.score, 1);
        assert_eq!(feedback.category, Some("quality".to_string()));
        assert_eq!(feedback.message, Some("Great output!".to_string()));
    }

    #[tokio::test]
    async fn test_client_creation() {
        let config =
            LangfuseConfig::new("https://cloud.langfuse.com", "pk-test", "sk-test", "test");

        let client = LangfuseClient::new(config);
        assert!(client.is_ok());
    }

    #[tokio::test]
    async fn test_feedback_client_creation() {
        let config =
            LangfuseConfig::new("https://cloud.langfuse.com", "pk-test", "sk-test", "test");

        let client = FeedbackClient::new(config);
        assert!(client.is_ok());
    }

    #[test]
    fn test_trace_with_user_and_session() {
        let trace = Trace::new("test-trace")
            .with_user_id("user-123")
            .with_session_id("session-456");

        assert_eq!(trace.user_id, Some("user-123".to_string()));
        assert_eq!(trace.session_id, Some("session-456".to_string()));
    }

    #[test]
    fn test_observation_chain() {
        let trace = Trace::new("test");

        // Create parent observation
        let parent = trace.new_span("parent");
        let parent_id = parent.id().to_string();

        // Create child observation
        let child = trace
            .new_generation("llm-call")
            .with_parent(&parent_id)
            .with_model("claude-3-5-sonnet")
            .with_usage(100, 50, 0.00375);

        let trace = trace.add_observation(parent).add_observation(child);

        assert_eq!(trace.observations.len(), 2);
    }
}
