//! Async chat completion. One client speaks both provider dialects: OpenAI
//! takes the system prompt as a message, Anthropic takes it as a field.

use crate::ai_tooling::chat::models::{Message, Role};
use crate::ai_tooling::chat::{ChatError, ChatSession, Result};
use crate::ai_tooling::config::{AiToolingConfig, ProviderKind};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(90);
const ANTHROPIC_ENDPOINT: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Per-call generation settings.
#[derive(Debug, Clone, Copy)]
pub struct CompletionOptions {
    pub max_tokens: u32,
    pub temperature: f32,
}

impl Default for CompletionOptions {
    fn default() -> Self {
        Self {
            max_tokens: 1024,
            temperature: 0.7,
        }
    }
}

pub struct ChatClient {
    http: Client,
    provider: ProviderKind,
    api_key: String,
    model: String,
    /// OpenAI only; always explicit so a blank `.env` value cannot produce a
    /// malformed request URL.
    base_url: String,
    options: CompletionOptions,
}

impl ChatClient {
    /// Builds a client from `.env` through the shared config loader.
    pub fn from_env() -> Result<Self> {
        Self::new(&AiToolingConfig::load()?)
    }

    pub fn new(config: &AiToolingConfig) -> Result<Self> {
        Ok(Self {
            http: Client::builder().timeout(REQUEST_TIMEOUT).build()?,
            provider: config.provider,
            api_key: config.provider_key()?.to_string(),
            model: config.model_id().to_string(),
            base_url: config.openai_base_url.clone(),
            options: CompletionOptions::default(),
        })
    }

    pub fn with_options(mut self, options: CompletionOptions) -> Self {
        self.options = options;
        self
    }

    pub fn provider(&self) -> ProviderKind {
        self.provider
    }

    pub fn model_id(&self) -> &str {
        &self.model
    }

    /// Sends the session's context and returns the assistant's reply. The
    /// session is left untouched — the caller decides whether to keep the turn.
    pub async fn complete(&self, session: &ChatSession) -> Result<Message> {
        if session.is_empty() {
            return Err(ChatError::EmptyConversation);
        }
        self.complete_messages(&session.context()).await
    }

    /// Same, from a plain message list — the shape that crosses a thread
    /// boundary when the UI hands work to a worker.
    pub async fn complete_messages(&self, messages: &[Message]) -> Result<Message> {
        let (system, history) = split_system(messages);
        if history.is_empty() {
            return Err(ChatError::EmptyConversation);
        }

        let body = match self.provider {
            ProviderKind::Anthropic => self.anthropic_body(system, &history),
            ProviderKind::OpenAi => self.openai_body(system, &history),
        };

        let request = match self.provider {
            ProviderKind::Anthropic => self
                .http
                .post(ANTHROPIC_ENDPOINT)
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", ANTHROPIC_VERSION),
            ProviderKind::OpenAi => self
                .http
                .post(format!(
                    "{}/chat/completions",
                    self.base_url.trim_end_matches('/')
                ))
                .bearer_auth(&self.api_key),
        };

        let response = request.json(&body).send().await?;
        let status = response.status();
        if !status.is_success() {
            return Err(ChatError::Api {
                provider: self.provider,
                status: status.as_u16(),
                body: response.text().await.unwrap_or_default(),
            });
        }

        let text = match self.provider {
            ProviderKind::Anthropic => parse_anthropic(&response.text().await?)?,
            ProviderKind::OpenAi => parse_openai(&response.text().await?)?,
        };
        Ok(Message::assistant(text))
    }

    /// Appends the user's turn, completes, and records the reply.
    pub async fn send(&self, session: &mut ChatSession, prompt: impl Into<String>) -> Result<Message> {
        session.push_user(prompt);
        let reply = self.complete(session).await?;
        session.push(reply.clone());
        Ok(reply)
    }

    fn openai_body(&self, system: Option<&Message>, history: &[&Message]) -> Value {
        let messages: Vec<Value> = system
            .into_iter()
            .chain(history.iter().copied())
            .map(|message| json!({ "role": message.role.as_str(), "content": message.content }))
            .collect();

        json!({
            "model": self.model,
            "messages": messages,
            "max_tokens": self.options.max_tokens,
            "temperature": self.options.temperature,
        })
    }

