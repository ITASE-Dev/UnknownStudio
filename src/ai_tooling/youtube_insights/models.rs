//! Metrics, heatmaps and blueprints — the data the insights engine passes
//! between its stages and persists.

use crate::ai_tooling::orchestration::dispatcher::ActionCommand;
use serde::{Deserialize, Serialize};

/// One upload, as measured.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoMetrics {
    pub video_id: String,
    pub title: String,
    pub published_at: Option<String>,
    pub view_count: u64,
    pub duration_seconds: u32,
}

impl VideoMetrics {
    pub fn url(&self) -> String {
        format!("https://www.youtube.com/watch?v={}", self.video_id)
    }
}

/// A channel's sampled uploads and the baseline they establish.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelMetrics {
    pub channel_id: String,
    pub title: Option<String>,
    pub videos: Vec<VideoMetrics>,
    /// Trimmed mean of the sample — the bar an outlier must clear.
    pub baseline_views: f64,
    /// Median, kept because the robust statistics are built on it.
    pub median_views: f64,
    pub sampled_at: u64,
}

impl ChannelMetrics {
    pub fn sample_size(&self) -> usize {
        self.videos.len()
    }

    /// Below this the distribution is too small for the statistics to mean
    /// anything, and every above-average video looks like an outlier.
    pub const MIN_RELIABLE_SAMPLE: usize = 20;

    pub fn is_reliable(&self) -> bool {
        self.sample_size() >= Self::MIN_RELIABLE_SAMPLE
    }
}

/// How an outlier was judged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutlierMethod {
    /// Median absolute deviation — robust to the very outliers being hunted.
    ModifiedZScore,
    /// MAD collapsed to zero (most uploads identical); mean absolute deviation.
    MeanAbsoluteDeviation,
    /// No spread at all: only a raw multiple of the baseline is meaningful.
    BaselineMultiple,
}

/// Why one video counts as golden.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViralScore {
    pub video_id: String,
    pub title: String,
    pub view_count: u64,
    /// Channel baseline this was measured against.
    pub baseline_views: f64,
    /// `views / baseline`. The number an editor actually recognises.
    pub multiplier: f64,
    /// Modified z-score: how many robust deviations above the median.
    pub modified_z: f64,
    /// Share of the sample this video beats, 0.0..=1.0.
    pub percentile: f64,
    pub method: OutlierMethod,
    pub is_outlier: bool,
}

impl ViralScore {
    /// One line explaining the verdict, for the UI and for the model.
    pub fn reason(&self) -> String {
        if !self.is_outlier {
            return format!(
                "{:.1}x baseline (z={:.1}) — within the channel's normal range",
                self.multiplier, self.modified_z
            );
        }
        format!(
            "{:.1}x the channel baseline, z={:.1}, beats {:.0}% of uploads",
            self.multiplier,
            self.modified_z,
            self.percentile * 100.0
        )
    }
}

/// The full verdict over a channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutlierAnalysis {
    pub channel_id: String,
    pub baseline_views: f64,
    pub median_views: f64,
    /// Robust spread the scores were divided by.
    pub deviation: f64,
    pub method: OutlierMethod,
    /// Every video scored, most viral first.
    pub scores: Vec<ViralScore>,
    pub sample_size: usize,
    pub reliable: bool,
}

impl OutlierAnalysis {
    pub fn golden(&self) -> impl Iterator<Item = &ViralScore> {
        self.scores.iter().filter(|score| score.is_outlier)
    }

    /// The single most extreme upload, outlier or not.
    pub fn best(&self) -> Option<&ViralScore> {
        self.scores.first()
    }
}

