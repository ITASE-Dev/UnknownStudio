pub mod chat_panel;
pub mod media_panel;
pub mod timeline_panel;

use crate::app::modals::{ModalAction, Modals};
use crate::app::router::AppRoute;
use crate::app::Project;
use crate::ui::components::inspector::thumbnail_preview_box_fit;
use crate::ui::core::buttons::{icon_button_painted, pro_button, Icon};
use crate::ui::theme::tokens::*;
use eframe::egui::{self, Align, Layout, Margin, RichText, Stroke, Ui};

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
            .show(ctx, |ui| media_panel::show(ui, &mut state.media));
    }

    egui::TopBottomPanel::bottom("studio_timeline")
        .resizable(true)
        .default_height(200.0)
        .height_range(140.0..=380.0)
        .frame(panel_frame())
        .show(ctx, |ui| timeline_panel::show(ui, &mut state.timeline));

    egui::CentralPanel::default()
        .frame(egui::Frame::none().fill(BG_APP).inner_margin(Margin::same(10.0)))
        .show(ctx, |ui| preview(ui, state, project));
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
    ui.vertical_centered(|ui| {
        thumbnail_preview_box_fit(
            ui,
            &format!(
                "{} · {}",
                project.map(|p| p.platform.as_str()).unwrap_or("YouTube 16:9"),
                state.timeline.timecode()
            ),
            avail_h,
        );
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            if icon_button_painted(ui, Icon::Play, false, state.playing).clicked() {
                state.playing = true;
            }
            if icon_button_painted(ui, Icon::Pause, false, !state.playing).clicked() {
                state.playing = false;
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
            });
        });
    });
}
