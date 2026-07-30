//! Backend contract. The engine owns intent routing, safety constraints and
//! media math; a provider only speaks to a model API.

use crate::action_engine::types::{ChatTurn, EngineError};
use std::path::PathBuf;

/// Progress sink handed to long-running calls (e.g. video job polling).
pub type Progress<'a> = &'a dyn Fn(String);

pub struct ChatContext<'a> {
    pub prompt: &'a str,
    pub history: &'a [ChatTurn],
    pub has_clip: bool,
}

/// What the model decided to do with a chat turn.
pub enum ChatOutcome {
    Reply(String),
    GenerateImage { prompt: String },
    GenerateVideo { prompt: String, seconds: u32 },
    AnalyzeClip,
}

pub struct PacingStats {
    pub total_duration_seconds: f64,
    pub total_clips: usize,
}

pub trait ActionProvider: Send + 'static {
    fn chat(&self, context: ChatContext<'_>) -> Result<ChatOutcome, EngineError>;

    /// Returns raw encoded image bytes; the prompt already carries every
    /// constraint the engine requires.
    fn generate_image(&self, prompt: &str) -> Result<Vec<u8>, EngineError>;

    fn generate_video(
        &self,
        prompt: &str,
        seconds: u32,
        progress: Progress<'_>,
    ) -> Result<PathBuf, EngineError>;

    fn transcribe(&self, wav: Vec<u8>) -> Result<String, EngineError>;

    /// Raw JSON array of recommendations, per the analysis schema.
    fn analyze(&self, frames: &[Vec<u8>], transcript: &str) -> Result<String, EngineError>;

    /// Raw JSON strategy object.
    fn seo_strategy(&self, stats: PacingStats, transcript: &str) -> Result<String, EngineError>;
}
