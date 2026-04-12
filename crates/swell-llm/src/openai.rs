//! OpenAI API backend with OpenTelemetry instrumentation.

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

#[derive(Debug, Clone)]
pub struct OpenAIBackend {
    model: String,
    api_key: String,
    base_url: String,
    client: Client,
}

impl OpenAIBackend {
    pub fn new(model: impl Into<String>, api_key: impl Into<String>) -> Result<Self, SwellError> {
        Ok(Self {
            model: model.into(),
            api_key: api_key.into(),
            base_url: "https://api.openai.com/v1".to_string(),
            client: Client::new(),
        })
    }

    pub fn with_base_url(
        model: impl Into<String>,
        api_key: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Result<Self, SwellError> {
        Ok(Self {
            model: model.into(),
            api_key: api_key.into(),
            base_url: base_url.into(),
            client: Client::new(),
        })
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
impl LlmBackend for OpenAIBackend {
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
            temperature: f32,
            max_tokens: u64,
            #[serde(skip_serializing_if = "Option::is_none")]
            stop: Option<Vec<String>>,
        }

        #[derive(Serialize)]
        struct ApiMessage {
            role: &'static str,
            content: String,
        }

        #[derive(Serialize)]
        struct ApiTool {
            #[serde(rename = "type")]
            tool_type: &'static str,
            function: ApiFunction,
        }

        #[derive(Serialize)]
        struct ApiFunction {
            name: String,
            description: String,
            parameters: serde_json::Value,
        }

        #[derive(Deserialize)]
        struct Response {
            #[allow(dead_code)]
            id: Option<String>,
            choices: Vec<ResponseChoice>,
            usage: ResponseUsage,
        }

        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct ResponseChoice {
            message: ResponseMessage,
            finish_reason: String,
        }

        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct ResponseMessage {
            role: String,
            content: Option<String>,
            #[serde(default)]
            tool_calls: Option<Vec<ResponseToolCall>>,
        }

        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct ResponseToolCall {
            id: String,
            #[serde(rename = "type")]
            call_type: String,
            function: ResponseFunction,
        }

        #[derive(Deserialize)]
        struct ResponseFunction {
            name: String,
            arguments: String,
        }

        #[derive(Deserialize)]
        struct ResponseUsage {
            prompt_tokens: u64,
            completion_tokens: u64,
            total_tokens: u64,
        }

        // Prepare request data
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
                    tool_type: "function",
                    function: ApiFunction {
                        name: t.name,
                        description: t.description,
                        parameters: t.input_schema,
                    },
                })
                .collect()
        });

        let request = Request {
            model: &self.model,
            messages: api_messages,
            tools: api_tools,
            temperature: config.temperature,
            max_tokens: config.max_tokens,
            stop: config.stop_sequences,
        };

        let latency = LatencyTracker::new();

        debug!(model = %self.model, "OpenAI API request");

        let url = format!("{}/chat/completions", self.base_url);

        // Make HTTP request
        let response = match self
            .client
            .post(&url)
            .header("authorization", format!("Bearer {}", self.api_key))
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
            warn!(status = %status, body = %body, "OpenAI API error");
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

        let choice = match api_response.choices.into_iter().next() {
            Some(choice) => choice,
            None => {
                return Err(SwellError::LlmError("No choices in response".to_string()));
            }
        };

        let content = choice.message.content.unwrap_or_default();

        let tool_calls: Option<Vec<LlmToolCall>> =
            choice
                .message
                .tool_calls
                .map(|calls: Vec<ResponseToolCall>| {
                    calls
                        .into_iter()
                        .map(|call| LlmToolCall {
                            id: call.id,
                            name: call.function.name,
                            arguments: serde_json::from_str(&call.function.arguments)
                                .unwrap_or(serde_json::Value::Object(serde_json::Map::new())),
                        })
                        .collect()
                });

        let input_tokens = api_response.usage.prompt_tokens;
        let output_tokens = api_response.usage.completion_tokens;

        // Record GenAI attributes on the span
        let tracer = self.tracer();
        let span_name = format!("OpenAI chat {}", self.model);
        let mut span_builder = tracer.span_builder(span_name);
        span_builder.attributes = Some(vec![
            KeyValue::new(gen_ai::OPERATION_NAME, "chat".to_string()),
            KeyValue::new(gen_ai::PROVIDER_NAME, "openai".to_string()),
            KeyValue::new(gen_ai::REQUEST_MODEL, self.model.clone()),
        ]);

        let mut span = tracer.build(span_builder);
        span.record_prompt_tokens(input_tokens);
        span.record_completion_tokens(output_tokens);
        span.record_latency_ms(latency_ms);
        span.record_response_model(&self.model);

        // Calculate and record cost
        let pricing = pricing::for_model(&self.model);
        let cost = pricing.calculate_cost(input_tokens, output_tokens);
        span.record_cost_usd(cost);

        span.end();

        // Record cost to global tracker for dashboard integration
        record_llm_cost(api_response.usage.total_tokens, &self.model);

        Ok(LlmResponse {
            content,
            tool_calls,
            usage: LlmUsage {
                input_tokens,
                output_tokens,
                total_tokens: api_response.usage.total_tokens,
            },
        })
    }

    async fn health_check(&self) -> bool {
        let url = format!("{}/models/{}", self.base_url, self.model);
        self.client
            .get(&url)
            .header("authorization", format!("Bearer {}", self.api_key))
            .send()
            .await
            .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backend_model_name() {
        let backend = OpenAIBackend::new("gpt-4-turbo", "fake-key").unwrap();
        assert_eq!(backend.model(), "gpt-4-turbo");
    }

    #[test]
    fn test_role_conversion() {
        assert_eq!(OpenAIBackend::convert_role(LlmRole::System), "system");
        assert_eq!(OpenAIBackend::convert_role(LlmRole::User), "user");
        assert_eq!(OpenAIBackend::convert_role(LlmRole::Assistant), "assistant");
    }
}
