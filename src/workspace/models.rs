//! Serializable project state. These types are the on-disk contract; the UI
//! converts its own runtime structures to and from them.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Bumped whenever a field changes meaning; readers refuse newer majors.
pub const PROJECT_FORMAT_VERSION: u32 = 1;

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum TargetPlatform {
    #[default]
    YouTubeLandscape,
    YouTubeShorts,
    TikTok,
    InstagramReels,
    Custom {
        width: u32,
        height: u32,
    },
}

impl TargetPlatform {
    pub fn resolution(self) -> (u32, u32) {
        match self {
            Self::YouTubeLandscape => (1920, 1080),
            Self::YouTubeShorts | Self::TikTok | Self::InstagramReels => (1080, 1920),
            Self::Custom { width, height } => (width, height),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProjectSettings {
    pub fps: f64,
    pub target: TargetPlatform,
    /// Editing blueprint / style preset the director follows.
    #[serde(default)]
    pub blueprint: Option<String>,
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self {
            fps: 30.0,
            target: TargetPlatform::default(),
            blueprint: None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ClipKind {
    ARoll,
    BRoll,
    Audio,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    Video,
    Audio,
}

/// One clip, in timeline seconds. Sources are stored relative to the project
/// root when they live inside it, so a project folder stays portable.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ClipSnapshot {
    pub id: u64,
    pub label: String,
    #[serde(default)]
    pub source: Option<PathBuf>,
    pub kind: ClipKind,
    pub start_seconds: f32,
    pub duration_seconds: f32,
    #[serde(default)]
    pub trim_in_seconds: f32,
    #[serde(default)]
    pub source_seconds: f32,
    #[serde(default)]
    pub has_audio: bool,
    #[serde(default = "unit_gain")]
    pub gain: f32,
}

fn unit_gain() -> f32 {
    1.0
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TrackSnapshot {
    pub name: String,
    pub kind: TrackKind,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub clips: Vec<ClipSnapshot>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct TimelineSnapshot {
    #[serde(default)]
    pub tracks: Vec<TrackSnapshot>,
    #[serde(default)]
    pub playhead_seconds: f32,
    #[serde(default = "unit_gain")]
    pub zoom: f32,
}

/// Media the project knows about, whether imported or generated.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MediaEntry {
    /// Path relative to the project root.
    pub path: PathBuf,
    pub display_name: String,
    #[serde(default)]
    pub duration_seconds: f32,
    #[serde(default)]
    pub has_audio: bool,
    #[serde(default)]
    pub generated: bool,
    /// Model or tool that produced a generated asset.
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub imported_at: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProjectConfig {
    pub version: u32,
    pub id: String,
    pub name: String,
    pub created_at: u64,
    pub modified_at: u64,
    #[serde(default)]
    pub settings: ProjectSettings,
    #[serde(default)]
    pub timeline: TimelineSnapshot,
    #[serde(default)]
    pub media: Vec<MediaEntry>,
}

impl ProjectConfig {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        let now = now_unix();
        Self {
            version: PROJECT_FORMAT_VERSION,
            id: id.into(),
            name: name.into(),
            created_at: now,
            modified_at: now,
            settings: ProjectSettings::default(),
            timeline: TimelineSnapshot::default(),
            media: Vec::new(),
        }
    }

    pub fn touch(&mut self) {
        self.modified_at = now_unix();
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ChatMessage {
    pub from_user: bool,
    pub text: String,
    #[serde(default)]
    pub at: u64,
}

impl ChatMessage {
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            from_user: true,
            text: text.into(),
            at: now_unix(),
        }
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            from_user: false,
            text: text.into(),
            at: now_unix(),
        }
    }
}

impl TimelineSnapshot {
    pub fn clip_count(&self) -> usize {
        self.tracks.iter().map(|t| t.clips.len()).sum()
    }

    /// Last clip boundary on any track.
    pub fn duration_seconds(&self) -> f32 {
        self.tracks
            .iter()
            .flat_map(|t| t.clips.iter().map(|c| c.start_seconds + c.duration_seconds))
            .fold(0.0, f32::max)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct ChatHistory {
    #[serde(default)]
    pub messages: Vec<ChatMessage>,
}

impl ChatHistory {
    pub fn push(&mut self, message: ChatMessage) {
        self.messages.push(message);
    }
}

/// One retention recommendation, mirroring the analysis schema the model emits.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AnalysisNote {
    pub id: String,
    pub critique: String,
    pub proposed_action: String,
    pub action_type: String,
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default)]
    pub applied: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct VisualAnalysis {
    #[serde(default)]
    pub notes: Vec<AnalysisNote>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct AudioAnalysis {
    #[serde(default)]
    pub transcript: Option<String>,
    /// `[start, end)` silence windows in source seconds.
    #[serde(default)]
    pub silence_seconds: Vec<(f32, f32)>,
    /// Peak per bucket, as produced by the audio engine.
    #[serde(default)]
    pub peaks: Vec<f32>,
    #[serde(default)]
    pub peaks_per_second: f32,
}

/// Per-media analysis, one JSON file each under `metadata/`.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AnalysisMetadata {
    /// Media this analysis belongs to, relative to the project root.
    pub media: PathBuf,
    #[serde(default)]
    pub visual: VisualAnalysis,
    #[serde(default)]
    pub audio: AudioAnalysis,
    #[serde(default)]
    pub updated_at: u64,
}

impl AnalysisMetadata {
    pub fn new(media: impl Into<PathBuf>) -> Self {
        Self {
            media: media.into(),
            visual: VisualAnalysis::default(),
            audio: AudioAnalysis::default(),
            updated_at: now_unix(),
        }
    }
}

/// Lightweight row for a project picker, without holding the whole timeline.
#[derive(Clone, Debug)]
pub struct ProjectSummary {
    pub id: String,
    pub name: String,
    pub modified_at: u64,
    pub clip_count: usize,
    pub duration_seconds: f32,
    pub target: TargetPlatform,
    pub blueprint: Option<String>,
    pub root: PathBuf,
}

impl ProjectSummary {
    pub fn from_config(config: &ProjectConfig, root: PathBuf) -> Self {
        Self {
            id: config.id.clone(),
            name: config.name.clone(),
            modified_at: config.modified_at,
            clip_count: config.timeline.clip_count(),
            duration_seconds: config.timeline.duration_seconds(),
            target: config.settings.target,
            blueprint: config.settings.blueprint.clone(),
            root,
        }
    }
}