    fn anthropic_body(&self, system: Option<&Message>, history: &[&Message]) -> Value {
        let messages: Vec<Value> = history
            .iter()
            .map(|message| json!({ "role": message.role.as_str(), "content": message.content }))
            .collect();

        let mut body = json!({
            "model": self.model,
            "messages": messages,
            "max_tokens": self.options.max_tokens,
            "temperature": self.options.temperature,
        });
        if let Some(system) = system {
            body["system"] = json!(system.content);
        }
        body
    }
}

/// Splits the pinned instructions from the turns. The last system message wins,
/// mirroring `ChatSession`, which keeps only one.
fn split_system(messages: &[Message]) -> (Option<&Message>, Vec<&Message>) {
    let system = messages.iter().rev().find(|m| m.role == Role::System);
    let history = messages.iter().filter(|m| m.role != Role::System).collect();
    (system, history)
}

fn parse_openai(raw: &str) -> Result<String> {
    let response: OpenAiResponse = serde_json::from_str(raw)?;
    let Some(choice) = response.choices.into_iter().next() else {
        return Err(ChatError::EmptyCompletion);
    };
    if let Some(refusal) = choice.message.refusal {
        return Err(ChatError::Refused(refusal));
    }
    choice
        .message
        .content
        .filter(|text| !text.trim().is_empty())
        .ok_or(ChatError::EmptyCompletion)
}

fn parse_anthropic(raw: &str) -> Result<String> {
    let response: AnthropicResponse = serde_json::from_str(raw)?;
    if response.stop_reason.as_deref() == Some("refusal") {
        return Err(ChatError::Refused("model refused the request".into()));
    }
    response
        .content
        .into_iter()
        .find_map(|block| (block.kind == "text").then_some(block.text?))
        .filter(|text| !text.trim().is_empty())
        .ok_or(ChatError::EmptyCompletion)
}

// -- Wire format ---------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct OpenAiResponse {
    #[serde(default)]
    choices: Vec<OpenAiChoice>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: OpenAiMessage,
}

#[derive(Debug, Deserialize)]
struct OpenAiMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    refusal: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    #[serde(default)]
    content: Vec<AnthropicBlock>,
    #[serde(default)]
    stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnthropicBlock {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(provider: ProviderKind) -> ChatClient {
        ChatClient {
            http: Client::new(),
            provider,
            api_key: "test-key".into(),
            model: "test-model".into(),
            base_url: "https://api.openai.com/v1".into(),
            options: CompletionOptions::default(),
        }
    }

    fn session() -> ChatSession {
        let mut session = ChatSession::with_system("be brief");
        session.push_user("hello");
        session.push_assistant("hi");
        session.push_user("again");
        session
    }

    #[test]
    fn openai_carries_the_system_prompt_as_a_message() {
        let context = session().context();
        let (system, history) = split_system(&context);
        let body = client(ProviderKind::OpenAi).openai_body(system, &history);
        let messages = body["messages"].as_array().expect("messages");

        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[3]["content"], "again");
        assert_eq!(body["model"], "test-model");
    }

    #[test]
    fn anthropic_lifts_the_system_prompt_out_of_the_messages() {
        let context = session().context();
        let (system, history) = split_system(&context);
        let body = client(ProviderKind::Anthropic).anthropic_body(system, &history);
        let messages = body["messages"].as_array().expect("messages");

        assert_eq!(body["system"], "be brief");
        assert_eq!(messages.len(), 3);
        assert!(messages.iter().all(|m| m["role"] != "system"));
    }

    #[test]
    fn openai_replies_are_extracted_and_refusals_surfaced() {
        let text = parse_openai(r#"{"choices":[{"message":{"content":"answer"}}]}"#).expect("reply");
        assert_eq!(text, "answer");

        let refusal = parse_openai(r#"{"choices":[{"message":{"refusal":"no"}}]}"#);
        assert!(matches!(refusal, Err(ChatError::Refused(_))));

        assert!(matches!(
            parse_openai(r#"{"choices":[]}"#),
            Err(ChatError::EmptyCompletion)
        ));
    }

    #[test]
    fn anthropic_replies_skip_non_text_blocks() {
        let raw = r#"{"content":[{"type":"thinking","text":"…"},{"type":"text","text":"answer"}]}"#;
        assert_eq!(parse_anthropic(raw).expect("reply"), "answer");

        let refused = parse_anthropic(r#"{"stop_reason":"refusal","content":[]}"#);
        assert!(matches!(refused, Err(ChatError::Refused(_))));

        assert!(matches!(
            parse_anthropic(r#"{"content":[{"type":"text","text":"  "}]}"#),
            Err(ChatError::EmptyCompletion)
        ));
    }
}
