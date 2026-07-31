//! Ingestion domain types.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoRecord {
    pub video_id: String,
    pub channel_id: String,
    pub title: String,
    /// RFC 3339, as returned by the API.
    pub published_at: Option<String>,
    pub view_count: u64,
    pub duration_seconds: u32,
    pub is_outlier: bool,
}

impl VideoRecord {
    pub fn url(&self) -> String {
        format!("https://www.youtube.com/watch?v={}", self.video_id)
    }
}

/// One channel's sampled uploads plus the baseline they were judged against.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelIngest {
    pub channel_id: String,
    pub title: Option<String>,
    pub uploads_playlist_id: Option<String>,
    /// Trimmed mean of the sampled view counts.
    pub average_views: f64,
    /// `average_views * outlier_multiplier`.
    pub threshold: f64,
    pub videos: Vec<VideoRecord>,
}

impl ChannelIngest {
    pub fn outliers(&self) -> impl Iterator<Item = &VideoRecord> {
        self.videos.iter().filter(|video| video.is_outlier)
    }

    /// A baseline from fewer than this many uploads produces false outliers.
    pub const MIN_RELIABLE_SAMPLE: usize = 20;

    pub fn sample_is_reliable(&self) -> bool {
        self.videos.len() >= Self::MIN_RELIABLE_SAMPLE
    }
}

/// `PT1H2M3S` → seconds. Anything unparseable is zero rather than an error;
/// duration is metadata, not a reason to drop a video.
pub fn parse_iso8601_duration(value: &str) -> u32 {
    let Some(body) = value.strip_prefix('P') else {
        return 0;
    };

    let (mut seconds, mut number, mut in_time) = (0u32, String::new(), false);
    for ch in body.chars() {
        match ch {
            'T' => in_time = true,
            c if c.is_ascii_digit() => number.push(c),
            // 'M' before the time marker is months, not minutes.
            'M' if !in_time => number.clear(),
            unit => {
                let multiplier = match unit {
                    'D' => 86_400,
                    'H' => 3_600,
                    'M' => 60,
                    'S' => 1,
                    _ => 0,
                };
                seconds += number.parse::<u32>().unwrap_or(0) * multiplier;
                number.clear();
            }
        }
    }
    seconds
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_cover_every_unit() {
        assert_eq!(parse_iso8601_duration("PT1H2M3S"), 3723);
        assert_eq!(parse_iso8601_duration("PT45S"), 45);
        assert_eq!(parse_iso8601_duration("P1DT30M"), 88_200);
        // Months must not be read as minutes.
        assert_eq!(parse_iso8601_duration("P2MT10S"), 10);
        assert_eq!(parse_iso8601_duration("garbage"), 0);
    }
}
