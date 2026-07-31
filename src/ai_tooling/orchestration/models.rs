//! Pipeline results: the rulebook itself and the report of one run.

use crate::ai_tooling::config::ProviderKind;
use serde::{Deserialize, Serialize};

/// The editing rulebook reverse-engineered from a retention spike.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Blueprint {
    pub video_id: String,
    /// Pacing target in words per minute.
    pub target_wpm: i64,
    /// Longest dead air tolerated before a cut.
    pub max_silence_ms: i64,
    /// Short label of the rhetorical device the hook uses.
    pub hook_style: String,
    /// Seconds between visual cutaways.
    pub b_roll_frequency_sec: f64,
    /// Imperative editor directives, 3–6 entries.
    #[serde(default)]
    pub heatmap_peak_actions: Vec<String>,
    /// Which model produced this, recorded for reproducibility.
    pub provider: ProviderKind,
    pub model_id: String,
}

/// The model's answer, before provenance is attached.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueprintFields {
    pub target_wpm: i64,
    pub max_silence_ms: i64,
    pub hook_style: String,
    pub b_roll_frequency_sec: f64,
    #[serde(default)]
    pub heatmap_peak_actions: Vec<String>,
}

impl BlueprintFields {
    pub fn attribute(self, video_id: &str, provider: ProviderKind, model_id: &str) -> Blueprint {
        Blueprint {
            video_id: video_id.to_string(),
            target_wpm: self.target_wpm,
            max_silence_ms: self.max_silence_ms,
            hook_style: self.hook_style,
            b_roll_frequency_sec: self.b_roll_frequency_sec,
            heatmap_peak_actions: self.heatmap_peak_actions,
            provider,
            model_id: model_id.to_string(),
        }
    }
}

/// Which stage a per-item failure came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Ingest,
    Scrape,
    Blueprint,
}

/// One recorded failure. Collected rather than raised, so one dead video does
/// not end the run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageFailure {
    pub stage: Stage,
    /// Channel id for ingest failures, video id otherwise.
    pub subject: String,
    pub reason: String,
}

/// Everything one pipeline run produced.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PipelineReport {
    pub channels_scanned: usize,
    pub videos_sampled: usize,
    pub outliers_found: usize,
    pub peaks: Vec<crate::ai_tooling::scraping::PeakAnalysis>,
    pub blueprints: Vec<Blueprint>,
    pub failures: Vec<StageFailure>,
}

impl PipelineReport {
    pub fn record(&mut self, stage: Stage, subject: impl Into<String>, reason: impl Into<String>) {
        self.failures.push(StageFailure {
            stage,
            subject: subject.into(),
            reason: reason.into(),
        });
    }
}
