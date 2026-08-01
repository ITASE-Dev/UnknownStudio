//! Implements [`EditorState`] over the studio's live state.
//!
//! Borrows both halves of the editor for the duration of one batch, so the
//! dispatcher can mutate the timeline and read the pool without either escaping.

use crate::ai_tooling::orchestration::{
    apply_actions_with_worker, ActionCommand, AsyncJob, DispatchReport, DispatcherError,
    EditorState, Marker,
};
use crate::views::studio::dnd::DragAsset;
use crate::views::studio::media_panel::{Asset, MediaState};
use crate::views::studio::timeline_panel::{TimelineMarker, TimelineState};
use std::sync::mpsc::Sender;

/// Mutable view of the editor for one dispatch batch.
pub struct StudioEditor<'a> {
    pub timeline: &'a mut TimelineState,
    pub pool: &'a MediaState,
}

impl<'a> StudioEditor<'a> {
    pub fn new(timeline: &'a mut TimelineState, pool: &'a MediaState) -> Self {
        Self { timeline, pool }
    }

    /// The model refers to clips as `c12`; ids on the timeline are numeric.
    fn parse_id(clip_id: &str) -> Option<u64> {
        clip_id.trim_start_matches(['c', 'C']).parse().ok()
    }

    fn resolve(&self, clip_id: &str) -> Result<u64, DispatcherError> {
        Self::parse_id(clip_id)
            .filter(|id| self.timeline.clip(*id).is_some())
            .ok_or_else(|| DispatcherError::UnknownClip(clip_id.to_string()))
    }

    fn asset(&self, name: &str) -> Option<&Asset> {
        self.pool.assets.iter().find(|asset| asset.name == name)
    }
}

impl EditorState for StudioEditor<'_> {
    fn track_count(&self) -> usize {
        self.timeline.tracks.len()
    }

    fn clip_span(&self, clip_id: &str) -> Option<(f32, f32)> {
        let id = Self::parse_id(clip_id)?;
        self.timeline
            .clip(id)
            .map(|clip| (clip.start, clip.start + clip.len))
    }

    fn clip_source(&self, clip_id: &str) -> Option<String> {
        let id = Self::parse_id(clip_id)?;
        self.timeline
            .clip(id)
            .and_then(|clip| clip.path.as_ref())
            .map(|path| path.to_string_lossy().into_owned())
    }

    fn has_asset(&self, name: &str) -> bool {
        self.asset(name).is_some()
    }

    fn add_marker(&mut self, marker: Marker) -> Result<(), DispatcherError> {
        self.timeline.markers.push(TimelineMarker {
            seconds: marker.time_sec,
            label: marker.label,
            color: marker.color,
        });
        self.timeline
            .markers
            .sort_by(|a, b| a.seconds.total_cmp(&b.seconds));
        Ok(())
    }

    fn split_clip(&mut self, clip_id: &str, time_sec: f32) -> Result<(), DispatcherError> {
        let id = self.resolve(clip_id)?;
        self.timeline
            .split_clip(id, time_sec)
            .then_some(())
            .ok_or_else(|| DispatcherError::Rejected(format!("clip {clip_id} could not be split")))
    }

    fn delete_clip(&mut self, clip_id: &str) -> Result<(), DispatcherError> {
        let id = self.resolve(clip_id)?;
        self.timeline.remove_clip(id);
        Ok(())
    }

    fn trim_clip(
        &mut self,
        clip_id: &str,
        start_sec: f32,
        end_sec: f32,
    ) -> Result<(), DispatcherError> {
        let id = self.resolve(clip_id)?;
        self.timeline
            .trim_clip(id, start_sec, end_sec)
            .then_some(())
            .ok_or_else(|| {
                DispatcherError::Rejected(format!(
                    "{clip_id} cannot be trimmed to {start_sec:.2}-{end_sec:.2} of its source"
                ))
            })
    }

    fn move_clip(
        &mut self,
        clip_id: &str,
        track_idx: usize,
        time_sec: f32,
    ) -> Result<(), DispatcherError> {
        let id = self.resolve(clip_id)?;
        self.timeline
            .move_clip_to_track(id, track_idx, time_sec)
            .then_some(())
            .ok_or_else(|| {
                DispatcherError::Rejected(format!(
                    "track {track_idx} will not take {clip_id} (locked, or wrong kind)"
                ))
            })
    }

    fn set_playhead(&mut self, time_sec: f32) {
        self.timeline.playhead = time_sec.clamp(0.0, self.timeline.seconds);
    }

    fn place_asset(
        &mut self,
        asset: &str,
        track_idx: usize,
        time_sec: f32,
    ) -> Result<(), DispatcherError> {
        let payload: DragAsset = self
            .asset(asset)
            .ok_or_else(|| DispatcherError::UnknownAsset(asset.to_string()))?
            .drag_payload();

        let accepts = self
            .timeline
            .tracks
            .get(track_idx)
            .is_some_and(|track| track.accepts(&payload));
        if !accepts {
            return Err(DispatcherError::Rejected(format!(
                "track {track_idx} will not take “{asset}” (locked, or wrong kind)"
            )));
        }

        self.timeline.place(track_idx, &payload, time_sec);
        Ok(())
    }
}

