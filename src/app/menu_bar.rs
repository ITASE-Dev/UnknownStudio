use super::modals::{ModalAction, Modals};
use super::router::{AppRoute, RouteHistory};
use crate::ui::components::status::{service_status_pill, ServiceState};
use crate::ui::theme::tokens::*;
use eframe::egui::{self, Align, Layout, Margin, RichText, Stroke, Vec2};

pub const HEIGHT: f32 = 32.0;

pub struct MenuBarCtx<'a> {
    pub route: &'a mut AppRoute,
    pub history: &'a mut RouteHistory,
    pub modals: &'a mut Modals,
    pub gallery_open: &'a mut bool,
    pub project_name: Option<&'a str>,
    pub service: (&'a str, ServiceState),
    pub time: f32,
}

/// Native-feeling top menu bar. Rendered first in `update`, for every route.
pub fn show(ctx: &egui::Context, m: &mut MenuBarCtx<'_>) {
    egui::TopBottomPanel::top("app_menu_bar")
        .exact_height(HEIGHT)
        .frame(
            egui::Frame::none()
                .fill(BG_PANEL)
                .inner_margin(Margin::symmetric(8.0, 0.0)),
        )
        .show(ctx, |ui| {
            ui.painter().line_segment(
                [ui.max_rect().left_bottom(), ui.max_rect().right_bottom()],
                Stroke::new(1.0_f32, BORDER),
            );
            let has_project = m.route.project().is_some();
            // The right-hand group has to live inside `menu::bar`, otherwise the bar
            // claims the full row and squeezes it to an ellipsis.
            egui::menu::bar(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                file_menu(ui, m);
                project_menu(ui, m, has_project);
                view_menu(ui, m, has_project);
                help_menu(ui, m);

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let (label, state) = m.service;
                    service_status_pill(ui, label, state, m.time);
                    ui.add_space(8.0);
                    breadcrumb(ui, *m.route, m.project_name);
                });
            });
        });
}

fn breadcrumb(ui: &mut egui::Ui, route: AppRoute, project: Option<&str>) {
    if let Some(name) = project {
        ui.add(egui::Label::new(RichText::new(name).color(TEXT_PRIMARY)).truncate(true));
        ui.label(RichText::new("/").color(TEXT_DISABLED));
    }
    ui.add(egui::Label::new(RichText::new(route.title()).color(TEXT_SECONDARY)).truncate(true));
}

/// Full-width flat menu entry, painted so the label stays left-aligned like a
/// native menu instead of being centred the way `Button` would.
fn item(ui: &mut egui::Ui, label: &str) -> bool {
    let w = ui.available_width().max(170.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 22.0), egui::Sense::click());
    let enabled = ui.is_enabled();
    let color = if enabled { TEXT_PRIMARY } else { TEXT_DISABLED };
    if resp.hovered() && enabled {
        ui.painter().rect_filled(rect, R_SM, ACCENT.linear_multiply(0.35));
    }
    ui.painter().text(
        egui::Pos2::new(rect.left() + 8.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::TextStyle::Button.resolve(ui.style()),
        color,
    );
    resp.clicked() && enabled
}

fn file_menu(ui: &mut egui::Ui, m: &mut MenuBarCtx<'_>) {
    ui.menu_button("File", |ui| {
        ui.set_min_width(190.0);
        if item(ui, "New Project…") {
            *m.route = AppRoute::Onboarding;
            ui.close_menu();
        }
        if item(ui, "Open Project…") {
            m.modals
                .info("Open Project", "File picking is not wired up in this mockup yet.");
            ui.close_menu();
        }
        if item(ui, "Import Media…") {
            m.modals
                .info("Import Media", "The ingest pipeline lands with the media backend.");
            ui.close_menu();
        }
        ui.separator();
        if item(ui, "Save Project") {
            m.modals
                .info("Saved", "Project state written to the local library.");
            ui.close_menu();
        }
        if item(ui, "Render & Export…") {
            let next = m
                .route
                .project()
                .map(AppRoute::Growth)
                .unwrap_or(AppRoute::Dashboard);
            m.modals.progress(
                "Rendering",
                "Encoding the timeline to H.264 · 1080p.",
                ModalAction::Navigate(next),
            );
            ui.close_menu();
        }
        ui.separator();
        if item(ui, "Quit") {
            m.modals.confirm(
                "Quit Studio?",
                "Unsaved changes to the current cut will be lost.",
                "Quit",
                ModalAction::Quit,
            );
            ui.close_menu();
        }
    });
}

fn project_menu(ui: &mut egui::Ui, m: &mut MenuBarCtx<'_>, has_project: bool) {
    ui.menu_button("Project", |ui| {
        ui.set_min_width(190.0);
        ui.add_enabled_ui(has_project, |ui| {
            if item(ui, "Open Studio") {
                if let Some(id) = m.route.project() {
                    *m.route = AppRoute::Studio(id);
                }
                ui.close_menu();
            }
            if item(ui, "Growth & Export") {
                if let Some(id) = m.route.project() {
                    *m.route = AppRoute::Growth(id);
                }
                ui.close_menu();
            }
            ui.separator();
            if item(ui, "Project Settings…") {
                m.modals.info(
                    "Project Settings",
                    "Platform, blueprint and identity lock live here.",
                );
                ui.close_menu();
            }
            if item(ui, "Delete Project…") {
                m.modals.confirm(
                    "Delete project?",
                    "This removes the cut and every generated asset.",
                    "Delete",
                    ModalAction::Navigate(AppRoute::Dashboard),
                );
                ui.close_menu();
            }
        });
    });
}

fn view_menu(ui: &mut egui::Ui, m: &mut MenuBarCtx<'_>, has_project: bool) {
    ui.menu_button("View", |ui| {
        ui.set_min_width(190.0);
        if item(ui, "Projects") {
            *m.route = AppRoute::Dashboard;
            ui.close_menu();
        }
        ui.add_enabled_ui(has_project, |ui| {
            if item(ui, "Studio Workspace") {
                if let Some(id) = m.route.project() {
                    *m.route = AppRoute::Studio(id);
                }
                ui.close_menu();
            }
            if item(ui, "Growth Center") {
                if let Some(id) = m.route.project() {
                    *m.route = AppRoute::Growth(id);
                }
                ui.close_menu();
            }
        });
        ui.separator();
        ui.checkbox(m.gallery_open, "Component Gallery");
        ui.separator();
        ui.add_enabled_ui(m.history.can_go_back(), |ui| {
            if item(ui, "Back") {
                m.history.back(m.route);
                ui.close_menu();
            }
        });
    });
}

fn help_menu(ui: &mut egui::Ui, m: &mut MenuBarCtx<'_>) {
    ui.menu_button("Help", |ui| {
        ui.set_min_width(190.0);
        if item(ui, "About") {
            m.modals.info(
                "AI Video Director Studio",
                "Pure egui shell · component library + routing scaffold.",
            );
            ui.close_menu();
        }
        if item(ui, "Keyboard Shortcuts") {
            m.modals.info(
                "Shortcuts",
                "⌘↩ send prompt · Space play/pause · Esc dismiss dialogs.",
            );
            ui.close_menu();
        }
    });
}
