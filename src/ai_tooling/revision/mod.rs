//! The revision engine.
//!
//! Reads a deconstructed competitor video and the user's populated timeline,
//! and produces a reviewable plan of concrete edits. Approving one runs it:
//! generate the asset, then apply [`ActionCommand`]s through the dispatcher the
//! chat assistant already uses.
//!
//! [`ActionCommand`]: crate::ai_tooling::orchestration::dispatcher::ActionCommand

pub mod diff;
pub mod executor;
pub mod generation;
pub mod models;
pub mod timeline;

pub use diff::{ComparisonEngine, DiffSettings};
pub use executor::{execute_task, ExecutionError, ExecutionOutcome};
pub use generation::{
    generate, AssetGenerator, GeneratedAsset, GeneratedKind, GenerationError, GenerationRequest,
    IdentityMode, MockGenerator, FACIAL_IDENTITY_CONSTRAINT, IDENTITY_NEGATIVE_PROMPT,
};
pub use models::{
    Evidence, GhostKind, GhostSpan, RevisionAction, RevisionPlan, RevisionTask, TaskStatus,
};
pub use timeline::{ClipRole, ClipView, CurrentTimelineState, TrackRole, TrackView};
