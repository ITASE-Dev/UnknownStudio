//! Pure timeline mutation planning and mechanics. No I/O, no host types — the
//! host adapts its own clip struct through `EditableClip`.

use crate::action_engine::types::{ClipId, ClipSnapshot};
use uuid::Uuid;

/// Half-open `[start, end)` range of absolute timeline frames.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FrameRange {
    pub start: u64,
    pub end: u64,
}

impl FrameRange {
    pub fn new(start: u64, end: u64) -> Self {
        Self { start, end }
    }

    pub fn len(&self) -> u64 {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Clone, Debug)]
pub enum TimelineEdit {
    /// Head trim: both the timeline start and the source trim-in advance, so the
    /// remaining section keeps showing the same source frames.
    TrimHead {
        clip_id: ClipId,
        new_start_frame: u64,
        new_duration_frames: u64,
    },
    /// Silence regions to lift out of the clip entirely.
    CutRanges {
        clip_id: ClipId,
        ranges: Vec<FrameRange>,
    },
    /// Declarative filter chain; pixels are never rewritten.
    SetVideoFilter {
        clip_id: ClipId,
        filter_spec: String,
    },
}

/// Legacy heuristic used when no silence data is available: drop the first 10%
/// ("dead air") while keeping the end frame fixed.
pub fn plan_head_trim(clip: &ClipSnapshot) -> TimelineEdit {
    const HEAD_TRIM_RATIO: f64 = 0.10;
    let trimmed = ((clip.duration_frames as f64) * HEAD_TRIM_RATIO).round() as u64;
    let new_duration_frames = clip.duration_frames.saturating_sub(trimmed).max(1);
    TimelineEdit::TrimHead {
        clip_id: clip.id,
        new_start_frame: clip.start_frame + (clip.duration_frames - new_duration_frames),
        new_duration_frames,
    }
}

/// Host-side clip adapter. Implement once for the studio's own clip type.
pub trait EditableClip: Clone {
    fn id(&self) -> ClipId;
    fn set_id(&mut self, id: ClipId);
    fn start_frame(&self) -> u64;
    fn set_start_frame(&mut self, frame: u64);
    fn duration_frames(&self) -> u64;
    fn set_duration_frames(&mut self, frames: u64);
    fn trim_in_frames(&self) -> u64;
    fn set_trim_in_frames(&mut self, frames: u64);

    fn end_frame(&self) -> u64 {
        self.start_frame().saturating_add(self.duration_frames())
    }
}

/// Applies `TimelineEdit::TrimHead` to a clip list.
pub fn apply_head_trim<C: EditableClip>(
    clips: &mut [C],
    clip_id: ClipId,
    new_start_frame: u64,
    new_duration_frames: u64,
) {
    let Some(clip) = clips.iter_mut().find(|c| c.id() == clip_id) else {
        return;
    };
    let removed = new_start_frame.saturating_sub(clip.start_frame());
    clip.set_start_frame(new_start_frame);
    clip.set_duration_frames(new_duration_frames);
    clip.set_trim_in_frames(clip.trim_in_frames() + removed);
}

/// Removes every range from the target clip.
///
/// Ranges are processed last-to-first so each cut only shrinks the left part
/// (which keeps `clip_id`); the right remainder always becomes a new clip. That
/// order keeps earlier ranges pointed at the right clip — the id never moves.
pub fn apply_cut_ranges<C: EditableClip>(
    clips: &mut Vec<C>,
    clip_id: ClipId,
    ranges: &[FrameRange],
) {
    let mut ordered: Vec<FrameRange> = ranges.iter().copied().filter(|r| !r.is_empty()).collect();
    ordered.sort_by(|a, b| b.start.cmp(&a.start));
    for range in ordered {
        cut_range_from_clip(clips, clip_id, range);
    }
}