/// The rhythm of a video, as numbers an edit can be matched against.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PacingHeatmap {
    pub video_id: String,
    /// Words per minute in the opening seconds — how hard the hook works.
    pub hook_retention_wpm: f32,
    /// Words per minute across the whole video.
    pub overall_wpm: f32,
    /// Cuts per minute implied by silence gaps.
    pub jump_cut_frequency: f32,
    /// Mean silence between words, in seconds.
    pub mean_gap_sec: f32,
    /// Longest silence tolerated anywhere.
    pub max_gap_sec: f32,
    /// Cutaways per minute. `None` when no visual data was supplied — an
    /// unknown density must not be mistaken for zero.
    pub broll_density: Option<f32>,
    /// Per-window speaking rate, for drawing the heatmap.
    pub windows: Vec<PacingWindow>,
    pub duration_sec: f32,
}

/// One measured slice of the timeline.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PacingWindow {
    pub start_sec: f32,
    pub end_sec: f32,
    pub words_per_minute: f32,
    /// Silence in this window, as a fraction of its length.
    pub silence_ratio: f32,
}

impl PacingHeatmap {
    /// The busiest window — where the viral video spends its energy.
    pub fn peak_window(&self) -> Option<PacingWindow> {
        self.windows
            .iter()
            .copied()
            .max_by(|a, b| a.words_per_minute.total_cmp(&b.words_per_minute))
    }

    /// Whether the opening outpaces the body, which is what a hook does.
    pub fn hook_is_front_loaded(&self) -> bool {
        self.overall_wpm > 0.0 && self.hook_retention_wpm > self.overall_wpm * 1.1
    }
}

/// An editing plan derived from a viral reference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViralBlueprint {
    /// Video the pacing was taken from.
    pub reference_video_id: String,
    pub target: PacingHeatmap,
    /// The user's current pacing, measured the same way.
    pub current: PacingHeatmap,
    /// Edits to apply, in timeline order.
    pub actions: Vec<ActionCommand>,
    /// Human-readable rationale, one line per decision.
    pub notes: Vec<String>,
    /// Projected pacing after the plan, for the UI to show alongside the target.
    pub projected_wpm: f32,
}

impl ViralBlueprint {
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    /// How far the current edit is from the reference, before any change.
    pub fn wpm_gap(&self) -> f32 {
        self.target.overall_wpm - self.current.overall_wpm
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(multiplier: f64, is_outlier: bool) -> ViralScore {
        ViralScore {
            video_id: "abc".into(),
            title: "Clip".into(),
            view_count: 100_000,
            baseline_views: 10_000.0,
            multiplier,
            modified_z: 7.4,
            percentile: 0.98,
            method: OutlierMethod::ModifiedZScore,
            is_outlier,
        }
    }

    #[test]
    fn the_reason_states_the_verdict_and_the_numbers_behind_it() {
        let golden = score(10.0, true).reason();
        assert!(golden.contains("10.0x"));
        assert!(golden.contains("98%"));

        let ordinary = score(1.2, false).reason();
        assert!(ordinary.contains("within the channel's normal range"));
    }

    #[test]
    fn a_small_sample_is_flagged_as_unreliable() {
        let metrics = ChannelMetrics {
            channel_id: "UC1".into(),
            title: None,
            videos: Vec::new(),
            baseline_views: 0.0,
            median_views: 0.0,
            sampled_at: 0,
        };
        assert!(!metrics.is_reliable());
    }

    #[test]
    fn an_unknown_broll_density_stays_none_rather_than_zero() {
        let heatmap = PacingHeatmap {
            video_id: "abc".into(),
            hook_retention_wpm: 200.0,
            overall_wpm: 160.0,
            jump_cut_frequency: 12.0,
            mean_gap_sec: 0.2,
            max_gap_sec: 1.1,
            broll_density: None,
            windows: vec![
                PacingWindow { start_sec: 0.0, end_sec: 10.0, words_per_minute: 200.0, silence_ratio: 0.1 },
                PacingWindow { start_sec: 10.0, end_sec: 20.0, words_per_minute: 150.0, silence_ratio: 0.2 },
            ],
            duration_sec: 20.0,
        };

        assert!(heatmap.broll_density.is_none());
        assert!(heatmap.hook_is_front_loaded());
        assert_eq!(heatmap.peak_window().expect("peak").words_per_minute, 200.0);
    }
}
