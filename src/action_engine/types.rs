use crate::action_engine::edits::TimelineEdit;
use serde::Deserialize;
use std::fmt;
use std::path::PathBuf;
use uuid::Uuid;

pub type ClipId = Uuid;

#[derive(Debug, Clone)]
pub enum EngineError {
    MissingApiKey,
    Http(String),
    Protocol(String),
    Media(String),
    InvalidContext(&'static str),
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingApiKey => write!(f, "OPENAI_API_KEY is not set"),
            Self::Http(m) => write!(f, "request failed: {m}"),
            Self::Protocol(m) => write!(f, "unexpected response: {m}"),
            Self::Media(m) => write!(f, "media error: {m}"),
            Self::InvalidContext(field) => write!(f, "missing context field: {field}"),
        }
    }
}

impl std::error::Error for EngineError {}

/// One line of chat history, decoupled from whatever the host renders.
#[derive(Clone)]
pub struct ChatTurn {
    pub from_user: bool,
    pub text: String,
}

/// Everything the engine needs to reason about a clip without owning the host's
/// timeline model. Frames are absolute timeline frames; `trim_in_frames` is the
/// source-side offset.
#[derive(Clone, Debug)]
pub struct ClipSnapshot {
    pub id: ClipId,
    pub source_path: String,
    pub start_frame: u64,
    pub trim_in_frames: u64,
    pub duration_frames: u64,
    pub fps: f64,
}

impl ClipSnapshot {
    pub fn fps(&self) -> f64 {
        self.fps.max(1.0)
    }

    /// `[start, duration)` of this clip inside the SOURCE file, in seconds.
    pub fn source_window(&self) -> (f64, f64) {
        let fps = self.fps();
        (
            self.trim_in_frames as f64 / fps,
            self.duration_frames as f64 / fps,
        )
    }
}

/// Action vocabulary shared with the model. Unknown strings degrade to
/// `InpaintVideo`, the variant that unconditionally carries the identity
/// constraint — an unrecognised action must never lose it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ActionKind {
    Trim,
    TrimSilence,
    ColorCorrect,
    GenerateBroll,
    InpaintVideo,
}

impl ActionKind {
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_uppercase().as_str() {
            "TRIM" => Self::Trim,
            "TRIM_SILENCE" => Self::TrimSilence,
            "COLOR_CORRECT" => Self::ColorCorrect,
            "GENERATE_BROLL" => Self::GenerateBroll,
            _ => Self::InpaintVideo,
        }
    }
}

/// One recommendation from the analysis pass. Field names mirror the JSON
/// schema the model is instructed to emit.
#[derive(Deserialize, Clone, Debug)]
pub struct Recommendation {
    pub id: String,
    pub critique: String,
    pub proposed_action: String,
    pub action_type: String,
    #[serde(default)]
    pub context: Option<String>,
}

impl Recommendation {
    pub fn kind(&self) -> ActionKind {
        ActionKind::parse(&self.action_type)
    }
}

#[derive(Deserialize, Clone, Debug)]
pub struct SeoStrategy {
    pub titles: Vec<String>,
    pub description: String,
    pub tags: Vec<String>,
    pub pacing_critique: String,
}

/// Video generation is expensive, so the model's tool call becomes a proposal
/// the host confirms rather than an immediate job.
#[derive(Clone, Debug)]
pub struct VideoProposal {
    pub prompt: String,
    pub seconds: u32,
    pub target_clip_id: Option<ClipId>,
}

/// Approved-recommendation payload. Typed instead of a JSON blob: the engine
/// never re-parses host state.
#[derive(Clone, Debug)]
pub struct ActionContext {
    pub critique: String,
    pub proposed_action: String,
    /// Free-form model text (e.g. raw `eq` values for `COLOR_CORRECT`).
    pub ai_context: Option<String>,
    pub clip: ClipSnapshot,
}

impl ActionContext {
    pub fn from_recommendation(recommendation: &Recommendation, clip: ClipSnapshot) -> Self {
        Self {
            critique: recommendation.critique.clone(),
            proposed_action: recommendation.proposed_action.clone(),
            ai_context: recommendation.context.clone(),
            clip,
        }
    }

    pub(crate) fn generation_prompt(&self) -> String {
        format!("{}: {}", self.proposed_action, self.critique)
    }
}

/// Host → engine.
pub enum ActionRequest {
    Chat {
        prompt: String,
        history: Vec<ChatTurn>,
        target_clip_id: Option<ClipId>,
    },
    GenerateImage {
        prompt: String,
        target_clip_id: ClipId,
    },
    /// Sent only after the user confirms a `VideoProposal`.
    GenerateVideo {
        prompt: String,
        seconds: u32,
        target_clip_id: Option<ClipId>,
    },
    /// Frames are pre-encoded JPEGs sampled by the host; audio is extracted by
    /// the worker so the UI thread never touches FFmpeg.
    AnalyzeClip {
        clip: ClipSnapshot,
        sampled_frames: Vec<Vec<u8>>,
    },
    ExecuteAction {
        kind: ActionKind,
        context: ActionContext,
    },
    GenerateSeoStrategy {
        total_duration_seconds: f64,
        total_clips: usize,
        transcript: String,
    },
}

/// Engine → host. Every variant is terminal except `Progress`.
pub enum ActionEvent {
    /// Transient status line; not part of the durable chat history.
    Progress(String),
    ChatReply(String),
    /// The model asked for an analysis; frame sampling belongs to the host.
    RunClipAnalysis(ClipId),
    AnalysisReady {
        clip_id: ClipId,
        recommendations: Vec<Recommendation>,
    },
    TranscriptReady {
        clip_id: ClipId,
        transcript: String,
    },
    SeoStrategyReady(SeoStrategy),
    VideoProposed(VideoProposal),
    ImageReady {
        target_clip_id: ClipId,
        image_bytes: Vec<u8>,
    },
    VideoReady {
        target_clip_id: Option<ClipId>,
        path: PathBuf,
    },
    /// A planned, host-applicable timeline mutation.
    Edit(TimelineEdit),
    Failed(EngineError),
}
