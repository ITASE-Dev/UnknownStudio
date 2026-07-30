use crate::ui::components::timeline::clip::{clip_block, ClipKind};
use crate::ui::components::timeline::headers::{header_width, track_header_sized, TrackKind};
use crate::ui::components::timeline::markers::{
    playhead_marker, playhead_timecode, ripple_cut_marker,
};
use crate::media::{Segment, Textures};
use crate::ui::components::timeline::{
    clip_rect, px_per_sec_for, ruler, ruler_scrub, seconds_at, track_lane,
};
use crate::ui::core::buttons::{icon_button_painted, Icon};
use crate::ui::core::inputs::pro_slider;
use crate::ui::theme::tokens::*;
use crate::views::studio::dnd::{self, DragAsset};
use eframe::egui::{Align, Layout, Rect, RichText, ScrollArea, Ui, Vec2};
use std::path::PathBuf;

pub struct Clip {
    pub label: String,
    pub kind: ClipKind,
    pub start: f32,
    pub len: f32,
    pub selected: bool,
    /// Source file for clips placed from the media pool; `None` for demo blocks.
    pub path: Option<PathBuf>,
    /// Offset into the source where this clip starts (grows when split).
    pub trim_in: f32,
}

impl Clip {
    fn new(label: &str, kind: ClipKind, start: f32, len: f32) -> Self {
        Self {
            label: label.into(),
            kind,
            start,
            len,
            selected: false,
            path: None,
            trim_in: 0.0,
        }
    }

    fn from_asset(asset: &DragAsset, start: f32) -> Self {
        Self {
            label: asset.name.clone(),
            kind: asset.kind,
            start,
            len: asset.seconds,
            selected: false,
            path: asset.path.clone(),
            trim_in: 0.0,
        }
    }

    fn is_video(&self) -> bool {
        self.kind != ClipKind::Audio
    }
}

pub struct Track {
    pub name: String,
    pub kind: TrackKind,
    pub clips: Vec<Clip>,
    pub locked: bool,
    pub muted: bool,
}

impl Track {
    fn accepts(&self, asset: &DragAsset) -> bool {
        !self.locked
            && match self.kind {
                TrackKind::Audio => asset.is_audio(),
                TrackKind::Video => !asset.is_audio(),
            }
    }

    /// Where a clip dropped at `seconds` ends up on a gapless track: the index
    /// it takes in the order, and the start that index implies. A drop past the
    /// last clip appends; anything else pushes the rest to the right.
    fn insert_position(&self, seconds: f32) -> (usize, f32) {
        let index = self
            .clips
            .iter()
            .take_while(|c| c.start + c.len * 0.5 < seconds)
            .count();
        let start = self.clips[..index].iter().map(|c| c.len).sum();
        (index, start)
    }

    /// Magnetic track: order by start time, then butt every clip against its
    /// predecessor so the track holds no gaps.
    fn repack(&mut self) {
        self.clips.sort_by(|a, b| a.start.total_cmp(&b.start));
        let mut cursor = 0.0;
        for clip in self.clips.iter_mut() {
            clip.start = cursor;
            cursor += clip.len;
        }
    }

}

pub struct TimelineState {
    pub tracks: Vec<Track>,
    pub seconds: f32,
    pub zoom: f32,
    pub playhead: f32,
    pub ripple_cuts: Vec<f32>,
}

impl Default for TimelineState {
    fn default() -> Self {
        Self {
            tracks: vec![
                Track {
                    name: "V1".into(),
                    kind: TrackKind::Video,
                    locked: false,
                    muted: false,
                    // Starts empty: every block on the timeline is real media
                    // the user placed, so the monitor always matches it.
                    clips: Vec::new(),
                },
                Track {
                    name: "A1".into(),
                    kind: TrackKind::Audio,
                    locked: false,
                    muted: false,
                    clips: Vec::new(),
                },
            ],
            seconds: 24.0,
            zoom: 1.0,
            playhead: 0.0,
            ripple_cuts: Vec::new(),
        }
    }
}

impl TimelineState {
    pub fn clip_count(&self) -> usize {
        self.tracks.iter().map(|t| t.clips.len()).sum()
    }

