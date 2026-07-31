//! Scraping domain types.

use serde::{Deserialize, Serialize};

/// One transcript cue: `[start, end)` in seconds plus its text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cue {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// The most-replayed moment and the speech around it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeakAnalysis {
    pub video_id: String,
    /// Timestamp of maximum replay.
    pub peak_seconds: f64,
    /// Heatmap intensity at the peak, 0.0..=1.0.
    pub peak_value: f64,
    /// Verbatim transcript of the window.
    pub hook_text: String,
    /// Words per minute across the window.
    pub wpm: f64,
    pub segment_start: f64,
    pub segment_end: f64,
    pub word_count: usize,
}

/// A caption track advertised by yt-dlp.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptionTrack {
    pub url: String,
    #[serde(default)]
    pub ext: Option<String>,
}
