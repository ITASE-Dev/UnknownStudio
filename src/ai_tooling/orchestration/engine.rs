//! Wires the stages together: ingest → scrape → blueprint.
//!
//! Per-item failures are recorded in the report; only a broken configuration or
//! a dead API aborts a run.

use crate::ai_tooling::config::AiToolingConfig;
use crate::ai_tooling::ingestion::{ChannelIngest, VideoRecord, YouTubeClient};
use crate::ai_tooling::orchestration::models::{
    Blueprint, BlueprintFields, PipelineReport, Stage,
};
use crate::ai_tooling::prompting::{build_payload, output_schema, SYSTEM_PROMPT};
use crate::ai_tooling::providers::{Completion, LlmClient};
use crate::ai_tooling::scraping::{DeepScraper, PeakAnalysis};
use crate::ai_tooling::{AiToolingError, Result};
use reqwest::Client;
use std::time::Duration;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

pub struct BlueprintEngine {
    config: AiToolingConfig,
    youtube: YouTubeClient,
    scraper: DeepScraper,
    llm: LlmClient,
}

impl BlueprintEngine {
    /// Builds every stage from `.env`.
    pub fn from_env() -> Result<Self> {
        Self::new(AiToolingConfig::load()?)
    }

    pub fn new(config: AiToolingConfig) -> Result<Self> {
        let http = Client::builder().timeout(REQUEST_TIMEOUT).build()?;
        Ok(Self {
            youtube: YouTubeClient::new(&config, http.clone()),
            scraper: DeepScraper::new(http.clone(), config.tuning.peak_window_seconds),
            llm: LlmClient::from_config(&config, http)?,
            config,
        })
    }

    pub fn config(&self) -> &AiToolingConfig {
        &self.config
    }

    /// Full run over every configured channel.
    pub async fn run(&self) -> Result<PipelineReport> {
        let mut report = PipelineReport::default();

        for channel_id in &self.config.channel_ids {
            report.channels_scanned += 1;
            match self.ingest(channel_id).await {
                Ok(ingest) => {
                    report.videos_sampled += ingest.videos.len();
                    let outliers: Vec<VideoRecord> = ingest.outliers().cloned().collect();
                    report.outliers_found += outliers.len();
                    self.process_outliers(&outliers, &mut report).await;
                }
                // A dead channel must not take the other channels down with it.
                Err(err) => report.record(Stage::Ingest, channel_id, err.to_string()),
            }
        }

        Ok(report)
    }

    /// Sampled uploads for one channel, outliers already flagged.
    pub async fn ingest(&self, channel_id: &str) -> Result<ChannelIngest> {
        let tuning = &self.config.tuning;
        self.youtube
            .ingest_channel(
                channel_id,
                tuning.max_videos,
                tuning.trim_percentile,
                tuning.outlier_multiplier,
            )
            .await
    }

    /// Scrape + blueprint for one video — the entry point the editor uses when
    /// a user points at a single reference video.
    pub async fn analyze_video(&self, video_id: &str, url: &str) -> Result<(PeakAnalysis, Blueprint)> {
        let peak = self.scraper.analyze(video_id, url).await?;
        let blueprint = self.blueprint(&peak).await?;
        Ok((peak, blueprint))
    }

    /// Turns scraped telemetry into a rulebook.
    pub async fn blueprint(&self, peak: &PeakAnalysis) -> Result<Blueprint> {
        if peak.hook_text.trim().is_empty() {
            return Err(AiToolingError::blueprint(
                &peak.video_id,
                "empty hook transcript — nothing for the model to analyse",
            ));
        }

        let completion = self
            .llm
            .complete(SYSTEM_PROMPT, &build_payload(peak), &output_schema())
            .await?;

        let Completion::Json(raw) = completion else {
            return Err(AiToolingError::blueprint(
                &peak.video_id,
                "provider returned no content (refusal or empty completion)",
            ));
        };

        let fields: BlueprintFields = serde_json::from_str(&raw).map_err(|err| {
            AiToolingError::blueprint(&peak.video_id, format!("model returned non-JSON output: {err}"))
        })?;

        Ok(fields.attribute(&peak.video_id, self.llm.kind(), self.llm.model_id()))
    }

    async fn process_outliers(&self, outliers: &[VideoRecord], report: &mut PipelineReport) {
        for video in outliers {
            let peak = match self.scraper.analyze(&video.video_id, &video.url()).await {
                Ok(peak) => peak,
                Err(err) => {
                    report.record(Stage::Scrape, &video.video_id, err.to_string());
                    continue;
                }
            };

            match self.blueprint(&peak).await {
                Ok(blueprint) => report.blueprints.push(blueprint),
                Err(err) => report.record(Stage::Blueprint, &video.video_id, err.to_string()),
            }
            report.peaks.push(peak);
        }
    }
}
