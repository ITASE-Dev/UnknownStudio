//! Runtime editor state → the LLM-facing DTOs.
//!
//! The conversion lives here, in the binary, so `ai_tooling` stays free of any
//! knowledge of the UI's own structures.

use crate::ai_tooling::orchestration::{
    AssetContext, ClipContext, MediaPoolContext, SelectionContext, TimelineContext, TrackContext,
    TrackKind as DtoTrackKind,
};
use crate::media::MediaKind;
use crate::models::MediaSelection;
use crate::ui::components::timeline::headers::TrackKind;
use crate::views::studio::media_panel::MediaState;
use crate::views::studio::timeline_panel::TimelineState;

pub fn timeline_context(state: &TimelineState) -> TimelineContext {
    TimelineContext {
        duration: state.content_end(),
        playhead: state.playhead,
        tracks: state
            .tracks
            .iter()
            .enumerate()
            .map(|(index, track)| TrackContext {
                index,
                name: track.name.clone(),
                kind: match track.kind {
                    TrackKind::Video => DtoTrackKind::Video,
                    TrackKind::Audio => DtoTrackKind::Audio,
                },
                muted: track.muted,
                locked: track.locked,
                clips: track
                    .clips
                    .iter()
                    .map(|clip| ClipContext {
                        id: clip.id,
                        name: clip.label.clone(),
                        start: clip.start,
                        end: clip.start + clip.len,
                        has_audio: clip.has_audio,
                    })
                    .collect(),
            })
            .collect(),
    }
}

pub fn pool_context(state: &MediaState) -> MediaPoolContext {
    MediaPoolContext {
        assets: state
            .assets
            .iter()
            .map(|asset| AssetContext {
                name: asset.name.clone(),
                seconds: asset.seconds,
                kind: match asset.kind {
                    MediaKind::Audio => DtoTrackKind::Audio,
                    _ => DtoTrackKind::Video,
                },
                generated: asset.generated,
            })
            .collect(),
    }
}

pub fn selection_context(selection: &MediaSelection) -> SelectionContext {
    let kind = match selection {
        MediaSelection::Clip { .. } => "clip",
        MediaSelection::AudioTrack { .. } => "track",
        MediaSelection::PreviewScreen { .. } => "monitor",
        MediaSelection::PoolAsset { .. } => "asset",
    };

    SelectionContext {
        kind: kind.to_string(),
        description: selection.title(),
        clip_id: selection.clip_id(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::views::studio::dnd::DragAsset;
    use crate::ui::components::timeline::clip::ClipKind;

    #[test]
    fn a_placed_clip_reaches_the_prompt_with_its_times() {
        let mut timeline = TimelineState::default();
        timeline.place(
            0,
            &DragAsset {
                name: "intro.mp4".into(),
                path: None,
                seconds: 6.0,
                kind: ClipKind::ARoll,
                has_audio: true,
            },
            0.0,
        );

        let context = timeline_context(&timeline);
        assert_eq!(context.clip_count(), 1);
        assert_eq!(context.duration, 6.0);

        let rendered = context.to_llm_context_string();
        assert!(rendered.contains("\"intro.mp4\" 0.0-6.0 a"));
        assert!(rendered.contains("t0 V1 video"));
        assert!(rendered.contains("t1 A1 audio"));
    }

    #[test]
    fn selections_map_to_their_context_kind() {
        let clip = MediaSelection::Clip {
            id: 3,
            label: "a.mp4".into(),
            track: "V1".into(),
            start_seconds: 0.0,
            duration_seconds: 2.0,
        };
        let context = selection_context(&clip);
        assert_eq!(context.kind, "clip");
        assert_eq!(context.clip_id, Some(3));

        let monitor = selection_context(&MediaSelection::PreviewScreen { seconds: 1.0 });
        assert_eq!(monitor.kind, "monitor");
        assert_eq!(monitor.clip_id, None);
    }
}
