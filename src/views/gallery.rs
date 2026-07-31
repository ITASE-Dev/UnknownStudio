//! Component gallery, kept as a dev surface reachable from View → Component Gallery.
//! It exercises every widget in `crate::ui` at whatever width the window gives it.

use crate::ui::components::chat::{
    ai_chat_bubble, prompt_input_area, typing_indicator, user_chat_bubble,
};
use crate::ui::components::inspector::{inspector_group, thumbnail_preview_box};
use crate::ui::components::media::{media_pool_grid, AssetKind, PoolAsset};
use crate::ui::components::status::{job_progress, meter_grid, service_status_bar, ServiceState};
use crate::ui::components::timeline::clip::{a_roll_clip, audio_waveform_clip, b_roll_clip};
use crate::ui::components::timeline::headers::{header_width, track_header_sized, TrackKind};
use crate::ui::components::timeline::markers::{playhead_marker, ripple_cut_marker};
use crate::ui::components::timeline::{clip_rect, px_per_sec_for, ruler, track_lane};
use crate::ui::core::buttons::{
    action_row, ai_button, ghost_button, icon_button_painted, segmented, Icon,
};
use crate::ui::core::inputs::{pro_text_input, search_input, slider_row};
use crate::ui::core::selects::pro_dropdown_row;
use crate::ui::core::toggles::{pro_checkbox, pro_toggle_row};
use crate::ui::core::typography::{panel, property_row, section_header, section_title};
use crate::ui::responsive::split;
use eframe::egui::{self, Rect, ScrollArea, Ui, Vec2};

const TL_SECONDS: f32 = 22.0;

pub struct GalleryState {
    search: String,
    title: String,
    prompt: String,
    mode: usize,
    platform: usize,
    identity_lock: bool,
    auto_broll: bool,
    remove_silence: bool,
    density: f32,
    zoom: f32,
    v1_locked: bool,
    v1_muted: bool,
    a1_locked: bool,
    a1_muted: bool,
    playhead: f32,
}

impl Default for GalleryState {
    fn default() -> Self {
        Self {
            search: String::new(),
            title: "Ep_014_final".into(),
            prompt: String::new(),
            mode: 0,
            platform: 0,
            identity_lock: true,
            auto_broll: true,
            remove_silence: false,
            density: 35.0,
            zoom: 1.0,
            v1_locked: false,
            v1_muted: false,
            a1_locked: false,
            a1_muted: false,
            playhead: 8.0,
        }
    }
}

pub fn window(ctx: &egui::Context, open: &mut bool, state: &mut GalleryState, time: f32) {
    if !*open {
        return;
    }
    egui::Window::new("Component Gallery")
        .open(open)
        .default_size([760.0, 620.0])
        .min_width(320.0)
        .vscroll(false)
        .show(ctx, |ui| {
            ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| content(ui, state, time));
        });
}

pub fn content(ui: &mut Ui, state: &mut GalleryState, time: f32) {
    controls(ui, state);
    system(ui, time);
    media(ui);
    assistant(ui, state, time);
    timeline(ui, state);
    ui.add_space(24.0);
}

fn controls(ui: &mut Ui, state: &mut GalleryState) {
    section_title(ui, "Core Controls", "Buttons, fields, toggles, pickers.");
    panel(ui, |ui| {
        action_row(ui, &[("Render Project", true), ("Cancel", false)]);
        ui.horizontal_wrapped(|ui| {
            ai_button(ui, "Generate B-Roll");
            ghost_button(ui, "Reset defaults");
            icon_button_painted(ui, Icon::Play, true, false);
            icon_button_painted(ui, Icon::Cut, false, true);
            icon_button_painted(ui, Icon::Trash, true, false);
        });
        ui.add_space(12.0);
        section_header(ui, "Fields");
        ui.horizontal_wrapped(|ui| {
            search_input(ui, &mut state.search, "Search media…");
        });
        pro_text_input(ui, &mut state.title, "Project title");
        ui.add_space(12.0);
        section_header(ui, "State");
        segmented(ui, &["Assemble", "Refine", "Grade"], &mut state.mode);
        ui.add_space(8.0);
        pro_toggle_row(ui, &mut state.identity_lock, "Identity Lock");
        pro_toggle_row(ui, &mut state.auto_broll, "Auto B-Roll");
        pro_checkbox(ui, &mut state.remove_silence, "Remove silences");
        ui.add_space(8.0);
        pro_dropdown_row(
            ui,
            "gallery_platform",
            "Platform",
            &["YouTube 16:9", "Shorts 9:16", "Reels 9:16"],
            &mut state.platform,
        );
        slider_row(ui, "B-Roll density", &mut state.density, 0.0..=100.0, " %");
        property_row(ui, "Timecode", "00:04:12:18");
    });
}

