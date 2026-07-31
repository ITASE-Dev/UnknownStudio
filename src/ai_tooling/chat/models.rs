//! Conversation primitives: roles, messages, and the token estimate the
//! context manager prunes against.

use serde::{Deserialize, Serialize};

/// Characters per token. Deliberately conservative — over-estimating costs a
/// little context, under-estimating costs a rejected request.
const CHARS_PER_TOKEN: usize = 4;

/// Per-message overhead the APIs add for role and framing.
const MESSAGE_OVERHEAD_TOKENS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Standing instructions. Never pruned.
    System,
    User,
    Assistant,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self::new(Role::System, content)
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::new(Role::User, content)
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self::new(Role::Assistant, content)
    }

    pub fn new(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
        }
    }

    /// Rough token cost. An estimate is enough: it only decides when to prune,
    /// and the provider is the authority on the real count.
    pub fn estimated_tokens(&self) -> usize {
        self.content.chars().count().div_ceil(CHARS_PER_TOKEN) + MESSAGE_OVERHEAD_TOKENS
    }
}

/// Total estimated cost of a message sequence.
pub fn estimated_tokens(messages: &[Message]) -> usize {
    messages.iter().map(Message::estimated_tokens).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_serialize_as_the_api_spells_them() {
        let json = serde_json::to_string(&Message::user("hi")).expect("serialize");
        assert!(json.contains(r#""role":"user""#));
        assert_eq!(Role::Assistant.as_str(), "assistant");
    }

    #[test]
    fn estimates_scale_with_length_and_include_overhead() {
        let short = Message::user("x");
        let long = Message::user(&"x".repeat(400));
        assert_eq!(short.estimated_tokens(), 1 + MESSAGE_OVERHEAD_TOKENS);
        assert_eq!(long.estimated_tokens(), 100 + MESSAGE_OVERHEAD_TOKENS);
        assert_eq!(
            estimated_tokens(&[short, long]),
            101 + MESSAGE_OVERHEAD_TOKENS * 2
        );
    }
}
