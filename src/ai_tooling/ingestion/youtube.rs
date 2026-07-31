//! YouTube Data API v3 client — channel metadata, uploads playlist, video stats.

use crate::ai_tooling::ingestion::models::{parse_iso8601_duration, ChannelIngest, VideoRecord};
use crate::ai_tooling::ingestion::outliers::{outlier_threshold, trimmed_average};
use crate::ai_tooling::{AiToolingConfig, AiToolingError, Result};
use reqwest::Client;
use serde::Deserialize;

const API_ROOT: &str = "https://www.googleapis.com/youtube/v3";
/// Hard API limit for `id`-list and `maxResults` requests.
const PAGE_SIZE: usize = 50;

pub struct YouTubeClient {
    http: Client,
    api_key: String,
}

impl YouTubeClient {
    pub fn new(config: &AiToolingConfig, http: Client) -> Self {
        Self {
            http,
            api_key: config.youtube_api_key.clone(),
        }
    }

    /// Samples a channel's latest uploads and flags the statistical outliers.
    pub async fn ingest_channel(
        &self,
        channel_id: &str,
        max_videos: usize,
        trim_percentile: f64,
        outlier_multiplier: f64,
    ) -> Result<ChannelIngest> {
        let (title, uploads_playlist_id) = self.channel_meta(channel_id).await?;

        let mut ingest = ChannelIngest {
            channel_id: channel_id.to_string(),
            title,
            uploads_playlist_id: uploads_playlist_id.clone(),
            average_views: 0.0,
            threshold: f64::INFINITY,
            videos: Vec::new(),
        };

        // No uploads playlist means the id is wrong (a @handle instead of UC…)
        // or the channel is empty; either way there is nothing to measure.
        let Some(playlist_id) = uploads_playlist_id else {
            return Ok(ingest);
        };

        let video_ids = self.playlist_video_ids(&playlist_id, max_videos).await?;
        let mut videos = self.video_details(&video_ids, channel_id).await?;
        if videos.is_empty() {
            return Ok(ingest);
        }

        let counts: Vec<u64> = videos.iter().map(|video| video.view_count).collect();
        let average = trimmed_average(&counts, trim_percentile);
        let threshold = outlier_threshold(average, outlier_multiplier);
        for video in &mut videos {
            video.is_outlier = video.view_count as f64 > threshold;
        }

        ingest.average_views = average;
        ingest.threshold = threshold;
        ingest.videos = videos;
        Ok(ingest)
    }

    /// `(title, uploads playlist id)`; both `None` when the channel is unknown.
    pub async fn channel_meta(&self, channel_id: &str) -> Result<(Option<String>, Option<String>)> {
        let response: ChannelListResponse = self
            .get(
                "channels",
                &[
                    ("part", "snippet,contentDetails"),
                    ("id", channel_id),
                    ("maxResults", "1"),
                ],
            )
            .await?;

        let Some(item) = response.items.into_iter().next() else {
            return Ok((None, None));
        };
        Ok((
            item.snippet.and_then(|snippet| snippet.title),
            item.content_details
                .and_then(|details| details.related_playlists)
                .and_then(|playlists| playlists.uploads),
        ))
    }

    /// Walks the uploads playlist until `limit` ids are collected.
    pub async fn playlist_video_ids(&self, playlist_id: &str, limit: usize) -> Result<Vec<String>> {
        let mut collected: Vec<String> = Vec::with_capacity(limit);
        let mut page_token: Option<String> = None;

        while collected.len() < limit {
            let page_size = PAGE_SIZE.min(limit - collected.len()).to_string();
            let mut query = vec![
                ("part", "contentDetails"),
                ("playlistId", playlist_id),
                ("maxResults", page_size.as_str()),
            ];
            if let Some(token) = page_token.as_deref() {
                query.push(("pageToken", token));
            }

            let response: PlaylistItemsResponse = self.get("playlistItems", &query).await?;
            collected.extend(
                response
                    .items
                    .into_iter()
                    .filter_map(|item| item.content_details.and_then(|d| d.video_id)),
            );

            page_token = response.next_page_token;
            if page_token.is_none() {
                break;
            }
        }

        collected.truncate(limit);
        Ok(collected)
    }

