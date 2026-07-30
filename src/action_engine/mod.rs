//! Self-contained AI action engine: intent routing, deterministic media math and
//! timeline edit planning. No dependency on the host UI or its data model — the
//! host speaks `ActionRequest` in and `ActionEvent` out.
#![allow(dead_code, unused_imports)]

pub mod audio;
pub mod edits;
pub mod engine;
pub mod filters;
pub mod openai;
pub mod prompts;
pub mod provider;
pub mod types;

pub use edits::{apply_cut_ranges, EditableClip, FrameRange, TimelineEdit};
pub use engine::ActionEngine;
pub use openai::OpenAiProvider;
pub use provider::{ActionProvider, ChatContext, ChatOutcome, PacingStats, Progress};
pub use types::{
    ActionContext, ActionEvent, ActionKind, ActionRequest, ChatTurn, ClipId, ClipSnapshot,
    EngineError, Recommendation, SeoStrategy, VideoProposal,
};