/// Applies a batch to the studio, forwarding heavy work to the background
/// engine. Returns the report so the caller can feed failures back to the model.
pub fn dispatch(
    actions: Vec<ActionCommand>,
    timeline: &mut TimelineState,
    pool: &MediaState,
    worker: &Sender<AsyncJob>,
) -> DispatchReport {
    let mut editor = StudioEditor::new(timeline, pool);
    apply_actions_with_worker(actions, &mut editor, worker)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::components::timeline::clip::ClipKind;
    use std::sync::mpsc;

    fn asset(name: &str, seconds: f32) -> DragAsset {
        DragAsset {
            name: name.into(),
            path: None,
            seconds,
            kind: ClipKind::ARoll,
            has_audio: false,
        }
    }

    fn timeline_with_clip() -> (TimelineState, u64) {
        let mut timeline = TimelineState::default();
        timeline.place(0, &asset("intro.mp4", 10.0), 0.0);
        let id = timeline.tracks[0].clips[0].id;
        (timeline, id)
    }

    #[test]
    fn the_model_can_split_a_real_clip_by_its_id() {
        let (mut timeline, id) = timeline_with_clip();
        let pool = MediaState::default();
        let (tx, _rx) = mpsc::channel();

        let report = dispatch(
            vec![ActionCommand::SplitClip {
                clip_id: format!("c{id}"),
                time_sec: 4.0,
            }],
            &mut timeline,
            &pool,
            &tx,
        );

        assert!(!report.had_failures(), "{}", report.feedback());
        assert_eq!(timeline.tracks[0].clips.len(), 2);
        assert_eq!(timeline.tracks[0].clips[0].len, 4.0);
        assert_eq!(timeline.tracks[0].clips[1].trim_in, 4.0);
    }

    #[test]
    fn a_marker_reaches_the_timeline_in_time_order() {
        let (mut timeline, _) = timeline_with_clip();
        let pool = MediaState::default();
        let (tx, _rx) = mpsc::channel();

        dispatch(
            vec![
                ActionCommand::AddMarker {
                    time_sec: 6.0,
                    color: "red".into(),
                    label: "late".into(),
                },
                ActionCommand::AddMarker {
                    time_sec: 2.0,
                    color: "blue".into(),
                    label: "early".into(),
                },
            ],
            &mut timeline,
            &pool,
            &tx,
        );

        let labels: Vec<&str> = timeline
            .markers
            .iter()
            .map(|marker| marker.label.as_str())
            .collect();
        assert_eq!(labels, vec!["early", "late"]);
    }

    #[test]
    fn an_unknown_id_is_reported_and_the_timeline_is_untouched() {
        let (mut timeline, _) = timeline_with_clip();
        let pool = MediaState::default();
        let (tx, _rx) = mpsc::channel();

        let report = dispatch(
            vec![ActionCommand::DeleteClip {
                clip_id: "c9999".into(),
            }],
            &mut timeline,
            &pool,
            &tx,
        );

        assert!(report.had_failures());
        assert!(report.feedback().contains("no clip with id c9999"));
        assert_eq!(timeline.tracks[0].clips.len(), 1);
    }

    #[test]
    fn placing_an_asset_on_the_wrong_lane_is_refused() {
        let (mut timeline, _) = timeline_with_clip();
        let mut pool = MediaState::default();
        pool.assets.push(Asset::demo(
            "b_roll.mp4",
            "1920×1080",
            4.0,
            crate::media::MediaKind::Video,
            false,
        ));
        let (tx, _rx) = mpsc::channel();

        // Track 1 is the audio lane.
        let report = dispatch(
            vec![ActionCommand::PlaceAsset {
                asset: "b_roll.mp4".into(),
                target_track_idx: 1,
                target_time_sec: 0.0,
            }],
            &mut timeline,
            &pool,
            &tx,
        );

        assert!(report.had_failures());
        assert!(report.feedback().contains("will not take"));
        assert!(timeline.tracks[1].clips.is_empty());
    }

    #[test]
    fn trimming_shortens_the_clip_and_keeps_the_track_gapless() {
        let (mut timeline, id) = timeline_with_clip();
        timeline.place(0, &asset("second.mp4", 5.0), 100.0);
        let pool = MediaState::default();
        let (tx, _rx) = mpsc::channel();

        let report = dispatch(
            vec![ActionCommand::TrimClip {
                clip_id: format!("c{id}"),
                start_sec: 2.0,
                end_sec: 6.0,
            }],
            &mut timeline,
            &pool,
            &tx,
        );

        assert!(!report.had_failures(), "{}", report.feedback());
        let clip = timeline.clip(id).expect("clip");
        assert_eq!(clip.len, 4.0);
        assert_eq!(clip.trim_in, 2.0);
        // The clip after it closes up rather than leaving a hole.
        assert_eq!(timeline.tracks[0].clips[1].start, 4.0);
    }

    #[test]
    fn heavy_work_leaves_the_timeline_alone_and_reaches_the_worker() {
        let (mut timeline, id) = timeline_with_clip();
        let pool = MediaState::default();
        let (tx, rx) = mpsc::channel();

        let report = dispatch(
            vec![ActionCommand::Export {
                preset: "youtube_1080p".into(),
            }],
            &mut timeline,
            &pool,
            &tx,
        );

        assert!(report.applied.is_empty());
        assert_eq!(timeline.tracks[0].clips.len(), 1);
        assert!(matches!(rx.recv(), Ok(AsyncJob::Export { .. })));
        assert!(timeline.clip(id).is_some());
    }
}
