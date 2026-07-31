pub mod chat_panel;
pub mod dnd;
pub mod media_panel;
pub mod persistence;
pub mod tool_runner;
pub mod timeline_panel;

use crate::app::modals::{ModalAction, Modals};
use crate::app::router::AppRoute;
use crate::app::Project;
use crate::audio_engine::{AudioEngine, WaveformService};
use crate::media::{Decoded, PreviewEngine, Quality, Textures};
use crate::ui::components::inspector::preview_plate;
use crate::ui::components::timeline::tools::Tool;
use crate::ui::core::buttons::{icon_button_painted, pro_button, Icon};
use crate::ui::core::inputs::pro_slider;
use crate::ui::theme::tokens::*;
use eframe::egui::{self, Align, Layout, Margin, RichText, Stroke, Ui};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

/// Texture key for the program monitor.
const PROGRAM_TEXTURE: &str = "program_monitor";

/// Breakpoints for the editor chrome: below these the side panels fold away.
const MEDIA_MIN_W: f32 = 1000.0;
const CHAT_MIN_W: f32 = 720.0;

pub struct StudioState {
    pub chat: chat_panel::ChatState,
    pub media: media_panel::MediaState,
    pub timeline: timeline_panel::TimelineState,
    pub show_chat: bool,
    pub show_media: bool,
    pub playing: bool,
    /// Media-pool asset currently being dragged towards the timeline.
    pub drag: Option<dnd::DragAsset>,

    /// Background decode service + the textures it fills.
    pub engine: PreviewEngine,
    pub textures: Textures,
    /// Audio output. `None` when the machine has no usable output device; the
    /// studio then runs silently on its own clock.
    pub audio: Option<AudioEngine>,
    /// Waveform peaks per source file, filled in the background.
    pub waveforms: HashMap<String, Arc<Vec<f32>>>,
    waveform_service: WaveformService,
    requested_waveforms: HashSet<String>,
    /// Thumbnail jobs already queued, so a pool redraw doesn't re-request them.
    requested_thumbs: HashSet<String>,
    /// Timeline position the monitor was last asked for.
    last_request: Option<f32>,

    /// Async FFmpeg tool execution + its last message.
    pub tools: Option<tool_runner::ToolRunner>,
    pub tool_status: Option<String>,
    /// Factor the speed tool applies.
    pub speed_factor: f64,
}

impl Default for StudioState {
    fn default() -> Self {
        Self {
            chat: Default::default(),
            media: Default::default(),
            timeline: Default::default(),
            show_chat: true,
            show_media: true,
            playing: false,
            drag: None,
            engine: PreviewEngine::new(),
            textures: Textures::default(),
            audio: AudioEngine::new(),
            waveforms: HashMap::new(),
            waveform_service: WaveformService::new(),
            requested_waveforms: HashSet::new(),
            requested_thumbs: HashSet::new(),
            last_request: None,
            tools: tool_runner::ToolRunner::new(),
            tool_status: None,
            speed_factor: 2.0,
        }
    }
}

pub fn show(
    ctx: &egui::Context,
    route: &mut AppRoute,
    state: &mut StudioState,
    modals: &mut Modals,
    project: Option<&Project>,
    time: f32,
) {
    let w = ctx.screen_rect().width();
    // Fold panels on narrow windows; the toggles stay authoritative above that.
    let chat_visible = state.show_chat && w >= CHAT_MIN_W;
    let media_visible = state.show_media && w >= MEDIA_MIN_W;

    toolbar(ctx, route, state, modals, project);

    // Files dropped onto the window land in the pool exactly like picked ones.
    let dropped: Vec<std::path::PathBuf> = ctx.input(|i| {
        i.raw
            .dropped_files
            .iter()
            .filter_map(|f| f.path.clone())
            .collect()
    });
    if !dropped.is_empty() {
        state.media.import_paths(dropped);
    }

    pump_media(ctx, state);
    pump_tools(state);
    let mut pending_tool: Option<Tool> = None;

    if chat_visible {
        egui::SidePanel::left("studio_chat")
            .resizable(true)
            .default_width((w * 0.24).clamp(260.0, 380.0))
            .width_range(240.0..=460.0)
            .frame(panel_frame())
            .show(ctx, |ui| chat_panel::show(ui, &mut state.chat, time));
    }

    if media_visible {
        egui::SidePanel::right("studio_media")
            .resizable(true)
            .default_width((w * 0.22).clamp(240.0, 340.0))
            .width_range(220.0..=420.0)
            .frame(panel_frame())
            .show(ctx, |ui| {
                if let Some(asset) = media_panel::show(ui, &mut state.media, &state.textures) {
                    state.drag = Some(asset);
                }
            });
    }

    egui::TopBottomPanel::bottom("studio_timeline")
        .resizable(true)
        .default_height(200.0)
        .height_range(140.0..=380.0)
        .frame(panel_frame())
        .show(ctx, |ui| {
            let availability = tool_availability(state);
            let context = timeline_panel::ToolContext {
                availability: &availability,
                busy: state.tools.as_ref().and_then(tool_runner::ToolRunner::running),
                status: state.tool_status.as_deref(),
            };
            timeline_panel::show(
                ui,
                &mut state.timeline,
                &mut state.drag,
                &state.textures,
                &state.waveforms,
                context,
            )
        })
        .inner
        .map(|tool| pending_tool = Some(tool));

    egui::CentralPanel::default()
        .frame(egui::Frame::none().fill(BG_APP).inner_margin(Margin::same(10.0)))
        .show(ctx, |ui| preview(ui, state, project));

    if let Some(tool) = pending_tool {
        run_tool(state, tool);
    }

    // The timeline consumes a valid drop; anything still in flight after every
    // panel has drawn was released elsewhere and is discarded.
    if let Some(asset) = &state.drag {
        dnd::ghost(ctx, asset);
        if ctx.input(|i| i.pointer.any_released()) {
            state.drag = None;
        }
    }
}

