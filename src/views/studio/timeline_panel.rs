use crate::ui::components::timeline::clip::{clip_block, ClipKind, ClipVisuals, Filmstrip};
use crate::ui::components::timeline::headers::{header_width, track_header_sized, TrackKind};
use crate::ui::components::timeline::markers::{
    playhead_marker, playhead_timecode, ripple_cut_marker,
};
use crate::audio_engine::AudioSegment;
use crate::media::{Poster, Segment, Textures, FILMSTRIP_FRAMES};
use crate::ui::components::timeline::{
    clip_rect, px_per_sec_for, ruler, ruler_scrub, seconds_at, track_lane,
};
use crate::ui::components::timeline::tools::{tool_status, tool_strip, Tool};
use crate::ui::core::buttons::{icon_button, icon_button_painted, Icon};
use crate::ui::core::icons;
use crate::ui::core::inputs::pro_slider;
use crate::ui::theme::tokens::*;
use crate::views::studio::dnd::{self, DragAsset};
use eframe::egui::{Align, Layout, Rect, RichText, ScrollArea, Ui, Vec2};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Stable clip identity, so a background render can find its clip again even
/// if the track was re-packed meanwhile.
static NEXT_CLIP_ID: AtomicU64 = AtomicU64::new(1);

fn next_clip_id() -> u64 {
    NEXT_CLIP_ID.fetch_add(1, Ordering::Relaxed)
}

pub struct Clip {
    pub id: u64,
    pub label: String,
    pub kind: ClipKind,
    pub start: f32,
    pub len: f32,
    pub selected: bool,
    /// Source file for clips placed from the media pool; `None` for demo blocks.
    pub path: Option<PathBuf>,
    /// Offset into the source where this clip starts (grows when split).
    pub trim_in: f32,
    /// Source has an audio stream — video clips are audible too.
    pub has_audio: bool,
    /// Full length of the source, so a trimmed clip can locate its slice of
    /// the waveform.
    pub source_seconds: f32,
}

impl Clip {
    fn new(label: &str, kind: ClipKind, start: f32, len: f32) -> Self {
        Self {
            id: next_clip_id(),
            label: label.into(),
            kind,
            start,
            len,
            selected: false,
            path: None,
            trim_in: 0.0,
            has_audio: false,
            source_seconds: len,
        }
    }

    fn from_asset(asset: &DragAsset, start: f32) -> Self {
        Self {
            id: next_clip_id(),
            label: asset.name.clone(),
            kind: asset.kind,
            start,
            len: asset.seconds,
            selected: false,
            path: asset.path.clone(),
            trim_in: 0.0,
            has_audio: asset.has_audio,
            source_seconds: asset.seconds,
        }
    }

    pub fn is_video(&self) -> bool {
        self.kind != ClipKind::Audio
    }

    /// `[from, to)` fraction of the source this clip shows.
    fn source_window(&self) -> (f32, f32) {
        let total = self.source_seconds.max(0.001);
        let from = (self.trim_in / total).clamp(0.0, 1.0);
        let to = ((self.trim_in + self.len) / total).clamp(from, 1.0);
        (from, to)
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
    /// Visible span. Follows the content instead of being a fixed canvas.
    pub seconds: f32,
    pub zoom: f32,
    pub playhead: f32,
    pub ripple_cuts: Vec<f32>,
    /// Track the toolbar's reorder/remove actions apply to.
    pub selected_track: usize,
}

/// Span shown when the timeline is empty.
const EMPTY_SPAN_SECONDS: f32 = 12.0;

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
            seconds: EMPTY_SPAN_SECONDS,
            zoom: 1.0,
            playhead: 0.0,
            ripple_cuts: Vec::new(),
            selected_track: 0,
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