    pub fn timecode(&self) -> String {
        let s = self.playhead;
        format!(
            "{:02}:{:02}:{:02}",
            (s as i32) / 60,
            (s as i32) % 60,
            ((s.fract() * 30.0) as i32).min(29)
        )
    }

    /// Places a dropped asset at the drop position, then closes every gap on
    /// that track: the drop point decides the *order*, not the empty space.
    pub fn place(&mut self, track_idx: usize, asset: &DragAsset, seconds: f32) {
        let Some(track) = self.tracks.get_mut(track_idx) else {
            return;
        };
        let (index, start) = track.insert_position(seconds);
        for clip in track.clips.iter_mut() {
            clip.selected = false;
        }
        let mut clip = Clip::from_asset(asset, start);
        clip.selected = true;
        track.clips.insert(index, clip);
        track.repack();

        self.seconds = self.seconds.max(self.content_end() + 2.0);
    }

    /// Re-packs one track (after a move) and keeps the visible span honest.
    pub fn repack_track(&mut self, track_idx: usize) {
        if let Some(track) = self.tracks.get_mut(track_idx) {
            track.repack();
        }
        self.seconds = self.seconds.max(self.content_end() + 2.0);
    }

    /// Deletes the selection and closes the holes it leaves.
    pub fn delete_selected(&mut self) {
        for track in self.tracks.iter_mut().filter(|t| !t.locked) {
            let before = track.clips.len();
            track.clips.retain(|c| !c.selected);
            if track.clips.len() != before {
                track.repack();
            }
        }
    }

    /// Split the clip under the playhead on the first unlocked video track.
    pub fn split_at_playhead(&mut self) {
        let ph = self.playhead;
        for track in self.tracks.iter_mut().filter(|t| !t.locked) {
            if let Some(i) = track
                .clips
                .iter()
                .position(|c| ph > c.start + 0.05 && ph < c.start + c.len - 0.05)
            {
                let head_len = ph - track.clips[i].start;
                let mut tail = Clip {
                    label: track.clips[i].label.clone(),
                    kind: track.clips[i].kind,
                    start: ph,
                    len: track.clips[i].len - head_len,
                    selected: false,
                    path: track.clips[i].path.clone(),
                    // The tail keeps showing the same source frames.
                    trim_in: track.clips[i].trim_in + head_len,
                };
                tail.selected = track.clips[i].selected;
                track.clips[i].len = head_len;
                track.clips.insert(i + 1, tail);
                break;
            }
        }
    }

    /// Timeline end: the last clip boundary on any track.
    pub fn content_end(&self) -> f32 {
        self.tracks
            .iter()
            .flat_map(|t| t.clips.iter().map(|c| c.start + c.len))
            .fold(0.0, f32::max)
    }

    /// Flattens the video tracks into the program the decoder plays. Upper
    /// tracks win: a region already covered by a higher track is not revisited,
    /// so overlaps resolve the same way the monitor shows them.
    pub fn program(&self) -> Vec<Segment> {
        let mut program: Vec<Segment> = Vec::new();

        for track in self.tracks.iter().filter(|t| !t.muted) {
            for clip in track.clips.iter().filter(|c| c.is_video()) {
                let Some(path) = clip.path.clone() else {
                    continue;
                };
                for (start, end) in uncovered(&program, clip.start, clip.start + clip.len) {
                    program.push(Segment {
                        start,
                        end,
                        path: path.clone(),
                        media_start: clip.trim_in + (start - clip.start),
                    });
                }
            }
        }

        program.sort_by(|a, b| a.start.total_cmp(&b.start));
        program
    }
}

/// Parts of `[start, end)` not already claimed by an earlier segment.
fn uncovered(program: &[Segment], start: f32, end: f32) -> Vec<(f32, f32)> {
    let mut gaps = vec![(start, end)];
    for segment in program {
        let mut next = Vec::with_capacity(gaps.len());
        for (gap_start, gap_end) in gaps {
            if segment.end <= gap_start || segment.start >= gap_end {
                next.push((gap_start, gap_end));
                continue;
            }
            if gap_start < segment.start {
                next.push((gap_start, segment.start));
            }
            if segment.end < gap_end {
                next.push((segment.end, gap_end));
            }
        }
        gaps = next;
    }
    gaps.retain(|(s, e)| e - s > 0.001);
    gaps
}

