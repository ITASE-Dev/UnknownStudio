//! Async FFmpeg-backed fundamental video editing toolkit.
//!
//! All public operations return [`ActionResult`] and run via
//! [`tokio::process::Command`] so they never block the egui thread.

pub mod audio_ops;
pub mod command;
pub mod concat;
pub mod crop_and_scale;
pub mod error;
pub mod overlay;
pub mod speed_adjust;
pub mod trim;

pub use audio_ops::{
    extract_audio, mute, replace_audio, replace_or_mix_audio, AudioExtractCodec, AudioMixOptions,
    AudioReplaceMode,
};
pub use concat::{concatenate, concatenate_copy, ConcatCodecMode, ConcatOptions};
pub use crop_and_scale::{
    crop_and_scale, to_horizontal_letterbox, to_vertical_short, CropScaleOptions, FitMode, Size,
};
pub use error::{ActionEngineError, ActionResult};
pub use overlay::{broll, overlay, watermark, OverlayOptions, OverlayPosition};
pub use speed_adjust::{change_speed, speed_adjust, SpeedOptions};
pub use trim::{trim, trim_accurate, trim_copy, TrimCodecMode, TrimOptions, TrimWindow};