/// Advances playback, keeps the decoder's program current, asks for the frame
/// under the playhead and uploads whatever came back. Everything here is
/// non-blocking: the worker threads own all FFmpeg work.
fn pump_media(ctx: &egui::Context, state: &mut StudioState) {
    pump_audio(state);

    let end = state.timeline.content_end();
    if state.playing {
        // The sound card is the master clock: video chases the audio position
        // so the two can't drift. Without a device we fall back to frame time.
        state.timeline.playhead = match &state.audio {
            Some(audio) if audio.is_playing() => audio.position_seconds(),
            _ => state.timeline.playhead + ctx.input(|i| i.stable_dt).min(0.1),
        };
        if state.timeline.playhead >= end {
            state.timeline.playhead = end;
            state.playing = false;
            if let Some(audio) = &state.audio {
                audio.pause();
            }
        }
        state.timeline.seconds = state.timeline.seconds.max(state.timeline.playhead);
    }

    let program = state.timeline.program();
    let program_changed = program != *state.engine.program();
    state.engine.set_program(program);

    // Re-request only when the position (or the program under it) moved.
    let playhead = state.timeline.playhead;
    if program_changed || state.last_request != Some(playhead) {
        state.last_request = Some(playhead);
        // Smoothness matters more than resolution while running; a parked
        // playhead gets the full-size frame.
        let quality = if state.playing {
            Quality::Proxy
        } else {
            Quality::Full
        };
        state.engine.request_frame(playhead, quality);
    }

    request_thumbnails(state);

    for decoded in state.engine.poll().collect::<Vec<_>>() {
        match decoded {
            Decoded::Frame { frame, .. } => state.textures.set(ctx, PROGRAM_TEXTURE, &frame),
            Decoded::Blank { .. } => state.textures.remove(PROGRAM_TEXTURE),
            Decoded::Thumbnail { path, frame } => {
                state.textures.set(ctx, path.to_string_lossy(), &frame)
            }
            Decoded::FilmstripFrame { path, index, frame } => {
                state
                    .textures
                    .set(ctx, format!("{}#{index}", path.to_string_lossy()), &frame)
            }
        }
    }
}

/// Which tools the current selection can satisfy, and why not when it can't.
fn tool_availability(state: &StudioState) -> [Result<(), &'static str>; Tool::ALL.len()] {
    let selected = state.timeline.selected_clip();
    let has_source = selected.is_some_and(|(_, c)| c.path.is_some());
    let runner = state.tools.is_some();

    let needs_clip = |ok: bool, reason: &'static str| -> Result<(), &'static str> {
        if !runner {
            return Err("FFmpeg tool runner unavailable.");
        }
        if ok {
            Ok(())
        } else {
            Err(reason)
        }
    };

    let mut table = [Err("Select a clip on the timeline first."); Tool::ALL.len()];
    for tool in Tool::ALL {
        table[tool.index()] = match tool {
            Tool::Trim | Tool::Speed | Tool::Crop | Tool::MuteAudio => {
                needs_clip(has_source, "Select a clip with a source file.")
            }
            Tool::ExtractAudio => needs_clip(
                selected.is_some_and(|(_, c)| c.path.is_some() && c.has_audio),
                "Select a clip that has an audio track.",
            ),
            Tool::Overlay => needs_clip(
                overlay_pair(state).is_some(),
                "Select a clip with another clip beneath it on the next track.",
            ),
            Tool::Concat => needs_clip(
                concat_inputs(state).len() >= 2,
                "Needs at least two clips with sources on the selected track.",
            ),
        };
    }
    table
}