/// `drag` carries a media-pool asset in flight; it is consumed here on drop.
pub fn show(
    ui: &mut Ui,
    state: &mut TimelineState,
    drag: &mut Option<DragAsset>,
    textures: &Textures,
) {
    toolbar(ui, state);
    ui.add_space(6.0);

    let pointer = ui.ctx().pointer_latest_pos();
    let released = ui.input(|i| i.pointer.any_released());
    // (track index, snapped start) resolved during the lane pass, applied after
    // it so the track list is not borrowed while mutating.
    let mut drop_into: Option<(usize, f32)> = None;
    let mut repack: Option<usize> = None;

    let head_w = header_width(ui);
    ui.spacing_mut().item_spacing = Vec2::ZERO;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.add_space(20.0);
            for t in &mut state.tracks {
                track_header_sized(ui, &t.name, t.kind, &mut t.locked, &mut t.muted, head_w);
            }
        });

        ScrollArea::horizontal()
            .id_source("timeline_scroll")
            .show(ui, |ui| {
                let px = px_per_sec_for(ui, state.seconds, state.zoom);
                ui.vertical(|ui| {
                    let ruler_resp = ruler(ui, state.seconds, px);
                    if let Some(seconds) = ruler_scrub(&ruler_resp, state.seconds, px) {
                        state.playhead = seconds;
                    }

                    let mut lanes: Vec<Rect> = Vec::with_capacity(state.tracks.len());
                    for (ti, t) in state.tracks.iter_mut().enumerate() {
                        let (lane, _) = track_lane(ui, state.seconds, px);
                        lanes.push(lane);

                        if let (Some(asset), Some(p)) = (drag.as_ref(), pointer) {
                            if lane.contains(p) {
                                let valid = t.accepts(asset);
                                let seconds = seconds_at(lane, p.x, px);
                                // Preview exactly where the gapless track will
                                // put it, not where the cursor happens to be.
                                let (_, start) = t.insert_position(seconds);
                                let preview = clip_rect(lane, start, asset.seconds, px);
                                dnd::drop_preview(ui.painter(), preview, asset.kind, valid);
                                if valid && released {
                                    drop_into = Some((ti, seconds));
                                }
                            }
                        }

                        let dim = t.muted || t.locked;
                        for c in t.clips.iter_mut() {
                            let rect = clip_rect(lane, c.start, c.len, px);
                            let poster = c
                                .path
                                .as_ref()
                                .and_then(|p| textures.get(&p.to_string_lossy()));
                            let resp = clip_block(ui, rect, c.kind, &c.label, c.selected, poster);
                            if resp.clicked() {
                                c.selected = !c.selected;
                            }
                            if resp.dragged() && !t.locked {
                                c.start = (c.start + resp.drag_delta().x / px).max(0.0);
                            }
                            // Reordering by dragging: the track re-packs once the
                            // clip is dropped, so no hole is ever left behind.
                            if resp.drag_stopped() && !t.locked {
                                repack = Some(ti);
                            }
                            if dim {
                                ui.painter().rect_filled(
                                    rect,
                                    R_SM,
                                    BG_APP.linear_multiply(0.45),
                                );
                            }
                        }
                    }

                    if let (Some(first), Some(last)) = (lanes.first(), lanes.last()) {
                        let area = Rect::from_min_max(first.left_top(), last.right_bottom());
                        for (i, cut) in state.ripple_cuts.clone().iter().enumerate() {
                            if ripple_cut_marker(ui, area, *cut, px, i).clicked() {
                                state.ripple_cuts.retain(|c| c != cut);
                            }
                        }
                        playhead_marker(ui, area, &mut state.playhead, px);
                        playhead_timecode(ui, area, state.playhead, px);
                    }
                });
            });
    });

    if let Some((track_idx, start)) = drop_into {
        if let Some(asset) = drag.take() {
            state.place(track_idx, &asset, start);
        }
    }
    if let Some(track_idx) = repack {
        state.repack_track(track_idx);
    }
}

