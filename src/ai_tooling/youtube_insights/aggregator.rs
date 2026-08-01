//! Fetches channel metrics. Built on the existing YouTube client so the API
//! surface, paging and key handling live in one place.

use crate::ai_tooling::config::AiToolingConfig;
use crate::ai_tooling::ingestion::outliers::trimmed_average;
use crate::ai_tooling::ingestion::YouTubeClient;
use crate::ai_tooling::youtube_insights::models::{ChannelMetrics, VideoMetrics};
use crate::ai_tooling::youtube_insights::outlier_engine::{self, median, OutlierSettings};
use crate::ai_tooling::youtube_insights::{InsightsError, Result};
use crate::ai_tooling::youtube_insights::models::OutlierAnalysis;
use reqwest::Client;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub struct InsightsAggregator {
    youtube: YouTubeClient,
    max_videos: usize,
    trim_percentile: f64,
}

impl InsightsAggregator {
    pub fn from_env() -> Result<Self> {
        Self::new(AiToolingConfig::load().map_err(InsightsError::Config)?)
    }

    pub fn new(config: AiToolingConfig) -> Result<Self> {
        let http = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|err| InsightsError::Api(err.to_string()))?;

        Ok(Self {
            youtube: YouTubeClient::new(&config, http),
            max_videos: config.tuning.max_videos,
            trim_percentile: config.tuning.trim_percentile,
        })
    }

    /// Samples a channel's recent uploads and computes its baseline.
    pub async fn channel_metrics(&self, channel_id: &str) -> Result<ChannelMetrics> {
        let ingest = self
            .youtube
            .ingest_channel(channel_id, self.max_videos, self.trim_percentile, f64::MAX)
            .await
            .map_err(|err| InsightsError::Api(err.to_string()))?;

        let videos: Vec<VideoMetrics> = ingest
            .videos
            .iter()
            .map(|video| VideoMetrics {
                video_id: video.video_id.clone(),
                title: video.title.clone(),
                published_at: video.published_at.clone(),
                view_count: video.view_count,
                duration_seconds: video.duration_seconds,
            })
            .collect();

        if videos.is_empty() {
            return Err(InsightsError::NoData(channel_id.to_string()));
        }

        let counts: Vec<u64> = videos.iter().map(|video| video.view_count).collect();
        let as_f64: Vec<f64> = counts.iter().map(|count| *count as f64).collect();

        Ok(ChannelMetrics {
            channel_id: channel_id.to_string(),
            title: ingest.title,
            baseline_views: trimmed_average(&counts, self.trim_percentile),
            median_views: median(&as_f64),
            videos,
            sampled_at: now_unix(),
        })
    }

    /// Metrics plus the outlier verdict — the usual entry point.
    pub async fn analyze_channel(
        &self,
        channel_id: &str,
        settings: OutlierSettings,
    ) -> Result<(ChannelMetrics, OutlierAnalysis)> {
        let metrics = self.channel_metrics(channel_id).await?;
        let analysis = outlier_engine::analyze(&metrics, settings);
        Ok((metrics, analysis))
    }

    /// Runs several channels concurrently; one failure does not sink the rest.
    pub async fn analyze_channels(
        &self,
        channel_ids: &[String],
        settings: OutlierSettings,
    ) -> Vec<(String, Result<OutlierAnalysis>)> {
        let futures = channel_ids.iter().map(|channel_id| async move {
            let result = self
                .analyze_channel(channel_id, settings)
                .await
                .map(|(_, analysis)| analysis);
            (channel_id.clone(), result)
        });

        futures_util::future::join_all(futures).await
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
