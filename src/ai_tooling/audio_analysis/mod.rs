//! Whisper transcription with per-word audio energy.
//!
//! The document produced here is the contract: word text and timings from the
//! recogniser, `gap_before` and `mean_dbfs` measured from the same PCM buffer
//! the recogniser saw. The whisper backend sits behind the `audio-analysis`
//! feature so the app builds without a C++ toolchain.

pub mod analyzer;
pub mod dsp;
pub mod extraction;
pub mod models;

pub use analyzer::{analyze_audio, assemble, write_report, RawTranscription, RawWord};
pub use dsp::{dbfs, gap_before, mean_dbfs, mean_dbfs_for_span, rms, FLOOR_DBFS};
pub use extraction::{extract_pcm, PcmBuffer, TARGET_SAMPLE_RATE};
pub use models::{
    AnalysisConfig, EnergyConfig, Media, Meta, PacingConfig, Segment, Transcript, TranscriptOutput,
    TranscriptStats, TranscriptionConfig, Word, SCHEMA_VERSION,
};

use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AudioAnalysisError {
    #[error("media not found: {0}")]
    MediaNotFound(PathBuf),

    #[error("no audio track in {0}")]
    NoAudio(PathBuf),

    #[error("ffmpeg: {0}")]
    Ffmpeg(String),

    #[error("transcription: {0}")]
    Transcription(String),

    #[error("serialization: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, AudioAnalysisError>;