fn toolbar(ui: &mut Ui, state: &mut TimelineState) {
    ui.horizontal(|ui| {
        if icon_button_painted(ui, Icon::Cut, true, false).clicked() {
            state.split_at_playhead();
        }
        if icon_button_painted(ui, Icon::Plus, true, false).clicked() {
            state.ripple_cuts.push(state.playhead);
        }
        if icon_button_painted(ui, Icon::Trash, true, false).clicked() {
            state.delete_selected();
        }
        ui.add_space(8.0);
        ui.label(
            RichText::new(format!("{} clips", state.clip_count()))
                .small()
                .color(TEXT_DISABLED),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.set_max_width(240.0);
            pro_slider(ui, &mut state.zoom, 0.5..=8.0, "×");
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str, seconds: f32) -> DragAsset {
        DragAsset {
            name: name.into(),
            path: None,
            seconds,
            kind: ClipKind::ARoll,
        }
    }

    fn starts(state: &TimelineState) -> Vec<(String, f32)> {
        state.tracks[0]
            .clips
            .iter()
            .map(|c| (c.label.clone(), c.start))
            .collect()
    }

    /// Every clip butts against the previous one, whatever gap the drop implied.
    fn assert_gapless(state: &TimelineState) {
        for track in &state.tracks {
            let mut cursor = 0.0;
            for clip in &track.clips {
                assert_eq!(clip.start, cursor, "gap before {}", clip.label);
                cursor += clip.len;
            }
        }
    }

    #[test]
    fn drops_pack_against_each_other() {
        let mut state = TimelineState::default();
        state.place(0, &asset("a", 4.0), 0.0);
        // Dropped far to the right: the empty space is closed, not preserved.
        state.place(0, &asset("b", 3.0), 40.0);
        state.place(0, &asset("c", 2.0), 90.0);

        assert_eq!(
            starts(&state),
            vec![("a".into(), 0.0), ("b".into(), 4.0), ("c".into(), 7.0)]
        );
        assert_gapless(&state);
        assert_eq!(state.content_end(), 9.0);
    }

    #[test]
    fn drop_position_decides_order() {
        let mut state = TimelineState::default();
        state.place(0, &asset("a", 4.0), 0.0);
        state.place(0, &asset("b", 4.0), 10.0);
        // Dropped over the first half of "a": takes its place, pushes it right.
        state.place(0, &asset("c", 2.0), 1.0);

        assert_eq!(
            starts(&state),
            vec![("c".into(), 0.0), ("a".into(), 2.0), ("b".into(), 6.0)]
        );
        assert_gapless(&state);
    }

    #[test]
    fn delete_closes_the_hole() {
        let mut state = TimelineState::default();
        state.place(0, &asset("a", 4.0), 0.0);
        state.place(0, &asset("b", 3.0), 10.0);
        state.place(0, &asset("c", 2.0), 20.0);

        state.tracks[0].clips[1].selected = true;
        state.tracks[0].clips[2].selected = false;
        state.delete_selected();

        assert_eq!(starts(&state), vec![("a".into(), 0.0), ("c".into(), 4.0)]);
        assert_gapless(&state);
    }

    #[test]
    fn split_keeps_the_track_contiguous_and_advances_trim() {
        let mut state = TimelineState::default();
        state.place(0, &asset("a", 6.0), 0.0);
        state.playhead = 2.5;
        state.split_at_playhead();

        let clips = &state.tracks[0].clips;
        assert_eq!(clips.len(), 2);
        assert_eq!((clips[0].start, clips[0].len, clips[0].trim_in), (0.0, 2.5, 0.0));
        assert_eq!((clips[1].start, clips[1].len, clips[1].trim_in), (2.5, 3.5, 2.5));
        assert_gapless(&state);
    }

    #[test]
    fn program_maps_timeline_seconds_to_source_seconds() {
        let mut state = TimelineState::default();
        let mut clip = asset("a", 6.0);
        clip.path = Some(PathBuf::from("a.mp4"));
        state.place(0, &clip, 0.0);
        state.playhead = 2.0;
        state.split_at_playhead();

        let program = state.program();
        assert_eq!(program.len(), 2);
        assert_eq!((program[0].start, program[0].media_start), (0.0, 0.0));
        assert_eq!((program[1].start, program[1].media_start), (2.0, 2.0));
    }
}
