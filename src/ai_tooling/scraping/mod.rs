//! Stage 2 — the "Most Replayed" peak and the speech around it.

pub mod models;
pub mod transcript;
pub mod ytdlp;

pub use models::{CaptionTrack, Cue, PeakAnalysis};
pub use ytdlp::{dump_metadata, VideoMetadata};

use crate::ai_tooling::{AiToolingError, Result};
use reqwest::Client;

pub struct DeepScraper {
    http: Client,
    /// Half-width of the transcript window around the peak.
    window_seconds: f64,
}

impl DeepScraper {
    pub fn new(http: Client, window_seconds: f64) -> Self {
        Self {
            http,
            window_seconds,
        }
    }

    /// Peak + hook transcript + pacing for one video.
    pub async fn analyze(&self, video_id: &str, url: &str) -> Result<PeakAnalysis> {
        let metadata = dump_metadata(url).await?;

        let peak = metadata.peak().ok_or_else(|| {
            AiToolingError::scrape(
                video_id,
                "no 'Most Replayed' heatmap — YouTube publishes it only for videos with enough watch data",
            )
        })?;

        let cues = self.transcript_cues(&metadata).await?;
        if cues.is_empty() {
            return Err(AiToolingError::scrape(
                video_id,
                "no English transcript available (auto-captions off?)",
            ));
        }

        let start = (peak.start_time - self.window_seconds).max(0.0);
        let end = peak.start_time + self.window_seconds;
        let (hook_text, word_count) = transcript::slice_window(&cues, start, end);

        Ok(PeakAnalysis {
            video_id: video_id.to_string(),
            peak_seconds: peak.start_time,
            peak_value: peak.value,
            hook_text,
            wpm: transcript::words_per_minute(word_count, end - start),
            segment_start: start,
            segment_end: end,
            word_count,
        })
    }

    /// Auto-generated captions first; they exist far more often than manual ones.
    async fn transcript_cues(&self, metadata: &VideoMetadata) -> Result<Vec<Cue>> {
        let track = transcript::select_english_track(&metadata.automatic_captions)
            .or_else(|| transcript::select_english_track(&metadata.subtitles));

        let Some(track) = track else {
            return Ok(Vec::new());
        };

        let response = self
            .http
            .get(&track.url)
            .header("Accept-Language", "en-US,en")
            .send()
            .await?;
        if !response.status().is_success() {
            return Ok(Vec::new());
        }

        let raw = response.text().await?;
        Ok(transcript::parse_cues(&raw, track.ext.as_deref()))
    }
}
