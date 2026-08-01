//! Stage 4 — pipeline wiring and its result types.

pub mod context_models;
pub mod dispatcher;
pub mod engine;
pub mod models;
pub mod prompt_builder;
pub mod retention_engine;
pub mod tools;

pub use context_models::{
    AssetContext, ClipContext, MediaPoolContext, SelectionContext, TimelineContext, TrackContext,
    TrackKind,
};
pub use dispatcher::{
    apply_action, apply_actions, apply_actions_with_worker, ActionCommand, AsyncJob,
    DispatchReport, DispatcherError, EditorState, Marker, Outcome,
};
pub use engine::BlueprintEngine;
pub use models::{
    Blueprint, BlueprintFields, PipelineReport, Stage, StageFailure, TextAnimation, TextStyle,
};
pub use prompt_builder::{CommandSpec, PromptBuilder, PromptContext, ACTION_COMMANDS};
pub use retention_engine::{
    enhance, generate_captions, generate_sfx, CaptionSettings, SfxSettings, SFX_BOOM, SFX_RISER,
    SFX_WHOOSH,
};
pub use tools::{anthropic_tools, get_available_tools, parse_tool_call, ToolCallError};
