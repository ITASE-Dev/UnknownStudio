//! yt-dlp metadata extraction. The Python original imports the module; here it
//! runs as a subprocess and we parse the single-JSON dump.

use crate::ai_tooling::scraping::models::CaptionTrack;
use crate::ai_tooling::{AiToolingError, Result};
use serde::Deserialize;
use std::collections::HashMap;
use tokio::process::Command;

/// One `--dump-single-json` payload, reduced to the fields this pipeline uses.
#[derive(Debug, Clone, Deserialize)]
pub struct VideoMetadata {
    #[serde(default)]
    pub heatmap: Option<Vec<HeatmapEntry>>,
    #[serde(default)]
    pub automatic_captions: HashMap<String, Vec<CaptionTrack>>,
    #[serde(default)]
    pub subtitles: HashMap<String, Vec<CaptionTrack>>,
}

/// One "Most Replayed" bucket.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct HeatmapEntry {
    #[serde(default)]
    pub start_time: f64,
    #[serde(default)]
    pub value: f64,
}

impl VideoMetadata {
    /// Highest-intensity bucket, or `None` when YouTube published no heatmap
    /// (it only does so for videos with enough watch data).
    pub fn peak(&self) -> Option<HeatmapEntry> {
        self.heatmap
            .as_ref()?
            .iter()
            .copied()
            .max_by(|a, b| a.value.total_cmp(&b.value))
    }
}

/// Runs yt-dlp for one URL. Metadata only — nothing is downloaded.
pub async fn dump_metadata(url: &str) -> Result<VideoMetadata> {
    let output = Command::new("yt-dlp")
        .args([
            "--dump-single-json",
            "--skip-download",
            "--no-playlist",
            "--no-warnings",
            "--no-progress",
            "--socket-timeout",
            "30",
            "--retries",
            "2",
            url,
        ])
        .output()
        .await
        .map_err(|err| AiToolingError::YtDlp(format!("could not run yt-dlp: {err}")))?;

    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        let reason = detail.lines().last().unwrap_or("extraction failed").trim();
        return Err(AiToolingError::YtDlp(reason.to_string()));
    }

    serde_json::from_slice(&output.stdout).map_err(AiToolingError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_peak_is_the_loudest_bucket() {
        let metadata: VideoMetadata = serde_json::from_str(
            r#"{"heatmap":[
                {"start_time":0.0,"value":0.2},
                {"start_time":42.5,"value":0.97},
                {"start_time":90.0,"value":0.5}
            ]}"#,
        )
        .expect("parse");

        let peak = metadata.peak().expect("peak");
        assert_eq!(peak.start_time, 42.5);
        assert_eq!(peak.value, 0.97);
    }

    #[test]
    fn no_heatmap_means_no_peak() {
        let metadata: VideoMetadata = serde_json::from_str("{}").expect("parse");
        assert!(metadata.peak().is_none());
        assert!(metadata.automatic_captions.is_empty());
    }
}
