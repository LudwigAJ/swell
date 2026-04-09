//! OpenAI API backend (also handles Azure OpenAI).

use crate::{LlmMessage, LlmRole, LlmResponse, LlmConfig, LlmToolDefinition, LlmUsage, LlmBackend};
use swell_core::{SwellError, LlmToolCall};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
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

    pub fn with_base_url(model: impl Into<String>, api_key: impl Into<String>, base_url: impl Into<String>) -> Result<Self, SwellError> {
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

        let url = format!("{}/chat/completions", self.base_url);
        let response = self
            .client
            .post(&url)
            .header("authorization", format!("Bearer {}", self.api_key))
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| SwellError::LlmError(format!("Request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            warn!(status = %status, body = %body, "OpenAI API error");
            return Err(SwellError::LlmError(format!(
                "API error {}: {}", status, body
            )));
        }

        let api_response: Response = response
            .json()
            .await
            .map_err(|e| SwellError::LlmError(format!("Failed to parse response: {}", e)))?;

        let choice = api_response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| SwellError::LlmError("No choices in response".to_string()))?;

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

        Ok(LlmResponse {
            content,
            tool_calls,
            usage: LlmUsage {
                input_tokens: api_response.usage.prompt_tokens,
                output_tokens: api_response.usage.completion_tokens,
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

    #[tokio::test]
    async fn test_backend_model_name() {
        let backend = OpenAIBackend::new("gpt-4-turbo", "fake-key").unwrap();
        assert_eq!(backend.model(), "gpt-4-turbo");
    }

    #[tokio::test]
    async fn test_role_conversion() {
        assert_eq!(OpenAIBackend::convert_role(LlmRole::System), "system");
        assert_eq!(OpenAIBackend::convert_role(LlmRole::User), "user");
        assert_eq!(OpenAIBackend::convert_role(LlmRole::Assistant), "assistant");
    }
}
