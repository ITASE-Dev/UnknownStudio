//! Channel insights: find a channel's outlier videos, measure what makes them
//! move, and turn that rhythm into edits on the user's own timeline.
//!
//! The stages are separable — metrics in, statistics, pacing, plan out — so
//! each can be tested and replaced without the others.

pub mod aggregator;
pub mod blueprint;
pub mod heatmap;
pub mod models;
pub mod outlier_engine;

pub use aggregator::InsightsAggregator;
pub use blueprint::{generate as generate_blueprint, BlueprintSettings};
pub use heatmap::{build as build_heatmap, from_transcript, TimedWord};
pub use models::{
    ChannelMetrics, OutlierAnalysis, OutlierMethod, PacingHeatmap, PacingWindow, VideoMetrics,
    ViralBlueprint, ViralScore,
};
pub use outlier_engine::{analyze, OutlierSettings};

use crate::ai_tooling::AiToolingError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum InsightsError {
    #[error("configuration: {0}")]
    Config(#[source] AiToolingError),

    #[error("youtube api: {0}")]
    Api(String),

    #[error("no uploads found for channel {0}")]
    NoData(String),

    #[error("transcript unavailable for {0}")]
    NoTranscript(String),
}

pub type Result<T> = std::result::Result<T, InsightsError>;