/// Selected clip plus whatever sits under it on the following track.
fn overlay_pair(state: &StudioState) -> Option<(PathBuf, PathBuf, f32, f32)> {
    let (track_idx, top) = state.timeline.selected_clip()?;
    let top_path = top.path.clone()?;
    let base = state
        .timeline
        .clip_at(track_idx + 1, top.start + top.len * 0.5)?;
    let base_path = base.path.clone()?;
    // Overlay window in the base clip's own time.
    let start = (top.start - base.start).max(0.0);
    Some((base_path, top_path, start, start + top.len))
}

fn concat_inputs(state: &StudioState) -> Vec<PathBuf> {
    let Some((track_idx, _)) = state.timeline.selected_clip() else {
        return Vec::new();
    };
    state
        .timeline
        .tracks
        .get(track_idx)
        .map(|t| t.clips.iter().filter_map(|c| c.path.clone()).collect())
        .unwrap_or_default()
}

/// Turns a pressed tool into a job on the selected clip.
fn run_tool(state: &mut StudioState, tool: Tool) {
    let Some((_, clip)) = state.timeline.selected_clip() else {
        return;
    };
    let (clip_id, path, trim_in, len) = (
        clip.id,
        clip.path.clone(),
        clip.trim_in as f64,
        clip.len as f64,
    );
    let speed = state.speed_factor;
    let overlay = overlay_pair(state);
    let concat = concat_inputs(state);

    let job = match (tool, path) {
        (Tool::Trim, Some(input)) => tool_runner::ToolJob::Trim {
            input,
            start: trim_in,
            duration: len,
        },
        (Tool::Speed, Some(input)) => tool_runner::ToolJob::Speed {
            input,
            factor: speed,
        },
        (Tool::Crop, Some(input)) => tool_runner::ToolJob::Crop { input },
        (Tool::ExtractAudio, Some(input)) => tool_runner::ToolJob::ExtractAudio { input },
        (Tool::MuteAudio, Some(input)) => tool_runner::ToolJob::MuteAudio { input },
        (Tool::Overlay, _) => {
            let Some((base, top, start, end)) = overlay else {
                return;
            };
            tool_runner::ToolJob::Overlay {
                base,
                top,
                start: start as f64,
                end: end as f64,
            }
        }
        (Tool::Concat, _) => tool_runner::ToolJob::Concat { inputs: concat },
        _ => return,
    };

    if let Some(runner) = &mut state.tools {
        state.tool_status = Some(format!("{}…", tool.label()));
        runner.submit(tool, clip_id, job);
    }
}

/// Applies finished renders: the clip adopts the new file, or the product lands
/// in the pool when it is a standalone asset.
fn pump_tools(state: &mut StudioState) {
    let Some(runner) = &mut state.tools else {
        return;
    };

    for outcome in runner.poll() {
        match outcome.result {
            Err(message) => {
                state.tool_status = Some(format!("{} failed: {message}", outcome.tool.label()));
            }
            Ok(product) if product.as_new_asset => {
                state.media.import_paths([product.path]);
                state.tool_status = Some(format!("{} → media pool", outcome.tool.label()));
            }
            Ok(product) => {
                if let Some(clip) = state.timeline.clip_mut(outcome.clip_id) {
                    clip.path = Some(product.path);
                    // A re-rendered source starts at zero; the trim is baked in.
                    clip.trim_in = 0.0;
                    if let Some(seconds) = product.seconds {
                        clip.len = seconds;
                        clip.source_seconds = seconds;
                    } else if outcome.tool == Tool::Speed {
                        let factor = state.speed_factor as f32;
                        clip.len /= factor;
                        clip.source_seconds /= factor;
                    }
                    if outcome.tool == Tool::MuteAudio {
                        clip.has_audio = false;
                    }
                }
                state.timeline.repack_track(state.timeline.selected_track);
                state.tool_status = Some(format!("{} done", outcome.tool.label()));
            }
        }
    }
}

/// Keeps the mixer's program current and collects finished waveform peaks.
fn pump_audio(state: &mut StudioState) {
    let program = state.timeline.audio_program();
    if let Some(audio) = &mut state.audio {
        audio.set_program(program);
    }

    for path in state.timeline.audio_sources() {
        let key = path.to_string_lossy().into_owned();
        if !state.waveforms.contains_key(&key) && state.requested_waveforms.insert(key) {
            state.waveform_service.request(path);
        }
    }

    for wave in state.waveform_service.poll().collect::<Vec<_>>() {
        state
            .waveforms
            .insert(wave.path.to_string_lossy().into_owned(), wave.peaks);
    }
}

/// Starts or stops playback on both clocks at once.
fn set_playing(state: &mut StudioState, playing: bool) {
    state.playing = playing;
    let Some(audio) = &state.audio else {
        return;
    };
    if playing {
        // Seek first: it clears the ring buffer so playback starts exactly at
        // the playhead instead of continuing stale audio.
        audio.seek(state.timeline.playhead);
        audio.play();
    } else {
        audio.pause();
    }
}

