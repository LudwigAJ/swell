//! Anthropic Claude API backend with OpenTelemetry instrumentation.

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

        #[derive(Serialize)]
        struct ApiMessage {
            role: &'static str,
            content: String,
        }

        #[derive(Serialize)]
        struct ApiTool {
            name: String,
            description: String,
            input_schema: serde_json::Value,
        }

        #[derive(Deserialize)]
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

        #[derive(Deserialize)]
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

        #[derive(Deserialize)]
        struct ResponseUsage {
            input_tokens: u64,
            output_tokens: u64,
        }

        // Prepare request data first
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

        // Make HTTP request
        let response = match self
            .client
            .post(ANTHROPIC_API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
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
                return Err(SwellError::LlmError(format!("Failed to parse response: {}", e)));
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
}
