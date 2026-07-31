//! Conversion between the studio's runtime timeline and the on-disk snapshot.

use crate::ai_tooling::chat::{Message, Role};
use crate::views::studio::timeline_panel::{Clip, TimelineState, Track};
use crate::workspace::{
    ChatHistory, ChatMessage, ClipKind as DiskClipKind, ClipSnapshot, ProjectContext,
    TimelineSnapshot, TrackKind as DiskTrackKind, TrackSnapshot,
};
use crate::ui::components::timeline::clip::ClipKind;
use crate::ui::components::timeline::headers::TrackKind;

/// Runtime timeline → on-disk snapshot. Sources inside the project folder are
/// stored relative to it so the folder stays portable.
pub fn to_snapshot(state: &TimelineState, ctx: &ProjectContext) -> TimelineSnapshot {
    TimelineSnapshot {
        tracks: state.tracks.iter().map(|t| track_to_disk(t, ctx)).collect(),
        playhead_seconds: state.playhead,
        zoom: state.zoom,
    }
}

/// On-disk snapshot → runtime timeline. Absent tracks fall back to the default
/// V1/A1 pair so a project always has somewhere to drop media.
pub fn apply_snapshot(state: &mut TimelineState, snapshot: &TimelineSnapshot, ctx: &ProjectContext) {
    if snapshot.tracks.is_empty() {
        *state = TimelineState::default();
        return;
    }

    state.tracks = snapshot.tracks.iter().map(|t| track_from_disk(t, ctx)).collect();
    state.playhead = snapshot.playhead_seconds;
    state.zoom = if snapshot.zoom > 0.0 { snapshot.zoom } else { 1.0 };
    state.selected_track = 0;
    state.fit_span();
}

fn track_to_disk(track: &Track, ctx: &ProjectContext) -> TrackSnapshot {
    TrackSnapshot {
        name: track.name.clone(),
        kind: match track.kind {
            TrackKind::Video => DiskTrackKind::Video,
            TrackKind::Audio => DiskTrackKind::Audio,
        },
        muted: track.muted,
        locked: track.locked,
        clips: track
            .clips
            .iter()
            .map(|clip| ClipSnapshot {
                id: clip.id,
                label: clip.label.clone(),
                source: clip.path.as_ref().map(|p| ctx.relativize(p)),
                kind: match clip.kind {
                    ClipKind::ARoll => DiskClipKind::ARoll,
                    ClipKind::BRoll => DiskClipKind::BRoll,
                    ClipKind::Audio => DiskClipKind::Audio,
                },
                start_seconds: clip.start,
                duration_seconds: clip.len,
                trim_in_seconds: clip.trim_in,
                source_seconds: clip.source_seconds,
                has_audio: clip.has_audio,
                gain: 1.0,
            })
            .collect(),
    }
}

fn track_from_disk(track: &TrackSnapshot, ctx: &ProjectContext) -> Track {
    Track {
        name: track.name.clone(),
        kind: match track.kind {
            DiskTrackKind::Video => TrackKind::Video,
            DiskTrackKind::Audio => TrackKind::Audio,
        },
        muted: track.muted,
        locked: track.locked,
        clips: track
            .clips
            .iter()
            .map(|clip| Clip {
                id: clip.id,
                label: clip.label.clone(),
                kind: match clip.kind {
                    DiskClipKind::ARoll => ClipKind::ARoll,
                    DiskClipKind::BRoll => ClipKind::BRoll,
                    DiskClipKind::Audio => ClipKind::Audio,
                },
                start: clip.start_seconds,
                len: clip.duration_seconds,
                selected: false,
                path: clip.source.as_ref().map(|p| ctx.resolve(p)),
                trim_in: clip.trim_in_seconds,
                has_audio: clip.has_audio,
                source_seconds: if clip.source_seconds > 0.0 {
                    clip.source_seconds
                } else {
                    clip.duration_seconds
                },
            })
            .collect(),
    }
}

/// Conversation → durable history. The system prompt is a constant the panel
/// re-applies on load, so it is not stored.
pub fn chat_to_disk(messages: &[Message]) -> ChatHistory {
    ChatHistory {
        messages: messages
            .iter()
            .filter_map(|message| match message.role {
                Role::User => Some(ChatMessage::user(message.content.clone())),
                Role::Assistant => Some(ChatMessage::assistant(message.content.clone())),
                Role::System => None,
            })
            .collect(),
    }
}

pub fn chat_from_disk(history: &ChatHistory) -> Vec<Message> {
    history
        .messages
        .iter()
        .map(|message| {
            if message.from_user {
                Message::user(message.text.clone())
            } else {
                Message::assistant(message.text.clone())
            }
        })
        .collect()
}
