//! Stage 3b — provider-agnostic structured completion.
//!
//! Adding a provider means one module here plus one arm in `LlmClient`.

pub mod anthropic;
pub mod openai;

use crate::ai_tooling::config::{AiToolingConfig, ProviderKind};
use crate::ai_tooling::Result;
use reqwest::Client;
use serde_json::Value;

/// The completion a provider returned. `Refused` keeps a model refusal distinct
/// from a transport failure — one is an answer, the other is an error.
#[derive(Debug, Clone)]
pub enum Completion {
    Json(String),
    Refused,
}

pub enum LlmClient {
    Anthropic(anthropic::AnthropicClient),
    OpenAi(openai::OpenAiClient),
}

impl LlmClient {
    pub fn from_config(config: &AiToolingConfig, http: Client) -> Result<Self> {
        let api_key = config.provider_key()?.to_string();
        Ok(match config.provider {
            ProviderKind::Anthropic => Self::Anthropic(anthropic::AnthropicClient::new(
                http,
                api_key,
                config.anthropic_model.clone(),
            )),
            ProviderKind::OpenAi => Self::OpenAi(openai::OpenAiClient::new(
                http,
                api_key,
                config.openai_model.clone(),
                config.openai_base_url.clone(),
            )),
        })
    }

    pub fn kind(&self) -> ProviderKind {
        match self {
            Self::Anthropic(_) => ProviderKind::Anthropic,
            Self::OpenAi(_) => ProviderKind::OpenAi,
        }
    }

    /// Model id recorded alongside every generated blueprint.
    pub fn model_id(&self) -> &str {
        match self {
            Self::Anthropic(client) => client.model_id(),
            Self::OpenAi(client) => client.model_id(),
        }
    }

    /// Schema-constrained completion: raw JSON text in the success case.
    pub async fn complete(
        &self,
        system_prompt: &str,
        payload: &Value,
        schema: &Value,
    ) -> Result<Completion> {
        match self {
            Self::Anthropic(client) => client.complete(system_prompt, payload, schema).await,
            Self::OpenAi(client) => client.complete(system_prompt, payload, schema).await,
        }
    }
}

/// Shared cap; the rulebook is small and a runaway answer is never useful.
pub(crate) const MAX_TOKENS: u32 = 2048;