/// Queues a poster frame for every pool asset, and a filmstrip for every video
/// source on the timeline, once each.
fn request_thumbnails(state: &mut StudioState) {
    for asset in &state.media.assets {
        let Some(path) = asset.path.clone() else {
            continue;
        };
        let key = path.to_string_lossy().into_owned();
        if state.requested_thumbs.insert(key) {
            state.engine.request_thumbnail(path);
        }
    }

    for path in state.timeline.video_sources() {
        let key = format!("filmstrip:{}", path.to_string_lossy());
        if state.requested_thumbs.insert(key) {
            state.engine.request_filmstrip(path);
        }
    }
}

fn panel_frame() -> egui::Frame {
    egui::Frame::none()
        .fill(BG_PANEL)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .inner_margin(Margin::same(10.0))
}

/// Workspace toolbar: panel toggles on the left, hand-off actions on the right.
fn toolbar(
    ctx: &egui::Context,
    route: &mut AppRoute,
    state: &mut StudioState,
    modals: &mut Modals,
    project: Option<&Project>,
) {
    egui::TopBottomPanel::top("studio_toolbar")
        .exact_height(40.0)
        .frame(
            egui::Frame::none()
                .fill(BG_APP)
                .inner_margin(Margin::symmetric(10.0, 0.0)),
        )
        .show(ctx, |ui| {
            ui.painter().line_segment(
                [ui.max_rect().left_bottom(), ui.max_rect().right_bottom()],
                Stroke::new(1.0_f32, BORDER),
            );
            ui.horizontal_centered(|ui| {
                let name = project.map(|p| p.name.as_str()).unwrap_or("Untitled Project");
                ui.add(
                    egui::Label::new(RichText::new(name).strong().color(TEXT_PRIMARY))
                        .truncate(true),
                );
                ui.add_space(10.0);
                if ui
                    .selectable_label(state.show_chat, RichText::new("Chat").small())
                    .clicked()
                {
                    state.show_chat = !state.show_chat;
                }
                if ui
                    .selectable_label(state.show_media, RichText::new("Media").small())
                    .clicked()
                {
                    state.show_media = !state.show_media;
                }

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if pro_button(ui, "Growth & Export", true).clicked() {
                        if let Some(p) = project {
                            *route = AppRoute::Growth(p.id);
                        }
                    }
                    if pro_button(ui, "Render", false).clicked() {
                        let next = project
                            .map(|p| AppRoute::Growth(p.id))
                            .unwrap_or(AppRoute::Dashboard);
                        modals.progress(
                            "Rendering",
                            "Encoding the current cut.",
                            ModalAction::Navigate(next),
                        );
                    }
                    if pro_button(ui, "Projects", false).clicked() {
                        *route = AppRoute::Dashboard;
                    }
                });
            });
        });
}

/// Program monitor: 16:9 plate that fits the remaining space, plus transport.
fn preview(ui: &mut Ui, state: &mut StudioState, project: Option<&Project>) {
    let transport_h = 40.0;
    let avail_h = (ui.available_height() - transport_h).max(80.0);
    let image = state.textures.get(PROGRAM_TEXTURE);
    ui.vertical_centered(|ui| {
        let width = ui.available_width().min(avail_h * 16.0 / 9.0);
        preview_plate(
            ui,
            &format!(
                "{} · {}",
                project.map(|p| p.platform.as_str()).unwrap_or("YouTube 16:9"),
                state.timeline.timecode()
            ),
            width,
            image,
        );
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            if icon_button_painted(ui, Icon::Play, false, state.playing).clicked() {
                set_playing(state, true);
            }
            if icon_button_painted(ui, Icon::Pause, false, !state.playing).clicked() {
                set_playing(state, false);
            }
            if icon_button_painted(ui, Icon::Cut, true, false).clicked() {
                state.timeline.split_at_playhead();
            }
            ui.add_space(8.0);
            ui.label(
                RichText::new(state.timeline.timecode())
                    .monospace()
                    .color(TEXT_PRIMARY),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("{} clips", state.timeline.clip_count()))
                        .small()
                        .color(TEXT_DISABLED),
                );
                ui.add_space(10.0);
                match &state.audio {
                    Some(audio) => {
                        let mut volume = audio.volume();
                        ui.set_max_width(200.0);
                        if pro_slider(ui, &mut volume, 0.0..=1.5, "×").changed() {
                            audio.set_volume(volume);
                        }
                    }
                    None => {
                        ui.label(
                            RichText::new("no audio device")
                                .small()
                                .color(TEXT_DISABLED),
                        );
                    }
                }
            });
        });
    });
}