        self.fit_span();
    }

    /// Re-packs one track (after a move) and keeps the visible span honest.
    pub fn repack_track(&mut self, track_idx: usize) {
        if let Some(track) = self.tracks.get_mut(track_idx) {
            track.repack();
        }
        self.fit_span();
    }

    /// Keeps the visible span just past the content, so a short clip is not
    /// squeezed into a corner of a canvas sized for something else.
    pub fn fit_span(&mut self) {
        let end = self.content_end();
        self.seconds = if end <= 0.0 {
            EMPTY_SPAN_SECONDS
        } else {
            // A small tail leaves somewhere to drop the next clip.
            end + (end * 0.08).clamp(0.5, 5.0)
        };
        self.playhead = self.playhead.clamp(0.0, self.seconds);
    }

    /// Appends a track of `kind`, grouping audio below video.
    pub fn add_track(&mut self, kind: TrackKind) {
        let track = Track {
            name: String::new(),
            kind,
            clips: Vec::new(),
            locked: false,
            muted: false,
        };
        let at = match kind {
            TrackKind::Video => self
                .tracks
                .iter()
                .position(|t| t.kind == TrackKind::Audio)
                .unwrap_or(self.tracks.len()),
            TrackKind::Audio => self.tracks.len(),
        };
        self.tracks.insert(at, track);
        self.selected_track = at;
        self.renumber();
    }

    /// Moves a track up (`-1`) or down (`+1`). Video priority follows the
    /// order, so this also decides which track wins an overlap.
    pub fn move_track(&mut self, index: usize, delta: isize) {
        let target = index as isize + delta;
        if index >= self.tracks.len() || target < 0 || target as usize >= self.tracks.len() {
            return;
        }
        let target = target as usize;
        self.tracks.swap(index, target);
        self.selected_track = target;
        self.renumber();
    }

    /// Removes a track; the last remaining one is kept so there is always
    /// somewhere to drop media.
    pub fn remove_track(&mut self, index: usize) {
        if self.tracks.len() <= 1 || index >= self.tracks.len() {
            return;
        }
        self.tracks.remove(index);
        self.selected_track = self.selected_track.min(self.tracks.len() - 1);
        self.renumber();
        self.fit_span();
    }

    /// V1..Vn / A1..An, top to bottom.
    fn renumber(&mut self) {
        let (mut video, mut audio) = (0, 0);
        for track in self.tracks.iter_mut() {
            let (prefix, n) = match track.kind {
                TrackKind::Video => {
                    video += 1;
                    ("V", video)
                }
                TrackKind::Audio => {
                    audio += 1;
                    ("A", audio)
                }
            };
            track.name = format!("{prefix}{n}");
        }
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
        self.fit_span();
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
                    id: next_clip_id(),
                    label: track.clips[i].label.clone(),
                    kind: track.clips[i].kind,
                    start: ph,
                    len: track.clips[i].len - head_len,
                    selected: false,
                    path: track.clips[i].path.clone(),
                    // The tail keeps showing the same source frames.
                    trim_in: track.clips[i].trim_in + head_len,
                    has_audio: track.clips[i].has_audio,
                    source_seconds: track.clips[i].source_seconds,
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

impl TimelineState {
    /// Everything audible, in timeline seconds. Unlike video, audio segments
    /// may overlap — the mixer sums them. Muted tracks contribute nothing.
    pub fn audio_program(&self) -> Vec<AudioSegment> {
        let mut program = Vec::new();
        for track in self.tracks.iter().filter(|t| !t.muted) {
            for clip in track.clips.iter().filter(|c| c.has_audio) {
                let Some(path) = clip.path.clone() else {
                    continue;
                };
                program.push(AudioSegment {
                    start: clip.start,
                    end: clip.start + clip.len,
                    path,
                    media_start: clip.trim_in,
                    gain: 1.0,
                });
            }
        }
        program.sort_by(|a, b| a.start.total_cmp(&b.start));
        program
    }

    /// First selected clip, with the track it sits on.
    pub fn selected_clip(&self) -> Option<(usize, &Clip)> {
        self.tracks
            .iter()
            .enumerate()
            .find_map(|(ti, t)| t.clips.iter().find(|c| c.selected).map(|c| (ti, c)))
    }

    pub fn clip_mut(&mut self, id: u64) -> Option<&mut Clip> {
        self.tracks
            .iter_mut()
            .flat_map(|t| t.clips.iter_mut())
            .find(|c| c.id == id)
    }

    /// Clip visible under `seconds` on `track_idx`.
    pub fn clip_at(&self, track_idx: usize, seconds: f32) -> Option<&Clip> {
        self.tracks
            .get(track_idx)?
            .clips
            .iter()
            .find(|c| seconds >= c.start && seconds < c.start + c.len)
    }

    /// Video sources on the timeline, for filmstrip decoding.
    pub fn video_sources(&self) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = self
            .tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .filter(|c| c.is_video())
            .filter_map(|c| c.path.clone())
            .collect();
        paths.dedup();
        paths
    }

    /// Source files that need waveform peaks.
    pub fn audio_sources(&self) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = self
            .tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .filter(|c| c.has_audio)
            .filter_map(|c| c.path.clone())
            .collect();
        paths.dedup();
        paths
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
/// What the tool strip needs to know about the current selection.
pub struct ToolContext<'a> {
    /// Per-tool availability for the current selection.
    pub availability: &'a [Result<(), &'static str>; Tool::ALL.len()],
    pub busy: Option<Tool>,
    pub status: Option<&'a str>,
}

