//! Anthropic Messages API with structured outputs.

use crate::ai_tooling::providers::{Completion, MAX_TOKENS};
use crate::ai_tooling::{AiToolingError, Result};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};

const ENDPOINT: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";

pub struct AnthropicClient {
    http: Client,
    api_key: String,
    model: String,
}

impl AnthropicClient {
    pub fn new(http: Client, api_key: String, model: String) -> Self {
        Self {
            http,
            api_key,
            model,
        }
    }

    pub fn model_id(&self) -> &str {
        &self.model
    }

    pub async fn complete(
        &self,
        system_prompt: &str,
        payload: &Value,
        schema: &Value,
    ) -> Result<Completion> {
        let body = json!({
            "model": self.model,
            "max_tokens": MAX_TOKENS,
            "system": system_prompt,
            "output_config": { "format": { "type": "json_schema", "schema": schema } },
            "messages": [{ "role": "user", "content": payload.to_string() }],
        });

        let response = self
            .http
            .post(ENDPOINT)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            return Err(AiToolingError::Api {
                service: "anthropic",
                status: status.as_u16(),
                body: response.text().await.unwrap_or_default(),
            });
        }

        let message: MessageResponse = response.json().await?;
        if message.stop_reason.as_deref() == Some("refusal") {
            return Ok(Completion::Refused);
        }

        Ok(message
            .content
            .into_iter()
            .find_map(|block| (block.kind == "text").then_some(block.text?))
            .map(Completion::Json)
            .unwrap_or(Completion::Refused))
    }
}

#[derive(Debug, Deserialize)]
struct MessageResponse {
    #[serde(default)]
    content: Vec<ContentBlock>,
    #[serde(default)]
    stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: Option<String>,
}
