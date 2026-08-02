//! Read-only projection of the studio timeline.
//!
//! The diff engine needs to ask structural questions of a *populated* edit —
//! what covers this second, is there already a cutaway here, where are the
//! cuts — without depending on the editor's types. The studio builds one of
//! these each time a comparison runs; nothing here can mutate anything.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackRole {
    Video,
    Audio,
}

/// What a clip contributes, which decides whether it counts as coverage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipRole {
    /// The main performance — a talking head, usually.
    ARoll,
    /// A cutaway already in place.
    BRoll,
    Audio,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipView {
    pub id: u64,
    pub label: String,
    pub start_sec: f32,
    pub end_sec: f32,
    pub role: ClipRole,
}

impl ClipView {
    pub fn duration_sec(&self) -> f32 {
        (self.end_sec - self.start_sec).max(0.0)
    }

    pub fn covers(&self, seconds: f32) -> bool {
        seconds >= self.start_sec && seconds < self.end_sec
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackView {
    pub index: usize,
    pub name: String,
    pub role: TrackRole,
    pub locked: bool,
    pub clips: Vec<ClipView>,
}

/// The user's edit as the engine sees it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CurrentTimelineState {
    pub tracks: Vec<TrackView>,
    /// Captions already on the timeline, as `[start, end)` spans.
    pub caption_spans: Vec<(f32, f32)>,
    pub duration_sec: f32,
}

impl CurrentTimelineState {
    pub fn is_empty(&self) -> bool {
        self.tracks.iter().all(|track| track.clips.is_empty())
    }

    pub fn clip_count(&self) -> usize {
        self.tracks.iter().map(|track| track.clips.len()).sum()
    }

    pub fn video_tracks(&self) -> impl Iterator<Item = &TrackView> {
        self.tracks.iter().filter(|t| t.role == TrackRole::Video)
    }

    pub fn audio_tracks(&self) -> impl Iterator<Item = &TrackView> {
        self.tracks.iter().filter(|t| t.role == TrackRole::Audio)
    }

    /// Whether a cutaway already covers `seconds` on any video track.
    pub fn has_broll_at(&self, seconds: f32) -> bool {
        self.video_tracks().any(|track| {
            track
                .clips
                .iter()
                .any(|clip| clip.role == ClipRole::BRoll && clip.covers(seconds))
        })
    }

    /// The A-roll shot playing at `seconds`, if any.
    pub fn aroll_at(&self, seconds: f32) -> Option<&ClipView> {
        self.video_tracks().find_map(|track| {
            track
                .clips
                .iter()
                .find(|clip| clip.role == ClipRole::ARoll && clip.covers(seconds))
        })
    }

    /// Whether an audio element starts within `tolerance` of `seconds` — how
    /// the engine tells an existing SFX from a missing one.
    pub fn has_audio_event_near(&self, seconds: f32, tolerance: f32) -> bool {
        self.audio_tracks().any(|track| {
            track
                .clips
                .iter()
                .any(|clip| (clip.start_sec - seconds).abs() <= tolerance)
        })
    }

    /// Every visible cut on the video tracks, in time order.
    pub fn cut_times(&self) -> Vec<f32> {
        let mut cuts: Vec<f32> = self
            .video_tracks()
            .flat_map(|track| track.clips.iter().map(|clip| clip.start_sec))
            .filter(|start| *start > 0.0)
            .collect();
        cuts.sort_by(f32::total_cmp);
        cuts.dedup_by(|a, b| (*a - *b).abs() < 0.01);
        cuts
    }

    pub fn cuts_per_minute(&self) -> f32 {
        if self.duration_sec <= 0.0 {
            return 0.0;
        }
        self.cut_times().len() as f32 / (self.duration_sec / 60.0)
    }

    /// Share of the runtime already covered by cutaways, 0.0..=1.0.
    pub fn broll_coverage(&self) -> f32 {
        if self.duration_sec <= 0.0 {
            return 0.0;
        }
        let covered: f32 = self
            .video_tracks()
            .flat_map(|track| track.clips.iter())
            .filter(|clip| clip.role == ClipRole::BRoll)
            .map(ClipView::duration_sec)
            .sum();
        (covered / self.duration_sec).clamp(0.0, 1.0)
    }

    /// How long the shot at `seconds` has been held without a cut. A large
    /// value is the structural signature of a static talking head.
    pub fn static_hold_sec(&self, seconds: f32) -> f32 {
        self.aroll_at(seconds)
            .map(ClipView::duration_sec)
            .unwrap_or(0.0)
    }

    /// A video track that can take an overlay: unlocked, and free at `seconds`.
    /// Prefers a track that already exists over asking for a new one.
    pub fn free_video_track_at(&self, seconds: f32, duration: f32) -> Option<usize> {
        let end = seconds + duration;
        self.video_tracks()
            .filter(|track| !track.locked)
            .find(|track| {
                track
                    .clips
                    .iter()
                    .all(|clip| clip.end_sec <= seconds || clip.start_sec >= end)
            })
            .map(|track| track.index)
    }

