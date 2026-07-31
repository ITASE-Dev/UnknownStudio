//! Stage 1 — sample a channel's uploads and flag statistical outliers.

pub mod models;
pub mod outliers;
pub mod youtube;

pub use models::{parse_iso8601_duration, ChannelIngest, VideoRecord};
pub use outliers::{outlier_threshold, trimmed_average};
pub use youtube::YouTubeClient;
