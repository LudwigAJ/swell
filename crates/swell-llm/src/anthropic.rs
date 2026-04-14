//! Anthropic Claude API backend with OpenTelemetry instrumentation.
//!
//! # Prompt Caching
//!
//! This backend supports Anthropic's prompt caching feature. When system messages
//! are provided, they are sent with `cache_control: { type: "ephemeral" }` to enable
//! provider-managed cache creation. The API response includes `cache_creation_input_tokens`
//! (tokens written to cache) and `cache_read_input_tokens` (tokens read from cache).

use crate::{LlmBackend, LlmConfig, LlmMessage, LlmResponse, LlmRole, LlmToolDefinition, LlmUsage};
use async_trait::async_trait;
use opentelemetry::trace::{Span, Tracer};
use opentelemetry::KeyValue;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use swell_core::{
    opentelemetry::{gen_ai, pricing, GenAiSpanExt, LatencyTracker},
    record_llm_cost, LlmToolCall, SwellError,
};
use tracing::{debug, warn};

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
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
    client: Client,
}

impl AnthropicBackend {
    pub fn new(model: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            api_key: api_key.into(),
            client: Client::new(),
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

        // Make HTTP request with prompt caching beta header
        let response = match self
            .client
            .post(ANTHROPIC_API_URL)
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
        self.client.head(ANTHROPIC_API_URL).send().await.is_ok()
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
