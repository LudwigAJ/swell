//! OpenAI API backend with OpenTelemetry instrumentation.

use crate::{LlmBackend, LlmConfig, LlmMessage, LlmResponse, LlmRole, LlmToolDefinition, LlmUsage};
use async_trait::async_trait;
use opentelemetry::trace::{Span, Tracer};
use opentelemetry::{KeyValue, global};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use swell_core::{
    opentelemetry::gen_ai,
    opentelemetry::pricing,
    opentelemetry::GenAiSpanExt,
    opentelemetry::LatencyTracker,
    LlmToolCall,
    SwellError,
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
        global::tracer("swell-llm")
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

        debug!(model = %self.model, "OpenAI API request");

        // Start OpenTelemetry span for the LLM call
        let tracer = self.tracer();
        let span_name = format!("OpenAI chat {}", self.model);
        let mut span = tracer.build(&span_name);
        span.set_attribute(KeyValue::new(gen_ai::OPERATION_NAME, "chat"));
        span.set_attribute(KeyValue::new(gen_ai::PROVIDER_NAME, "openai"));
        span.set_attribute(KeyValue::new(gen_ai::REQUEST_MODEL, self.model.clone()));
        span.set_attribute(KeyValue::new(
            "http.target",
            "/chat/completions",
        ));
        span.set_attribute(KeyValue::new("server.address", "api.openai.com"));

        // Track latency
        let latency = LatencyTracker::new();

        let url = format!("{}/chat/completions", self.base_url);
        let response = self
            .client
            .post(&url)
            .header("authorization", format!("Bearer {}", self.api_key))
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                span.record_error(&e.to_string());
                span.set_status(opentelemetry::trace::Status::error(e.to_string()));
                SwellError::LlmError(format!("Request failed: {}", e))
            })?;

        let latency_ms = latency.elapsed_ms();

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            warn!(status = %status, body = %body, "OpenAI API error");
            span.record_error(&format!("API error {}: {}", status, body));
            span.set_status(opentelemetry::trace::Status::error(body.clone()));
            return Err(SwellError::LlmError(format!(
                "API error {}: {}",
                status, body
            )));
        }

        let api_response: Response = response
            .json()
            .await
            .map_err(|e| {
                span.record_error(&e.to_string());
                span.set_status(opentelemetry::trace::Status::error(e.to_string()));
                SwellError::LlmError(format!("Failed to parse response: {}", e))
            })?;

        let choice = api_response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| {
                span.record_error("No choices in response");
                span.set_status(opentelemetry::trace::Status::error("No choices in response"));
                SwellError::LlmError("No choices in response".to_string())
            })?;

        let content = choice.message.content.unwrap_or_default();

        let tool_calls = choice.message.tool_calls.map(|calls| {
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
        span.record_prompt_tokens(input_tokens);
        span.record_completion_tokens(output_tokens);
        span.record_latency_ms(latency_ms);

        // OpenAI response includes the model used
        span.record_response_model(&self.model);

        // Calculate and record cost
        let pricing = pricing::for_model(&self.model);
        let cost = pricing.calculate_cost(input_tokens, output_tokens);
        span.record_cost_usd(cost);

        // End span successfully
        span.end();

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
