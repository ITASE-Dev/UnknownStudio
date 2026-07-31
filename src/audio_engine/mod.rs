//! Everything audio: realtime playback, timeline mixing, offline analysis and
//! waveform peaks. Speaks seconds and file paths — it knows nothing about the
//! UI or the video side.
#![allow(dead_code, unused_imports)]

pub mod analysis;
pub mod engine;
pub mod mixer;
pub mod source;
pub mod waveform;

pub use analysis::{
    decode_mono_pcm, encode_wav_pcm16, silent_runs, ANALYSIS_SAMPLE_RATE,
    DEFAULT_MIN_SILENCE_SECONDS, DEFAULT_SILENCE_THRESHOLD_DB,
};
pub use engine::AudioEngine;
pub use mixer::{AudioSegment, TimelineMixer};
pub use source::AudioSource;
pub use waveform::{Waveform, WaveformService, PEAKS_PER_SECOND};
