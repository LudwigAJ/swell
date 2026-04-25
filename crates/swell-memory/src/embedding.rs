//! Embedding client for generating text embeddings via Voyage AI API.
//!
//! This module provides:
//! - `EmbeddingModel` enum with Voyage Code 3 variant
//! - `EmbeddingClient` trait for embedding generation
//! - `VoyageEmbeddingClient` implementation with real HTTP requests
//!
//! # Voyage AI API
//!
//! The VoyageEmbeddingClient sends requests to `https://api.voyageai.com/v1/embeddings`
//! with the following structure:
//!
//! ```json
//! {
//!     "model": "voyage-code-3",
//!     "input": ["text1", "text2", ...]
//! }
//! ```
//!
//! Response format:
//! ```json
//! {
//!     "data": [
//!         {"embedding": [0.1, 0.2, ...], "index": 0},
//!         {"embedding": [0.3, 0.4, ...], "index": 1}
//!     ],
//!     "model": "voyage-code-3",
//!     "usage": {"prompt_tokens": 10, "total_tokens": 10}
//! }
//! ```

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};

pub use swell_core::SwellError;

/// Embedding model variants supported by the embedding client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmbeddingModel {
    /// Voyage Code 3 model - optimized for code understanding
    VoyageCode3,
}

impl EmbeddingModel {
    /// Returns the model identifier string used in API requests.
    pub fn as_str(&self) -> &'static str {
        match self {
            EmbeddingModel::VoyageCode3 => "voyage-code-3",
        }
    }
}

impl std::fmt::Display for EmbeddingModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Trait for embedding clients that generate text embeddings.
#[async_trait]
pub trait EmbeddingClient: Send + Sync {
    /// Generate embeddings for the given texts.
    ///
    /// # Arguments
    /// * `texts` - A slice of strings to generate embeddings for
    ///
    /// # Returns
    /// A vector of embedding vectors, where each inner vector contains
    /// the embedding values for the corresponding text in the input.
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, SwellError>;
}

/// Configuration for VoyageEmbeddingClient.
#[derive(Debug, Clone)]
pub struct VoyageEmbeddingConfig {
    /// API key for Voyage AI (loaded from environment)
    pub api_key: String,
    /// Base URL for the Voyage API
    pub base_url: String,
    /// Request timeout duration
    pub timeout_secs: u64,
}

impl Default for VoyageEmbeddingConfig {
    fn default() -> Self {
        Self {
            api_key: std::env::var("VOYAGE_API_KEY")
                .expect("VOYAGE_API_KEY environment variable must be set"),
            base_url: "https://api.voyageai.com".to_string(),
            timeout_secs: 60,
        }
    }
}

impl VoyageEmbeddingConfig {
    /// Create a new config with the given API key.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.voyageai.com".to_string(),
            timeout_secs: 60,
        }
    }

    /// Set a custom base URL.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Set a custom timeout.
    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }
}

/// Voyage AI embedding client implementation.
#[derive(Clone)]
pub struct VoyageEmbeddingClient {
    config: VoyageEmbeddingConfig,
    client: Client,
    model: EmbeddingModel,
}

impl VoyageEmbeddingClient {
    /// Create a new VoyageEmbeddingClient with default configuration.
    pub fn new() -> Result<Self, SwellError> {
        Self::with_config(VoyageEmbeddingConfig::default())
    }

    /// Create a new VoyageEmbeddingClient with custom configuration.
    pub fn with_config(config: VoyageEmbeddingConfig) -> Result<Self, SwellError> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()
            .map_err(|e| SwellError::LlmError(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self {
            config,
            client,
            model: EmbeddingModel::VoyageCode3,
        })
    }

    /// Create a builder for constructing a VoyageEmbeddingClient.
    pub fn builder() -> VoyageEmbeddingClientBuilder {
        VoyageEmbeddingClientBuilder::new()
    }

    /// Get the base URL being used.
    pub fn base_url(&self) -> &str {
        &self.config.base_url
    }

    /// Get the model being used.
    pub fn model(&self) -> &EmbeddingModel {
        &self.model
    }
}