pub fn show(
    ui: &mut Ui,
    state: &mut TimelineState,
    drag: &mut Option<DragAsset>,
    textures: &Textures,
    waveforms: &HashMap<String, Arc<Vec<f32>>>,
    tools: ToolContext<'_>,
) -> Option<Tool> {
    let pressed_tool = toolbar(ui, state, &tools);
    ui.add_space(6.0);

    let pointer = ui.ctx().pointer_latest_pos();
    let released = ui.input(|i| i.pointer.any_released());
    // (track index, snapped start) resolved during the lane pass, applied after
    // it so the track list is not borrowed while mutating.
    let mut drop_into: Option<(usize, f32)> = None;
    let mut repack: Option<usize> = None;
    let mut select_track: Option<usize> = None;

    let head_w = header_width(ui);
    ui.spacing_mut().item_spacing = Vec2::ZERO;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.add_space(20.0);
            let selected = state.selected_track;
            for (ti, t) in state.tracks.iter_mut().enumerate() {
                let resp = track_header_sized(
                    ui,
                    &t.name,
                    t.kind,
                    &mut t.locked,
                    &mut t.muted,
                    head_w,
                    ti == selected,
                );
                if resp.clicked() {
                    select_track = Some(ti);
                }
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
                            let key = c.path.as_ref().map(|p| p.to_string_lossy().into_owned());
                            let (from, to) = c.source_window();
                            let frames: Vec<Option<Poster>> = match (&key, c.is_video()) {
                                (Some(key), true) => filmstrip_frames(textures, key),
                                _ => Vec::new(),
                            };
                            let visuals = ClipVisuals {
                                filmstrip: (!frames.is_empty()).then(|| Filmstrip {
                                    frames: &frames,
                                    from,
                                    to,
                                }),
                                peaks: key
                                    .as_deref()
                                    .and_then(|k| waveforms.get(k))
                                    .map(|peaks| (peaks.as_slice(), from, to)),
                            };
                            let resp = clip_block(ui, rect, c.kind, &c.label, c.selected, visuals);
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
    if let Some(track_idx) = select_track {
        state.selected_track = track_idx;
    }

    pressed_tool
}

/// Decoded frames for one source, indexed by filmstrip position. Missing
/// entries are frames the worker has not produced yet.
fn filmstrip_frames(textures: &Textures, key: &str) -> Vec<Option<Poster>> {
    (0..FILMSTRIP_FRAMES)
        .map(|i| textures.get(&format!("{key}#{i}")))
        .collect()
}

fn toolbar(ui: &mut Ui, state: &mut TimelineState, tools: &ToolContext<'_>) -> Option<Tool> {
    let mut pressed = None;
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

        ui.add_space(10.0);
        // Track management acts on the header-selected track.
        if icon_button(ui, &icons::plus(icons::ADD_VIDEO_TRACK), true, false)
            .on_hover_text("Add video track")
            .clicked()
        {
            state.add_track(TrackKind::Video);
        }
        if icon_button(ui, &icons::plus(icons::ADD_AUDIO_TRACK), true, false)
            .on_hover_text("Add audio track")
            .clicked()
        {
            state.add_track(TrackKind::Audio);
        }
        if icon_button(ui, icons::MOVE_UP, true, false)
            .on_hover_text("Move selected track up")
            .clicked()
        {
            state.move_track(state.selected_track, -1);
        }
        if icon_button(ui, icons::MOVE_DOWN, true, false)
            .on_hover_text("Move selected track down")
            .clicked()
        {
            state.move_track(state.selected_track, 1);
        }
        if icon_button(ui, icons::REMOVE_TRACK, true, false)
            .on_hover_text("Remove selected track")
            .clicked()
        {
            state.remove_track(state.selected_track);
        }

        ui.add_space(8.0);
        ui.label(
            RichText::new(format!("{} clips", state.clip_count()))
                .small()
                .color(TEXT_DISABLED),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.set_max_width(200.0);
            pro_slider(ui, &mut state.zoom, 0.5..=8.0, "×");
        });
    });

    // Second row: the render tools plus whatever the last one reported. Kept
    // separate so a narrow timeline panel wraps instead of clipping controls.
    ui.horizontal(|ui| {
        pressed = tool_strip(ui, tools.busy, |tool| tools.availability[tool.index()]);
        ui.add_space(6.0);
        tool_status(ui, tools.status);
    });
    pressed
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
            has_audio: true,
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
    fn added_tracks_group_and_renumber() {
        let mut state = TimelineState::default();
        state.add_track(TrackKind::Video);
        state.add_track(TrackKind::Audio);

        let names: Vec<&str> = state.tracks.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["V1", "V2", "A1", "A2"]);
    }

    #[test]
    fn moving_a_track_reorders_and_renumbers() {
        let mut state = TimelineState::default();
        state.add_track(TrackKind::Video);
        // V2 is at index 1; move it above V1.
        state.move_track(1, -1);

        assert_eq!(state.selected_track, 0);
        let names: Vec<&str> = state.tracks.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["V1", "V2", "A1"]);
    }

    #[test]
    fn upper_video_track_wins_the_program() {
        let mut state = TimelineState::default();
        state.add_track(TrackKind::Video);

        let mut top = asset("top", 4.0);
        top.path = Some(PathBuf::from("top.mp4"));
        let mut under = asset("under", 4.0);
        under.path = Some(PathBuf::from("under.mp4"));

        state.place(0, &top, 0.0);
        state.place(1, &under, 0.0);

        let program = state.program();
        assert_eq!(program.len(), 1);
        assert_eq!(program[0].path, PathBuf::from("top.mp4"));

        // After reordering, the other track composites on top.
        state.move_track(1, -1);
        assert_eq!(state.program()[0].path, PathBuf::from("under.mp4"));
    }

    #[test]
    fn the_last_track_cannot_be_removed() {
        let mut state = TimelineState::default();
        state.remove_track(1);
        state.remove_track(0);
        assert_eq!(state.tracks.len(), 1);
    }

    #[test]
    fn span_follows_the_content() {
        let mut state = TimelineState::default();
        let empty = state.seconds;
        state.place(0, &asset("a", 4.0), 0.0);
        let with_clip = state.seconds;

        assert!(with_clip < empty, "span should shrink to a 4s clip");
        assert!(with_clip >= 4.0, "the clip must still fit");
        assert!(with_clip <= 4.0 * 1.5, "and not float in dead space");

        state.tracks[0].clips[0].selected = true;
        state.delete_selected();
        assert_eq!(state.seconds, empty);
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
