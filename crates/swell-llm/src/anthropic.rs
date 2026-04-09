//! Anthropic Claude API backend.

use crate::{LlmBackend, LlmConfig, LlmMessage, LlmResponse, LlmRole, LlmToolDefinition, LlmUsage};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use swell_core::{LlmToolCall, SwellError};
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

        debug!(model = %self.model, "Anthropic API request");

        let response = self
            .client
            .post(ANTHROPIC_API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| SwellError::LlmError(format!("Request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            warn!(status = %status, body = %body, "Anthropic API error");
            return Err(SwellError::LlmError(format!(
                "API error {}: {}",
                status, body
            )));
        }

        let api_response: Response = response
            .json()
            .await
            .map_err(|e| SwellError::LlmError(format!("Failed to parse response: {}", e)))?;

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

        Ok(LlmResponse {
            content,
            tool_calls: if tool_calls.is_empty() {
                None
            } else {
                Some(tool_calls)
            },
            usage: LlmUsage {
                input_tokens: api_response.usage.input_tokens,
                output_tokens: api_response.usage.output_tokens,
                total_tokens: api_response.usage.input_tokens + api_response.usage.output_tokens,
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

    #[tokio::test]
    async fn test_backend_model_name() {
        let backend = AnthropicBackend::new("claude-opus-4-5", "fake-key");
        assert_eq!(backend.model(), "claude-opus-4-5");
    }

    #[tokio::test]
    async fn test_role_conversion() {
        assert_eq!(AnthropicBackend::convert_role(LlmRole::System), "system");
        assert_eq!(AnthropicBackend::convert_role(LlmRole::User), "user");
        assert_eq!(
            AnthropicBackend::convert_role(LlmRole::Assistant),
            "assistant"
        );
    }
}
