//! Stage 4 — pipeline wiring and its result types.

pub mod engine;
pub mod models;

pub use engine::BlueprintEngine;
pub use models::{Blueprint, BlueprintFields, PipelineReport, Stage, StageFailure};
