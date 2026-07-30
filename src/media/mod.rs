//! Media ingest: file picking, probing and the pool item the UI renders.
#![allow(dead_code, unused_imports)]

pub mod decoder;
pub mod engine;
pub mod probe;
pub mod textures;

pub use decoder::{Quality, RgbFrame, VideoDecoder};
pub use engine::{Decoded, PreviewEngine, Segment};
pub use probe::{probe_media, MediaInfo};
pub use textures::{Poster, Textures};

use std::path::{Path, PathBuf};

pub const VIDEO_EXTENSIONS: [&str; 6] = ["mp4", "mov", "avi", "mkv", "webm", "m4v"];
pub const AUDIO_EXTENSIONS: [&str; 6] = ["mp3", "wav", "aac", "m4a", "flac", "ogg"];
pub const IMAGE_EXTENSIONS: [&str; 4] = ["png", "jpg", "jpeg", "webp"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MediaKind {
    Video,
    Audio,
    Image,
}

impl MediaKind {
    pub fn from_extension(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        if VIDEO_EXTENSIONS.contains(&ext.as_str()) {
            Some(Self::Video)
        } else if AUDIO_EXTENSIONS.contains(&ext.as_str()) {
            Some(Self::Audio)
        } else if IMAGE_EXTENSIONS.contains(&ext.as_str()) {
            Some(Self::Image)
        } else {
            None
        }
    }
}

/// A file that made it into the pool.
#[derive(Clone)]
pub struct ImportedMedia {
    pub name: String,
    pub path: PathBuf,
    pub kind: MediaKind,
    pub seconds: f32,
    pub fps: f64,
    pub has_audio: bool,
    /// Human-readable format line for the inspector.
    pub meta: String,
}

fn all_extensions() -> Vec<&'static str> {
    VIDEO_EXTENSIONS
        .iter()
        .chain(AUDIO_EXTENSIONS.iter())
        .chain(IMAGE_EXTENSIONS.iter())
        .copied()
        .collect()
}

/// Native multi-select dialog, filtered to media this app can open.
pub fn pick_files() -> Vec<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("Media", &all_extensions())
        .add_filter("Video", &VIDEO_EXTENSIONS)
        .add_filter("Audio", &AUDIO_EXTENSIONS)
        .add_filter("Image", &IMAGE_EXTENSIONS)
        .pick_files()
        .unwrap_or_default()
}

/// Probes one file into a pool item. Unreadable or unsupported files are
/// skipped rather than surfaced as errors — the pool simply won't list them.
pub fn import(path: PathBuf) -> Option<ImportedMedia> {
    let kind = MediaKind::from_extension(&path)?;
    let name = path.file_name()?.to_string_lossy().into_owned();
    let path_str = path.to_string_lossy().into_owned();

    // Stills have no timeline length of their own; they get a default insert
    // duration like every NLE.
    const STILL_SECONDS: f32 = 5.0;

    let Ok(info) = probe_media(&path_str) else {
        return (kind == MediaKind::Image).then(|| ImportedMedia {
            name,
            path,
            kind,
            seconds: STILL_SECONDS,
            fps: 30.0,
            has_audio: false,
            meta: "still".into(),
        });
    };

    let meta = if info.has_video && info.width > 0 {
        format!("{}×{} · {:.0}p", info.width, info.height, info.fps)
    } else if info.has_audio {
        "audio".to_string()
    } else {
        "unknown".to_string()
    };

    Some(ImportedMedia {
        name,
        path,
        seconds: if kind == MediaKind::Image {
            STILL_SECONDS
        } else {
            info.duration_seconds as f32
        },
        kind: if info.has_video { kind } else { MediaKind::Audio },
        fps: info.fps,
        has_audio: info.has_audio,
        meta,
    })
}

pub fn format_duration(seconds: f32) -> String {
    let total = seconds.max(0.0).round() as i32;
    format!("{:02}:{:02}", total / 60, total % 60)
}
