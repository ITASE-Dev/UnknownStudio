//! AI tooling for the studio.
//!
//! - `chat` — stateful assistant conversations with context management.
//! - `orchestration` and friends — ingest a channel's uploads, find statistical
//!   outliers, scrape the highest-replayed moment, and reverse-engineer the
//!   editing rulebook that produced it.

pub mod audio_analysis;
pub mod chat;
pub mod comfyui;
pub mod competitor;
pub mod config;
pub mod ingestion;
pub mod orchestration;
pub mod pipeline;
pub mod prompting;
pub mod providers;
pub mod revision;
pub mod scraping;
pub mod visual_analysis;
pub mod youtube_insights;

pub use audio_analysis::{analyze_audio, AudioAnalysisError, TranscriptOutput};
pub use chat::{ChatBridge, ChatClient, ChatError, ChatEvent, ChatSession, ContextBudget, Message, Role};
pub use comfyui::{ComfyEvent, ComfyUiClient, ComfyUiError, JobProgress, ProgressListener};
pub use competitor::{CompetitorDataStore, CompetitorVideo, InMemoryWarehouse, SemanticIndex};
pub use revision::{
    ComparisonEngine, CurrentTimelineState, DiffSettings, RevisionAction, RevisionPlan,
    RevisionTask, TaskStatus,
};
pub use config::{AiToolingConfig, ProviderKind};
pub use ingestion::{ChannelIngest, VideoRecord, YouTubeClient};
pub use orchestration::{Blueprint, BlueprintEngine, PipelineReport, StageFailure};
pub use pipeline::{CompetitorDNA, LlmPipelineEngine, PipelineError, PipelineOutput};
pub use providers::LlmClient;
pub use scraping::{DeepScraper, PeakAnalysis};
pub use visual_analysis::{analyze_motion, MotionSpike, VisualTimeline};
pub use youtube_insights::{InsightsAggregator, InsightsError, OutlierAnalysis, PacingHeatmap, ViralBlueprint};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AiToolingError {
    #[error("missing environment variable: {0}")]
    MissingEnv(&'static str),

    #[error("invalid environment variable {key}: {reason}")]
    InvalidEnv { key: &'static str, reason: String },

    #[error("unsupported LLM provider: {0}")]
    UnsupportedProvider(String),

    #[error("request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("{service} returned {status}: {body}")]
    Api {
        service: &'static str,
        status: u16,
        body: String,
    },

    #[error("malformed JSON: {0}")]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("yt-dlp: {0}")]
    YtDlp(String),

    /// A per-video failure worth recording without aborting the run.
    #[error("scrape {video_id}: {reason}")]
    Scrape { video_id: String, reason: String },

    #[error("blueprint {video_id}: {reason}")]
    Blueprint { video_id: String, reason: String },

    #[error("visual analysis: {0}")]
    Vision(String),
}

pub type Result<T> = std::result::Result<T, AiToolingError>;

impl AiToolingError {
    pub(crate) fn scrape(video_id: &str, reason: impl Into<String>) -> Self {
        Self::Scrape {
            video_id: video_id.to_string(),
            reason: reason.into(),
        }
    }

    pub(crate) fn blueprint(video_id: &str, reason: impl Into<String>) -> Self {
        Self::Blueprint {
            video_id: video_id.to_string(),
            reason: reason.into(),
        }
    }

    /// True for failures that affect one item only, so the pipeline records
    /// them and moves on instead of aborting the whole run.
    pub fn is_per_item(&self) -> bool {
        matches!(self, Self::Scrape { .. } | Self::Blueprint { .. })
    }
}
