//! Stateful chat: role-typed messages, a pruning context manager, and an async
//! client that speaks either provider dialect.

pub mod bridge;
pub mod client;
pub mod memory;
pub mod models;

pub use bridge::{ChatBridge, ChatEvent};
pub use client::{ChatClient, CompletionOptions};
pub use memory::{ChatSession, ContextBudget};
pub use models::{estimated_tokens, Message, Role};

use crate::ai_tooling::config::ProviderKind;
use crate::ai_tooling::AiToolingError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ChatError {
    #[error("configuration: {0}")]
    Config(#[from] AiToolingError),

    #[error("request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("{provider:?} returned {status}: {body}")]
    Api {
        provider: ProviderKind,
        status: u16,
        body: String,
    },

    #[error("malformed response: {0}")]
    Json(#[from] serde_json::Error),

    /// The model declined to answer — an outcome, not a transport failure.
    #[error("model refused: {0}")]
    Refused(String),

    #[error("model returned an empty completion")]
    EmptyCompletion,

    #[error("nothing to send: the conversation has no turns")]
    EmptyConversation,

    #[error("could not start the chat runtime: {0}")]
    Runtime(String),
}

pub type Result<T> = std::result::Result<T, ChatError>;