    pub fn first_free_audio_track(&self) -> Option<usize> {
        self.audio_tracks()
            .find(|track| !track.locked)
            .map(|track| track.index)
    }

    /// Maps a time on the competitor's timeline onto this one, by proportion.
    ///
    /// Two videos of different lengths still share a narrative shape — a beat
    /// 20% of the way in lands at 20% here. This is what lets a 12-minute
    /// reference inform a 4-minute edit.
    pub fn map_narrative_time(&self, competitor_time: f32, competitor_duration: f32) -> f32 {
        if competitor_duration <= 0.0 {
            return 0.0;
        }
        let ratio = (competitor_time / competitor_duration).clamp(0.0, 1.0);
        (ratio * self.duration_sec).clamp(0.0, self.duration_sec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(id: u64, start: f32, end: f32, role: ClipRole) -> ClipView {
        ClipView {
            id,
            label: format!("clip{id}"),
            start_sec: start,
            end_sec: end,
            role,
        }
    }

    /// 60s edit: one long A-roll, a cutaway at 20–24s, one audio track.
    fn populated() -> CurrentTimelineState {
        CurrentTimelineState {
            tracks: vec![
                TrackView {
                    index: 0,
                    name: "V1".into(),
                    role: TrackRole::Video,
                    locked: false,
                    clips: vec![clip(1, 0.0, 30.0, ClipRole::ARoll), clip(2, 30.0, 60.0, ClipRole::ARoll)],
                },
                TrackView {
                    index: 1,
                    name: "V2".into(),
                    role: TrackRole::Video,
                    locked: false,
                    clips: vec![clip(3, 20.0, 24.0, ClipRole::BRoll)],
                },
                TrackView {
                    index: 2,
                    name: "A1".into(),
                    role: TrackRole::Audio,
                    locked: false,
                    clips: vec![clip(4, 5.0, 6.0, ClipRole::Audio)],
                },
            ],
            caption_spans: Vec::new(),
            duration_sec: 60.0,
        }
    }

    #[test]
    fn existing_coverage_is_recognised_so_it_is_not_proposed_twice() {
        let state = populated();
        assert!(state.has_broll_at(22.0), "the cutaway covers this second");
        assert!(!state.has_broll_at(10.0), "nothing covers this one");
        assert!(!state.has_broll_at(24.0), "the end of a span is exclusive");
    }

    #[test]
    fn a_long_unbroken_shot_reads_as_a_static_hold() {
        let state = populated();
        assert_eq!(state.static_hold_sec(10.0), 30.0);
        assert_eq!(state.static_hold_sec(120.0), 0.0, "past the end");
    }

    #[test]
    fn an_existing_sfx_nearby_counts_as_already_handled() {
        let state = populated();
        assert!(state.has_audio_event_near(5.3, 0.5));
        assert!(!state.has_audio_event_near(40.0, 0.5));
    }

    #[test]
    fn cuts_are_deduplicated_across_tracks_and_exclude_the_start() {
        let state = populated();
        // 30.0 from V1 and 20.0 from V2; the clip at 0.0 is not a cut.
        assert_eq!(state.cut_times(), vec![20.0, 30.0]);
        assert_eq!(state.cuts_per_minute(), 2.0);
    }

    #[test]
    fn coverage_counts_only_cutaways() {
        let state = populated();
        // 4 seconds of B-roll in 60.
        assert!((state.broll_coverage() - 4.0 / 60.0).abs() < 1e-5);
    }

    #[test]
    fn an_overlay_lands_on_a_track_that_is_actually_free() {
        let state = populated();
        // V2 is busy 20–24, so a 4s overlay at 21 cannot go there; V1 is busy
        // for the whole runtime, so there is nowhere.
        assert_eq!(state.free_video_track_at(21.0, 4.0), None);
        // At 40s V2 is free.
        assert_eq!(state.free_video_track_at(40.0, 4.0), Some(1));
    }

    #[test]
    fn a_locked_track_is_never_offered() {
        let mut state = populated();
        state.tracks[1].locked = true;
        assert_eq!(state.free_video_track_at(40.0, 4.0), None);
    }

    #[test]
    fn narrative_time_scales_between_videos_of_different_lengths() {
        let state = populated();
        // A beat a quarter of the way through a 600s reference.
        assert_eq!(state.map_narrative_time(150.0, 600.0), 15.0);
        // Out of range input is clamped rather than producing a time past the end.
        assert_eq!(state.map_narrative_time(900.0, 600.0), 60.0);
        assert_eq!(state.map_narrative_time(10.0, 0.0), 0.0);
    }

    #[test]
    fn an_empty_timeline_answers_every_query_without_panicking() {
        let state = CurrentTimelineState::default();
        assert!(state.is_empty());
        assert_eq!(state.clip_count(), 0);
        assert_eq!(state.broll_coverage(), 0.0);
        assert_eq!(state.cuts_per_minute(), 0.0);
        assert!(!state.has_broll_at(5.0));
        assert_eq!(state.free_video_track_at(0.0, 1.0), None);
    }
}
