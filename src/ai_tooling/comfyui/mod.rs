//! ComfyUI integration: queue a workflow over HTTP, watch it over a WebSocket.
//!
//! The HTTP side triggers work and fetches results; the socket side reports
//! progress. Both are async and neither ever touches the UI thread.

pub mod http_client;
pub mod models;
pub mod ws_listener;

pub use http_client::{set_node_input, ComfyUiClient};
pub use models::{
    parse_event, ComfyEvent, HistoryEntry, OutputFile, PromptResponse, Workflow,
};
pub use ws_listener::{JobProgress, ProgressListener};

use thiserror::Error;

/// Default ComfyUI address; overridden by `COMFYUI_URL` in `.env`.
pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:8188";

#[derive(Debug, Error)]
pub enum ComfyUiError {
    #[error("network: {0}")]
    Network(#[from] reqwest::Error),

    #[error("ComfyUI returned {status}: {body}")]
    Api { status: u16, body: String },

    /// The graph was refused before it ran — a missing model or a bad link.
    #[error("workflow rejected: {detail}")]
    Rejected { detail: String },

    /// The graph could not be prepared locally.
    #[error("workflow: {detail}")]
    Workflow { detail: String },

    #[error("could not parse response: {0}")]
    Parse(#[from] serde_json::Error),

    #[error("websocket: {0}")]
    WsDisconnect(String),

    #[error("prompt {prompt_id} did not finish within {seconds}s")]
    Timeout { prompt_id: String, seconds: u64 },

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, ComfyUiError>;

/// Server address from the environment, falling back to the local default.
pub fn base_url_from_env() -> String {
    crate::ai_tooling::config::load_dotenv();
    std::env::var("COMFYUI_URL")
        .ok()
        .map(|url| url.trim().to_string())
        .filter(|url| !url.is_empty())
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
}

/// Client id shared by the HTTP client and the socket, so progress for this
/// app's prompts can be told apart from another client's.
pub fn new_client_id() -> String {
    format!("unknown-studio-{}", uuid::Uuid::new_v4().simple())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_ids_are_unique_and_identifiable() {
        let first = new_client_id();
        assert!(first.starts_with("unknown-studio-"));
        assert_ne!(first, new_client_id());
    }

    #[test]
    fn the_default_address_is_the_local_server() {
        // Unset in most environments; the fallback must be usable as-is.
        if std::env::var("COMFYUI_URL").is_err() {
            assert_eq!(base_url_from_env(), DEFAULT_BASE_URL);
        }
    }
}
