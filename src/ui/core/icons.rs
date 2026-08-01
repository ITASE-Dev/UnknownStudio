//! Icon vocabulary, drawn from the Phosphor font registered in `theme::apply`.
//!
//! One name per meaning, so a glyph is never chosen twice for two ideas. The
//! hand-painted [`super::buttons::Icon`] set stays for transport controls,
//! which must render even without font coverage.

use egui_phosphor::regular as ph;

// Timeline tools — each maps to one `action_engine::tools` operation.
/// Bake the clip's in/out points into a new file.
pub const TRIM: &str = ph::SCISSORS;
/// Change playback rate.
pub const SPEED: &str = ph::FAST_FORWARD;
/// Reframe / rescale (e.g. 16:9 → 9:16).
pub const CROP: &str = ph::CROP;
/// Composite one clip over another.
pub const OVERLAY: &str = ph::STACK_SIMPLE;
/// Join clips into one file.
pub const CONCAT: &str = ph::ARROWS_MERGE;
/// Pull the audio track out to its own file.
pub const EXTRACT_AUDIO: &str = ph::MUSIC_NOTE;
/// Strip audio from a clip.
pub const MUTE_AUDIO: &str = ph::SPEAKER_SIMPLE_SLASH;

// Track management.
pub const ADD_VIDEO_TRACK: &str = ph::FILM_STRIP;
pub const ADD_AUDIO_TRACK: &str = ph::WAVEFORM;
pub const MOVE_UP: &str = ph::ARROW_LINE_UP;
pub const MOVE_DOWN: &str = ph::ARROW_LINE_DOWN;
pub const REMOVE_TRACK: &str = ph::X;

// Context menu.
/// Hand the selection to the director.
pub const SEND_TO_CHAT: &str = ph::CHAT_CIRCLE_TEXT;
pub const PROPERTIES: &str = ph::INFO;
pub const DELETE: &str = ph::TRASH;
pub const CANCEL: &str = ph::X_CIRCLE;

// Settings.
pub const SETTINGS: &str = ph::GEAR;
pub const CREDENTIALS: &str = ph::KEY;
pub const SHOW_SECRET: &str = ph::EYE;
pub const HIDE_SECRET: &str = ph::EYE_SLASH;

// Status.
pub const BUSY: &str = ph::CIRCLE_NOTCH;
pub const DONE: &str = ph::CHECK_CIRCLE;
pub const FAILED: &str = ph::WARNING_CIRCLE;

/// A glyph paired with a `+`, for "add one of these".
pub fn plus(glyph: &str) -> String {
    format!("{}{glyph}", ph::PLUS)
}