impl Default for VoyageEmbeddingClient {
    fn default() -> Self {
        Self::new().expect("Failed to create VoyageEmbeddingClient with default config")
    }
}

/// Builder for VoyageEmbeddingClient.
#[derive(Debug, Clone)]
pub struct VoyageEmbeddingClientBuilder {
    api_key: Option<String>,
    base_url: Option<String>,
    timeout_secs: Option<u64>,
}

impl VoyageEmbeddingClientBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self {
            api_key: None,
            base_url: None,
            timeout_secs: None,
        }
    }

    /// Set the API key.
    pub fn with_api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(api_key.into());
        self
    }

    /// Set the base URL.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = Some(base_url.into());
        self
    }

    /// Set the timeout in seconds.
    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = Some(secs);
        self
    }

    /// Build the VoyageEmbeddingClient.
    pub fn build(self) -> Result<VoyageEmbeddingClient, SwellError> {
        let api_key = self.api_key.unwrap_or_else(|| {
            std::env::var("VOYAGE_API_KEY").expect("VOYAGE_API_KEY must be set")
        });

        let base_url = self
            .base_url
            .unwrap_or_else(|| "https://api.voyageai.com".to_string());

        let timeout_secs = self.timeout_secs.unwrap_or(60);

        let config = VoyageEmbeddingConfig {
            api_key,
            base_url,
            timeout_secs,
        };

        VoyageEmbeddingClient::with_config(config)
    }
}

impl Default for VoyageEmbeddingClientBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// API Request/Response Types
// ============================================================================

#[derive(Debug, Serialize)]
struct EmbeddingsRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct EmbeddingsResponse {
    data: Vec<EmbeddingData>,
    model: String,
    usage: Usage,
}

#[derive(Debug, Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
    index: usize,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct Usage {
    prompt_tokens: usize,
    total_tokens: usize,
}

// ============================================================================
// EmbeddingClient Implementation
// ============================================================================

#[async_trait]
impl EmbeddingClient for VoyageEmbeddingClient {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, SwellError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let url = format!("{}/v1/embeddings", self.config.base_url);

