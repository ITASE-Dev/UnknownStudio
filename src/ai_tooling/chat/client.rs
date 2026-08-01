//! Async chat completion. One client speaks both provider dialects: OpenAI
//! takes the system prompt as a message, Anthropic takes it as a field.

use crate::ai_tooling::chat::models::{Message, Role};
use crate::ai_tooling::chat::{ChatError, ChatSession, Result};
use crate::ai_tooling::config::{AiToolingConfig, ProviderKind};
use crate::ai_tooling::orchestration::dispatcher::ActionCommand;
use crate::ai_tooling::orchestration::tools::{anthropic_tools, get_available_tools, parse_tool_call};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(90);
const ANTHROPIC_ENDPOINT: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// A completion: prose, tool calls, or both. Providers may answer with tool
/// calls and no text, so the message is optional.
#[derive(Debug, Clone, Default)]
pub struct ChatResponse {
    pub message: Option<Message>,
    /// Tool calls, already parsed into executable commands.
    pub actions: Vec<ActionCommand>,
    /// Tool calls that could not be parsed, kept so the UI can say why.
    pub rejected: Vec<String>,
}

impl ChatResponse {
    pub fn is_empty(&self) -> bool {
        self.message.is_none() && self.actions.is_empty()
    }
}

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
    /// Whether the editing tools are offered. Off makes the model answer in
    /// prose only — useful for a read-only conversation.
    tools_enabled: bool,
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
            tools_enabled: true,
        })
    }

    pub fn with_options(mut self, options: CompletionOptions) -> Self {
        self.options = options;
        self
    }

    pub fn with_tools(mut self, enabled: bool) -> Self {
        self.tools_enabled = enabled;
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
    pub async fn complete(&self, session: &ChatSession) -> Result<ChatResponse> {
        if session.is_empty() {
            return Err(ChatError::EmptyConversation);
        }
        self.complete_messages(&session.context()).await
    }

    /// Same, from a plain message list — the shape that crosses a thread
    /// boundary when the UI hands work to a worker.
    pub async fn complete_messages(&self, messages: &[Message]) -> Result<ChatResponse> {
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

        let body = response.text().await?;
        let parsed = match self.provider {
            ProviderKind::Anthropic => parse_anthropic(&body)?,
            ProviderKind::OpenAi => parse_openai(&body)?,
        };

        // Tool calls count as an answer; only a wholly empty turn is an error.
        if parsed.is_empty() {
            return Err(ChatError::EmptyCompletion);
        }
        Ok(parsed)
    }

    /// Appends the user's turn, completes, and records any prose reply.
    pub async fn send(
        &self,
        session: &mut ChatSession,
        prompt: impl Into<String>,
    ) -> Result<ChatResponse> {
        session.push_user(prompt);
        let response = self.complete(session).await?;
        if let Some(message) = &response.message {
            session.push(message.clone());
        }
        Ok(response)
    }

    fn openai_body(&self, system: Option<&Message>, history: &[&Message]) -> Value {
        let messages: Vec<Value> = system
            .into_iter()
            .chain(history.iter().copied())
            .map(|message| json!({ "role": message.role.as_str(), "content": message.content }))
            .collect();

        let mut body = json!({
            "model": self.model,
            "messages": messages,
            "max_tokens": self.options.max_tokens,
            "temperature": self.options.temperature,
        });
        if self.tools_enabled {
            body["tools"] = get_available_tools();
            // `auto` and not `required`: a question deserves prose, not a call.
            body["tool_choice"] = json!("auto");
        }
        body
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
        if self.tools_enabled {
            body["tools"] = anthropic_tools();
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

fn parse_openai(raw: &str) -> Result<ChatResponse> {
    let response: OpenAiResponse = serde_json::from_str(raw)?;
    let Some(choice) = response.choices.into_iter().next() else {
        return Ok(ChatResponse::default());
    };
    if let Some(refusal) = choice.message.refusal {
        return Err(ChatError::Refused(refusal));
    }

    let mut parsed = ChatResponse {
        message: choice
            .message
            .content
            .filter(|text| !text.trim().is_empty())
            .map(Message::assistant),
        ..ChatResponse::default()
    };

    for call in choice.message.tool_calls {
        collect(&mut parsed, &call.function.name, &call.function.arguments);
    }
    Ok(parsed)
}

fn parse_anthropic(raw: &str) -> Result<ChatResponse> {
    let response: AnthropicResponse = serde_json::from_str(raw)?;
    if response.stop_reason.as_deref() == Some("refusal") {
        return Err(ChatError::Refused("model refused the request".into()));
    }

    let mut parsed = ChatResponse::default();
    for block in response.content {
        match block.kind.as_str() {
            "text" => {
                let text = block.text.unwrap_or_default();
                if !text.trim().is_empty() {
                    parsed.message = Some(Message::assistant(text));
                }
            }
            "tool_use" => {
                let name = block.name.unwrap_or_default();
                collect(&mut parsed, &name, &block.input.unwrap_or(Value::Null));
            }
            _ => {}
        }
    }
    Ok(parsed)
}

/// A malformed call is recorded rather than dropped: the user should see that
/// the model tried to act, and why it did not land.
fn collect(response: &mut ChatResponse, name: &str, arguments: &Value) {
    match parse_tool_call(name, arguments) {
        Ok(command) => response.actions.push(command),
        Err(error) => response.rejected.push(error.to_string()),
    }
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
    #[serde(default)]
    tool_calls: Vec<OpenAiToolCall>,
}

#[derive(Debug, Deserialize)]
struct OpenAiToolCall {
    function: OpenAiFunctionCall,
}

#[derive(Debug, Deserialize)]
struct OpenAiFunctionCall {
    name: String,
    /// OpenAI sends the arguments as a JSON string.
    #[serde(default)]
    arguments: Value,
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
    /// `tool_use` blocks only.
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    input: Option<Value>,
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
            tools_enabled: true,
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
        let reply = parse_openai(r#"{"choices":[{"message":{"content":"answer"}}]}"#).expect("reply");
        assert_eq!(reply.message.expect("message").content, "answer");

        let refusal = parse_openai(r#"{"choices":[{"message":{"refusal":"no"}}]}"#);
        assert!(matches!(refusal, Err(ChatError::Refused(_))));

        assert!(parse_openai(r#"{"choices":[]}"#).expect("empty").is_empty());
    }

    #[test]
    fn anthropic_replies_skip_non_text_blocks() {
        let raw = r#"{"content":[{"type":"thinking","text":"…"},{"type":"text","text":"answer"}]}"#;
        let reply = parse_anthropic(raw).expect("reply");
        assert_eq!(reply.message.expect("message").content, "answer");

        let refused = parse_anthropic(r#"{"stop_reason":"refusal","content":[]}"#);
        assert!(matches!(refused, Err(ChatError::Refused(_))));

        assert!(parse_anthropic(r#"{"content":[{"type":"text","text":"  "}]}"#)
            .expect("blank")
            .is_empty());
    }

    #[test]
    fn both_payloads_offer_the_editing_tools() {
        let context = session().context();
        let (system, history) = split_system(&context);

        let openai = client(ProviderKind::OpenAi).openai_body(system, &history);
        assert_eq!(openai["tool_choice"], "auto");
        assert_eq!(openai["tools"][0]["function"]["name"], "add_marker");

        let anthropic = client(ProviderKind::Anthropic).anthropic_body(system, &history);
        assert_eq!(anthropic["tools"][0]["name"], "add_marker");

        // A read-only client offers none.
        let quiet = client(ProviderKind::OpenAi).with_tools(false);
        assert!(quiet.openai_body(system, &history).get("tools").is_none());
    }

    #[test]
    fn openai_tool_calls_become_commands() {
        let raw = r#"{"choices":[{"message":{"content":null,"tool_calls":[
            {"function":{"name":"add_marker","arguments":"{\"time_sec\":4.5,\"color\":\"red\",\"label\":\"hook\"}"}},
            {"function":{"name":"delete_clip","arguments":"{\"clip_id\":\"c9\"}"}}
        ]}}]}"#;

        let reply = parse_openai(raw).expect("reply");
        assert!(reply.message.is_none(), "a tool call needs no prose");
        assert_eq!(reply.actions.len(), 2);
        assert_eq!(
            reply.actions[0],
            ActionCommand::AddMarker {
                time_sec: 4.5,
                color: "red".into(),
                label: "hook".into()
            }
        );
        assert!(!reply.is_empty(), "tool calls are an answer");
    }

    #[test]
    fn anthropic_tool_use_blocks_become_commands() {
        let raw = r#"{"content":[
            {"type":"text","text":"Cutting there."},
            {"type":"tool_use","name":"split_clip","input":{"clip_id":"c3","time_sec":12}}
        ]}"#;

        let reply = parse_anthropic(raw).expect("reply");
        assert_eq!(reply.message.expect("prose").content, "Cutting there.");
        assert_eq!(
            reply.actions,
            vec![ActionCommand::SplitClip {
                clip_id: "c3".into(),
                time_sec: 12.0
            }]
        );
    }

    #[test]
    fn an_unparseable_call_is_reported_not_dropped() {
        let raw = r#"{"choices":[{"message":{"tool_calls":[
            {"function":{"name":"split_clip","arguments":"{\"clip_id\":\"c1\"}"}}
        ]}}]}"#;

        let reply = parse_openai(raw).expect("reply");
        assert!(reply.actions.is_empty());
        assert_eq!(reply.rejected.len(), 1);
        assert!(reply.rejected[0].contains("time_sec"));
    }
}