fn system(ui: &mut Ui, time: f32) {
    section_title(ui, "AI & System", "Service health and background jobs.");
    panel(ui, |ui| {
        service_status_bar(
            ui,
            &[
                ("Audio Engine Online", ServiceState::Online),
                ("ComfyUI Rendering…", ServiceState::Working),
                ("LLM Director Offline", ServiceState::Error),
                ("Upscaler Idle", ServiceState::Idle),
            ],
            time,
        );
        ui.add_space(12.0);
        job_progress(ui, "Generating B-Roll · shot 4 of 11", 0.36);
        ui.add_space(10.0);
        meter_grid(ui, &[("VRAM", 14.2, " GB"), ("GPU", 71.0, " %"), ("Queue", 3.0, "")]);
    });
}

fn media(ui: &mut Ui) {
    section_title(ui, "Media & Inspector", "Grid reflows; inspector stacks when narrow.");
    panel(ui, |ui| {
        split(
            ui,
            0.68,
            &mut (),
            |ui, _| {
                media_pool_grid(
                    ui,
                    &[
                        PoolAsset { name: "Clip_01.mp4", meta: "", duration: "00:06", kind: AssetKind::Ingested, selected: true, thumb: None },
                        PoolAsset { name: "Clip_02.mp4", meta: "", duration: "00:07", kind: AssetKind::Ingested, selected: false, thumb: None },
                        PoolAsset { name: "gen_neon_city.mp4", meta: "flux · seed 8812", duration: "00:04", kind: AssetKind::Generated, selected: false, thumb: None },
                        PoolAsset { name: "VO_take3.wav", meta: "", duration: "00:11", kind: AssetKind::Ingested, selected: false, thumb: None },
                    ],
                );
            },
            |ui, _| {
                inspector_group(ui, "Preview", |ui| {
                    thumbnail_preview_box(ui, "Clip_01.mp4 · 1920×1080");
                });
            },
        );
    });
}

fn assistant(ui: &mut Ui, state: &mut GalleryState, time: f32) {
    section_title(ui, "Director Assistant", "Bubbles track the panel measure.");
    panel(ui, |ui| {
        user_chat_bubble(ui, "Cut the first minute to 40 seconds, keep the hook.");
        ui.add_space(8.0);
        ai_chat_bubble(
            ui,
            "Director",
            "Trimmed 22s of silence and added generative B-Roll at 00:18.",
        );
        ui.add_space(8.0);
        typing_indicator(ui, time);
        ui.add_space(10.0);
        prompt_input_area(ui, &mut state.prompt);
    });
}

fn timeline(ui: &mut Ui, state: &mut GalleryState) {
    section_title(ui, "Timeline", "Lanes fill the viewport; ticks thin out as you zoom.");
    panel(ui, |ui| {
        slider_row(ui, "Zoom", &mut state.zoom, 0.5..=6.0, "");
        ui.add_space(8.0);
        let head_w = header_width(ui);
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.add_space(20.0);
                track_header_sized(ui, "V1", TrackKind::Video, &mut state.v1_locked, &mut state.v1_muted, head_w, false);
                track_header_sized(ui, "A1", TrackKind::Audio, &mut state.a1_locked, &mut state.a1_muted, head_w, false);
            });
            ScrollArea::horizontal()
                .id_source("gallery_timeline")
                .show(ui, |ui| {
                    let px = px_per_sec_for(ui, TL_SECONDS, state.zoom);
                    ui.vertical(|ui| {
                        ruler(ui, TL_SECONDS, px);
                        let (v1, _) = track_lane(ui, TL_SECONDS, px);
                        a_roll_clip(ui, clip_rect(v1, 0.0, 6.0, px), "Clip_01.mp4", false);
                        b_roll_clip(ui, clip_rect(v1, 6.0, 4.0, px), "Neon City", true);
                        a_roll_clip(ui, clip_rect(v1, 10.0, 7.0, px), "Clip_02.mp4", false);
                        let (a1, _) = track_lane(ui, TL_SECONDS, px);
                        audio_waveform_clip(ui, clip_rect(a1, 0.0, 11.0, px), "VO_take3.wav", false);
                        audio_waveform_clip(ui, clip_rect(a1, 11.0, 9.0, px), "bed_music.wav", false);

                        let area = Rect::from_min_max(v1.left_top(), a1.right_bottom());
                        ripple_cut_marker(ui, area, 13.0, px, 0);
                        playhead_marker(ui, area, &mut state.playhead, px);
                    });
                });
        });
    });
}