        let request = EmbeddingsRequest {
            model: self.model.as_str(),
            input: texts,
        };

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| SwellError::LlmError(format!("HTTP request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(SwellError::LlmError(format!(
                "Voyage API error ({}): {}",
                status, body
            )));
        }

        let embeddings_response: EmbeddingsResponse = response
            .json()
            .await
            .map_err(|e| SwellError::LlmError(format!("Failed to parse response: {}", e)))?;

        // Sort by index to ensure correct ordering
        let mut embeddings: Vec<Vec<f32>> = vec![Vec::new(); texts.len()];
        for data in embeddings_response.data {
            if data.index < texts.len() {
                embeddings[data.index] = data.embedding;
            }
        }

        Ok(embeddings)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Server;

    #[tokio::test]
    async fn test_embedding_model_display() {
        assert_eq!(EmbeddingModel::VoyageCode3.to_string(), "voyage-code-3");
    }

    #[tokio::test]
    async fn test_embedding_client_trait_object() {
        // Test that we can use trait objects
        let client: VoyageEmbeddingClient = VoyageEmbeddingClient::builder()
            .with_api_key("test-key")
            .with_base_url("http://localhost")
            .with_timeout(30)
            .build()
            .unwrap();

        // Verify the client was built correctly
        assert_eq!(client.base_url(), "http://localhost");
    }

    #[tokio::test]
    async fn test_voyage_embedding_client_with_mock_server() {
        // Create a mockito server
        let mut server = Server::new_async().await;

        // Define the expected request and response
        let request_model = "voyage-code-3";
        let response_embeddings = [vec![0.1, 0.2, 0.3, 0.4], vec![0.5, 0.6, 0.7, 0.8]];

        let mock_response = serde_json::json!({
            "data": [
                {"embedding": response_embeddings[0].clone(), "index": 0},
                {"embedding": response_embeddings[1].clone(), "index": 1}
            ],
            "model": request_model,
            "usage": {"prompt_tokens": 10, "total_tokens": 10}
        });

        // Create a mock that matches the expected headers and path
        let m = server
            .mock("POST", "/v1/embeddings")
            .match_header("Authorization", "Bearer test-api-key")
            .match_header("Content-Type", "application/json")
            .with_body(serde_json::to_string(&mock_response).unwrap())
            .create();

        // Create client pointing to mock server
        let client = VoyageEmbeddingClient::builder()
            .with_api_key("test-api-key")
            .with_base_url(server.url())
            .with_timeout(30)
            .build()
            .unwrap();

        // Call embed
        let texts = vec!["hello world".to_string(), "goodbye world".to_string()];
        let result = client.embed(&texts).await;

        // Assert success
        assert!(result.is_ok());
        let embeddings = result.unwrap();

        // Assert correct number of embeddings
        assert_eq!(embeddings.len(), 2);

        // Assert embeddings match (with some tolerance for floating point)
        for (i, emb) in embeddings.iter().enumerate() {
            assert_eq!(emb.len(), response_embeddings[i].len());
            for (j, val) in emb.iter().enumerate() {
                assert!((val - response_embeddings[i][j]).abs() < 0.001);
            }
        }

        m.assert();
    }

    #[tokio::test]
    async fn test_embed_empty_texts() {
        let server = Server::new_async().await;

        let client = VoyageEmbeddingClient::builder()
            .with_api_key("test-api-key")
            .with_base_url(server.url())
            .build()
            .unwrap();

        let result = client.embed(&[]).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_embed_single_text() {
        let mut server = Server::new_async().await;

        let response_embeddings = vec![0.1, 0.2, 0.3, 0.4];

        let mock_response = serde_json::json!({
            "data": [
                {"embedding": response_embeddings.clone(), "index": 0}
            ],
            "model": "voyage-code-3",
            "usage": {"prompt_tokens": 5, "total_tokens": 5}
        });

        let m = server
            .mock("POST", "/v1/embeddings")
            .match_header("Authorization", "Bearer test-api-key")
            .match_header("Content-Type", "application/json")
            .with_body(serde_json::to_string(&mock_response).unwrap())
            .create();

        let client = VoyageEmbeddingClient::builder()
            .with_api_key("test-api-key")
            .with_base_url(server.url())
            .build()
            .unwrap();

        let result = client.embed(&["single text".to_string()]).await;

        assert!(result.is_ok());
        let embeddings = result.unwrap();
        assert_eq!(embeddings.len(), 1);
        assert_eq!(embeddings[0], response_embeddings);

        m.assert();
    }

    #[tokio::test]
    async fn test_embed_api_error_response() {
        let mut server = Server::new_async().await;

        // Return an error response
        let m = server
            .mock("POST", "/v1/embeddings")
            .match_header("Authorization", "Bearer test-api-key")
            .with_status(401)
            .with_body(r#"{"error": "Invalid API key"}"#)
            .create();

        let client = VoyageEmbeddingClient::builder()
            .with_api_key("test-api-key")
            .with_base_url(server.url())
            .build()
            .unwrap();

        let result = client.embed(&["test".to_string()]).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("401"));

        m.assert();
    }

    #[tokio::test]
    async fn test_voyage_embedding_config_default() {
        // This test will panic if VOYAGE_API_KEY is not set, which is expected
        // in test environment without the env var
        let result = std::env::var("VOYAGE_API_KEY");
        if result.is_err() {
            // Expected in test environment - skip
            return;
        }

        let config = VoyageEmbeddingConfig::default();
        assert_eq!(config.base_url, "https://api.voyageai.com");
        assert_eq!(config.timeout_secs, 60);
    }

    #[tokio::test]
    async fn test_voyage_embedding_config_builder() {
        let config = VoyageEmbeddingConfig::new("my-api-key")
            .with_base_url("https://custom.voyageai.com")
            .with_timeout(120);

        assert_eq!(config.api_key, "my-api-key");
        assert_eq!(config.base_url, "https://custom.voyageai.com");
        assert_eq!(config.timeout_secs, 120);
    }

    #[tokio::test]
    async fn test_embedding_model_as_str() {
        assert_eq!(EmbeddingModel::VoyageCode3.as_str(), "voyage-code-3");
    }
}
