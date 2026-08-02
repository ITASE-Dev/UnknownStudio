//! OpenAI Chat Completions with strict Structured Outputs.

use crate::ai_tooling::pipeline::schema::SchemaSpec;
use crate::ai_tooling::providers::{Completion, MAX_TOKENS};
use crate::ai_tooling::{AiToolingError, Result};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};

/// Schema name used by the original blueprint call, kept so that request is
/// byte-for-byte what it always was.
pub(crate) const SCHEMA_NAME: &str = "editing_blueprint";

pub struct OpenAiClient {
    http: Client,
    api_key: String,
    model: String,
    /// Full base URL, e.g. `https://api.openai.com/v1`. Always explicit so an
    /// empty `.env` value can never become a broken request URL.
    base_url: String,
}

impl OpenAiClient {
    pub fn new(http: Client, api_key: String, model: String, base_url: String) -> Self {
        Self {
            http,
            api_key,
            model,
            base_url,
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
        self.complete_with(system_prompt, payload, &SchemaSpec::new(SCHEMA_NAME, schema.clone()))
            .await
    }

    /// Structured completion against a named schema.
    ///
    /// The pipeline needs one name per agent — the API echoes it back, and a
    /// single shared name makes three different contracts indistinguishable in
    /// a log.
    pub async fn complete_with(
        &self,
        system_prompt: &str,
        payload: &Value,
        spec: &SchemaSpec,
    ) -> Result<Completion> {
        let body = json!({
            "model": self.model,
            "max_tokens": MAX_TOKENS,
            "response_format": {
                "type": "json_schema",
                "json_schema": { "name": spec.name, "strict": true, "schema": spec.schema },
            },
            "messages": [
                { "role": "system", "content": system_prompt },
                { "role": "user", "content": payload.to_string() },
            ],
        });

        let endpoint = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let response = self
            .http
            .post(endpoint)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            return Err(AiToolingError::Api {
                service: "openai",
                status: status.as_u16(),
                body: response.text().await.unwrap_or_default(),
            });
        }

        let completion: ChatResponse = response.json().await?;
        let Some(choice) = completion.choices.into_iter().next() else {
            return Ok(Completion::Refused);
        };
        if choice.message.refusal.is_some() {
            return Ok(Completion::Refused);
        }

        Ok(choice
            .message
            .content
            .filter(|text| !text.trim().is_empty())
            .map(Completion::Json)
            .unwrap_or(Completion::Refused))
    }
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    #[serde(default)]
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ChatMessage,
}

#[derive(Debug, Deserialize)]
struct ChatMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    refusal: Option<String>,
}
