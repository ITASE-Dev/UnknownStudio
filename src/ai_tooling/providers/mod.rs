//! Stage 3b — provider-agnostic structured completion.
//!
//! Adding a provider means one module here plus one arm in `LlmClient`.

pub mod anthropic;
pub mod openai;

use crate::ai_tooling::config::{AiToolingConfig, ProviderKind};
use crate::ai_tooling::pipeline::schema::SchemaSpec;
use crate::ai_tooling::{AiToolingError, Result};
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;

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

    /// Structured completion against a named schema, retried through transient
    /// failures. This is what the agent pipeline calls.
    pub async fn complete_spec(
        &self,
        system_prompt: &str,
        payload: &Value,
        spec: &SchemaSpec,
        retry: RetryPolicy,
    ) -> Result<Completion> {
        let mut attempt = 0;
        loop {
            let result = match self {
                Self::Anthropic(client) => {
                    client.complete_with(system_prompt, payload, spec).await
                }
                Self::OpenAi(client) => client.complete_with(system_prompt, payload, spec).await,
            };

            match result {
                Err(err) if retry.should_retry(&err, attempt) => {
                    tokio::time::sleep(retry.backoff(attempt)).await;
                    attempt += 1;
                }
                other => return other,
            }
        }
    }
}

/// How hard to try again when the provider says "not now".
///
/// Only transient conditions are retried. A 400 means the schema is wrong and
/// will be wrong next time too; retrying it burns quota to reach the same
/// error slower.
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    pub max_retries: u32,
    pub base_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay: Duration::from_millis(700),
        }
    }
}

impl RetryPolicy {
    /// No retries — for tests, and for callers that own their own loop.
    pub fn none() -> Self {
        Self {
            max_retries: 0,
            base_delay: Duration::ZERO,
        }
    }

    pub fn should_retry(&self, error: &AiToolingError, attempt: u32) -> bool {
        attempt < self.max_retries && is_transient(error)
    }

    /// Exponential backoff: 700ms, 1.4s, 2.8s.
    pub fn backoff(&self, attempt: u32) -> Duration {
        self.base_delay * 2_u32.pow(attempt.min(6))
    }
}

/// Rate limits, server faults and dropped connections are worth another go.
fn is_transient(error: &AiToolingError) -> bool {
    match error {
        AiToolingError::Api { status, .. } => *status == 429 || (500..600).contains(status),
        AiToolingError::Http(err) => err.is_timeout() || err.is_connect(),
        _ => false,
    }
}

/// Shared cap; the rulebook is small and a runaway answer is never useful.
pub(crate) const MAX_TOKENS: u32 = 2048;