    /// Statistics for up to `PAGE_SIZE` ids per request.
    pub async fn video_details(
        &self,
        video_ids: &[String],
        channel_id: &str,
    ) -> Result<Vec<VideoRecord>> {
        let mut records = Vec::with_capacity(video_ids.len());

        for chunk in video_ids.chunks(PAGE_SIZE) {
            let ids = chunk.join(",");
            let response: VideoListResponse = self
                .get(
                    "videos",
                    &[
                        ("part", "snippet,statistics,contentDetails"),
                        ("id", ids.as_str()),
                        ("maxResults", "50"),
                    ],
                )
                .await?;

            records.extend(response.items.into_iter().filter_map(|item| {
                let video_id = item.id?;
                let snippet = item.snippet.unwrap_or_default();
                Some(VideoRecord {
                    video_id,
                    channel_id: channel_id.to_string(),
                    title: snippet.title.unwrap_or_default(),
                    published_at: snippet.published_at,
                    view_count: item
                        .statistics
                        .and_then(|stats| stats.view_count)
                        .and_then(|count| count.parse().ok())
                        .unwrap_or(0),
                    duration_seconds: item
                        .content_details
                        .and_then(|details| details.duration)
                        .map(|iso| parse_iso8601_duration(&iso))
                        .unwrap_or(0),
                    is_outlier: false,
                })
            }));
        }

        Ok(records)
    }

    async fn get<T: for<'de> Deserialize<'de>>(
        &self,
        endpoint: &str,
        query: &[(&str, &str)],
    ) -> Result<T> {
        let response = self
            .http
            .get(format!("{API_ROOT}/{endpoint}"))
            .query(query)
            .query(&[("key", self.api_key.as_str())])
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            return Err(AiToolingError::Api {
                service: "youtube",
                status: status.as_u16(),
                body: response.text().await.unwrap_or_default(),
            });
        }
        Ok(response.json().await?)
    }
}

// -- Wire format ---------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ChannelListResponse {
    #[serde(default)]
    items: Vec<ChannelItem>,
}

#[derive(Debug, Deserialize)]
struct ChannelItem {
    snippet: Option<Snippet>,
    #[serde(rename = "contentDetails")]
    content_details: Option<ChannelContentDetails>,
}

#[derive(Debug, Deserialize)]
struct ChannelContentDetails {
    #[serde(rename = "relatedPlaylists")]
    related_playlists: Option<RelatedPlaylists>,
}

#[derive(Debug, Deserialize)]
struct RelatedPlaylists {
    uploads: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PlaylistItemsResponse {
    #[serde(default)]
    items: Vec<PlaylistItem>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PlaylistItem {
    #[serde(rename = "contentDetails")]
    content_details: Option<PlaylistItemDetails>,
}

#[derive(Debug, Deserialize)]
struct PlaylistItemDetails {
    #[serde(rename = "videoId")]
    video_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct VideoListResponse {
    #[serde(default)]
    items: Vec<VideoItem>,
}

#[derive(Debug, Deserialize)]
struct VideoItem {
    id: Option<String>,
    snippet: Option<Snippet>,
    statistics: Option<Statistics>,
    #[serde(rename = "contentDetails")]
    content_details: Option<VideoContentDetails>,
}

#[derive(Debug, Default, Deserialize)]
struct Snippet {
    title: Option<String>,
    #[serde(rename = "publishedAt")]
    published_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Statistics {
    /// The API sends counts as strings.
    #[serde(rename = "viewCount")]
    view_count: Option<String>,
}

#[derive(Debug, Deserialize)]
struct VideoContentDetails {
    duration: Option<String>,
}