fn cut_range_from_clip<C: EditableClip>(clips: &mut Vec<C>, clip_id: ClipId, range: FrameRange) {
    let Some(idx) = clips.iter().position(|c| c.id() == clip_id) else {
        return;
    };

    let clip = clips[idx].clone();
    let cut_start = range.start.max(clip.start_frame());
    let cut_end = range.end.min(clip.end_frame());
    if cut_end <= cut_start {
        return;
    }

    let left_len = cut_start - clip.start_frame();
    let right_len = clip.end_frame() - cut_end;

    if right_len > 0 {
        let mut right = clip.clone();
        right.set_id(Uuid::new_v4());
        right.set_start_frame(cut_end);
        right.set_duration_frames(right_len);
        right.set_trim_in_frames(clip.trim_in_frames() + (cut_end - clip.start_frame()));
        clips.insert(idx + 1, right);
    }

    if left_len > 0 {
        clips[idx].set_duration_frames(left_len);
    } else {
        clips.remove(idx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    struct TestClip {
        id: ClipId,
        start: u64,
        duration: u64,
        trim_in: u64,
    }

    impl EditableClip for TestClip {
        fn id(&self) -> ClipId {
            self.id
        }
        fn set_id(&mut self, id: ClipId) {
            self.id = id;
        }
        fn start_frame(&self) -> u64 {
            self.start
        }
        fn set_start_frame(&mut self, frame: u64) {
            self.start = frame;
        }
        fn duration_frames(&self) -> u64 {
            self.duration
        }
        fn set_duration_frames(&mut self, frames: u64) {
            self.duration = frames;
        }
        fn trim_in_frames(&self) -> u64 {
            self.trim_in
        }
        fn set_trim_in_frames(&mut self, frames: u64) {
            self.trim_in = frames;
        }
    }

    fn clip(id: ClipId) -> TestClip {
        TestClip {
            id,
            start: 100,
            duration: 100,
            trim_in: 10,
        }
    }

    #[test]
    fn cut_splits_clip_and_preserves_source_offsets() {
        let id = Uuid::new_v4();
        let mut clips = vec![clip(id)];
        apply_cut_ranges(&mut clips, id, &[FrameRange::new(140, 160)]);

        assert_eq!(clips.len(), 2);
        assert_eq!(clips[0].id, id);
        assert_eq!((clips[0].start, clips[0].duration, clips[0].trim_in), (100, 40, 10));
        assert_eq!((clips[1].start, clips[1].duration, clips[1].trim_in), (160, 40, 70));
    }

    #[test]
    fn multiple_cuts_keep_targeting_the_original_id() {
        let id = Uuid::new_v4();
        let mut clips = vec![clip(id)];
        apply_cut_ranges(
            &mut clips,
            id,
            &[FrameRange::new(110, 120), FrameRange::new(170, 180)],
        );

        assert_eq!(clips.len(), 3);
        assert_eq!(clips[0].id, id);
        assert_eq!(clips.iter().map(|c| c.duration).sum::<u64>(), 80);
    }

    #[test]
    fn cut_covering_whole_clip_removes_it() {
        let id = Uuid::new_v4();
        let mut clips = vec![clip(id)];
        apply_cut_ranges(&mut clips, id, &[FrameRange::new(0, 500)]);
        assert!(clips.is_empty());
    }

    #[test]
    fn head_trim_advances_start_and_trim_in_together() {
        let id = Uuid::new_v4();
        let mut clips = vec![clip(id)];
        let snapshot = ClipSnapshot {
            id,
            source_path: String::new(),
            start_frame: 100,
            trim_in_frames: 10,
            duration_frames: 100,
            fps: 30.0,
        };

        let TimelineEdit::TrimHead {
            new_start_frame,
            new_duration_frames,
            ..
        } = plan_head_trim(&snapshot)
        else {
            panic!("expected TrimHead");
        };
        apply_head_trim(&mut clips, id, new_start_frame, new_duration_frames);

        assert_eq!((clips[0].start, clips[0].duration, clips[0].trim_in), (110, 90, 20));
    }
}
