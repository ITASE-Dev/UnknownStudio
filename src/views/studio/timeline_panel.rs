use crate::ui::components::timeline::clip::{clip_block, ClipKind};
use crate::ui::components::timeline::headers::{header_width, track_header_sized, TrackKind};
use crate::ui::components::timeline::markers::{
    playhead_marker, playhead_timecode, ripple_cut_marker,
};
use crate::ui::components::timeline::{clip_rect, px_per_sec_for, ruler, track_lane};
use crate::ui::core::buttons::{icon_button_painted, Icon};
use crate::ui::core::inputs::pro_slider;
use crate::ui::theme::tokens::*;
use eframe::egui::{Align, Layout, Rect, RichText, ScrollArea, Ui, Vec2};

pub struct Clip {
    pub label: String,
    pub kind: ClipKind,
    pub start: f32,
    pub len: f32,
    pub selected: bool,
}

impl Clip {
    fn new(label: &str, kind: ClipKind, start: f32, len: f32) -> Self {
        Self { label: label.into(), kind, start, len, selected: false }
    }
}

pub struct Track {
    pub name: String,
    pub kind: TrackKind,
    pub clips: Vec<Clip>,
    pub locked: bool,
    pub muted: bool,
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
                    clips: vec![
                        Clip::new("Clip_01.mp4", ClipKind::ARoll, 0.0, 6.0),
                        Clip::new("Neon City", ClipKind::BRoll, 6.0, 4.0),
                        Clip::new("Clip_02.mp4", ClipKind::ARoll, 10.0, 7.0),
                        Clip::new("Server Rack", ClipKind::BRoll, 17.0, 3.0),
                    ],
                },
                Track {
                    name: "A1".into(),
                    kind: TrackKind::Audio,
                    locked: false,
                    muted: false,
                    clips: vec![
                        Clip::new("VO_take3.wav", ClipKind::Audio, 0.0, 11.0),
                        Clip::new("bed_music.wav", ClipKind::Audio, 11.0, 9.0),
                    ],
                },
            ],
            seconds: 24.0,
            zoom: 1.0,
            playhead: 8.0,
            ripple_cuts: vec![13.0],
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

    /// Split the clip under the playhead on the first unlocked video track.
    pub fn split_at_playhead(&mut self) {
        let ph = self.playhead;
        for track in self.tracks.iter_mut().filter(|t| !t.locked) {
            if let Some(i) = track
                .clips
                .iter()
                .position(|c| ph > c.start + 0.05 && ph < c.start + c.len - 0.05)
            {
                let tail_len = track.clips[i].start + track.clips[i].len - ph;
                track.clips[i].len = ph - track.clips[i].start;
                let label = track.clips[i].label.clone();
                let kind = track.clips[i].kind;
                track.clips.insert(i + 1, Clip::new(&label, kind, ph, tail_len));
                break;
            }
        }
    }
}

pub fn show(ui: &mut Ui, state: &mut TimelineState) {
    toolbar(ui, state);
    ui.add_space(6.0);

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
                    if ruler(ui, state.seconds, px).clicked() {
                        // Scrubbing from the ruler lands with the playback engine.
                    }

                    let mut lanes: Vec<Rect> = Vec::with_capacity(state.tracks.len());
                    for t in state.tracks.iter_mut() {
                        let (lane, _) = track_lane(ui, state.seconds, px);
                        lanes.push(lane);
                        let dim = t.muted || t.locked;
                        for c in t.clips.iter_mut() {
                            let rect = clip_rect(lane, c.start, c.len, px);
                            let resp = clip_block(ui, rect, c.kind, &c.label, c.selected);
                            if resp.clicked() {
                                c.selected = !c.selected;
                            }
                            if resp.dragged() && !t.locked {
                                c.start = (c.start + resp.drag_delta().x / px).max(0.0);
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
            for t in state.tracks.iter_mut() {
                t.clips.retain(|c| !c.selected);
            }
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
