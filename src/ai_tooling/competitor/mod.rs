//! Competitor data warehouse.
//!
//! `youtube_insights` decides *which* uploads are outliers. This module takes
//! those outliers apart — retention, audio, cutting rhythm, transcript, hooks —
//! and stores the result so the revision engine can diff against it later.

pub mod ingest;
pub mod models;
pub mod store;

pub use ingest::{deconstruct, IngestError, STAGES};
pub use models::{
    AudioDynamics, BRollPlacement, CompetitorVideo, DropCause, EndingStyle, HeatmapPeak,
    HookAnalysis, HookType, RetentionAnalysis, RetentionDrop, SceneCut, SilenceGap,
    TranscriptAndHooks, TranscriptSegment, TransitionSound, VideoEndingAnalysis,
    VisualAndPacingStructure, VlmTag, VolumeEvent,
};
pub use store::{
    CompetitorDataStore, InMemoryWarehouse, SemanticEntry, SemanticHit, SemanticIndex,
    SemanticKind, StoreError,
};
