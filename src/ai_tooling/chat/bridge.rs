//! Thread bridge between the immediate-mode UI and the async client.
//!
//! The UI thread never awaits: it hands a context snapshot to a worker task and
//! polls for the reply on later frames.

use crate::ai_tooling::chat::models::Message;
use crate::ai_tooling::chat::{ChatClient, ChatError, Result};
use crate::ai_tooling::config::AiToolingConfig;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use tokio::runtime::Runtime;
use tokio::sync::mpsc as async_mpsc;

/// What came back for a submitted turn.
#[derive(Debug, Clone)]
pub enum ChatEvent {
    Reply(Message),
    /// Rendered failure text; the UI shows it as a notice, not a chat turn.
    Failed(String),
}

pub struct ChatBridge {
    /// Kept alive for as long as the bridge: dropping it stops the worker.
    _runtime: Runtime,
    requests: async_mpsc::UnboundedSender<Vec<Message>>,
    events: Receiver<ChatEvent>,
}

impl ChatBridge {
    /// Builds the client from `.env` and starts the worker.
    pub fn from_env() -> Result<Self> {
        Self::new(ChatClient::from_env()?)
    }

    pub fn from_config(config: &AiToolingConfig) -> Result<Self> {
        Self::new(ChatClient::new(config)?)
    }

    pub fn new(client: ChatClient) -> Result<Self> {
        let runtime = Runtime::new().map_err(|err| ChatError::Runtime(err.to_string()))?;
        let (requests, mut inbox) = async_mpsc::unbounded_channel::<Vec<Message>>();
        let (events_tx, events): (Sender<ChatEvent>, Receiver<ChatEvent>) = mpsc::channel();

        // One request at a time: replies must stay ordered with the turns the
        // user sees, and a queue of parallel completions would interleave them.
        runtime.spawn(async move {
            while let Some(context) = inbox.recv().await {
                let event = match client.complete_messages(&context).await {
                    Ok(reply) => ChatEvent::Reply(reply),
                    Err(err) => ChatEvent::Failed(err.to_string()),
                };
                if events_tx.send(event).is_err() {
                    return;
                }
            }
        });

        Ok(Self {
            _runtime: runtime,
            requests,
            events,
        })
    }

    /// Queues a completion. Returns `false` once the worker is gone.
    pub fn request(&self, context: Vec<Message>) -> bool {
        self.requests.send(context).is_ok()
    }

    /// Non-blocking drain, safe to call every frame.
    pub fn poll(&self) -> impl Iterator<Item = ChatEvent> + '_ {
        std::iter::from_fn(move || match self.events.try_recv() {
            Ok(event) => Some(event),
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => None,
        })
    }
}
