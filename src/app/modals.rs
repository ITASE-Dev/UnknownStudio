use super::router::AppRoute;
use crate::ui::components::status::job_progress;
use crate::ui::core::buttons::{ghost_button, pro_button};
use crate::ui::theme::tokens::*;
use eframe::egui::{self, Align, Align2, Color32, Layout, Margin, Order, RichText, Sense, Vec2};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ModalKind {
    Info,
    Error,
    Confirm,
    Progress,
}

/// What happens once a modal is dismissed or its work finishes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ModalAction {
    None,
    Navigate(AppRoute),
    Quit,
}

pub struct Modal {
    pub kind: ModalKind,
    pub title: String,
    pub body: String,
    pub progress: f32,
    pub confirm_label: String,
    pub action: ModalAction,
}

impl Modal {
    fn new(kind: ModalKind, title: &str, body: &str) -> Self {
        Self {
            kind,
            title: title.into(),
            body: body.into(),
            progress: 0.0,
            confirm_label: "Continue".into(),
            action: ModalAction::None,
        }
    }
}

/// Global overlay stack. The top-most modal is the only one rendered.
#[derive(Default)]
pub struct Modals {
    stack: Vec<Modal>,
}

impl Modals {
    pub fn is_open(&self) -> bool {
        !self.stack.is_empty()
    }

    pub fn push(&mut self, modal: Modal) {
        self.stack.push(modal);
    }

    pub fn info(&mut self, title: &str, body: &str) {
        self.push(Modal::new(ModalKind::Info, title, body));
    }

    pub fn error(&mut self, title: &str, body: &str) {
        self.push(Modal::new(ModalKind::Error, title, body));
    }

    pub fn confirm(&mut self, title: &str, body: &str, confirm_label: &str, action: ModalAction) {
        let mut m = Modal::new(ModalKind::Confirm, title, body);
        m.confirm_label = confirm_label.into();
        m.action = action;
        self.push(m);
    }

    /// Long-running job overlay; advances on its own and then runs `action`.
    pub fn progress(&mut self, title: &str, body: &str, action: ModalAction) {
        let mut m = Modal::new(ModalKind::Progress, title, body);
        m.action = action;
        self.push(m);
    }

    pub fn close_top(&mut self) {
        self.stack.pop();
    }
}

/// Right-aligned action row with a fixed height, so the dialog hugs its content.
fn button_row(ui: &mut egui::Ui, width: f32, add: impl FnOnce(&mut egui::Ui)) {
    ui.allocate_ui_with_layout(
        Vec2::new(width, CTRL_H),
        Layout::right_to_left(Align::Center),
        add,
    );
}

/// Render the top modal over everything else. Call last in `App::update`.
pub fn show(ctx: &egui::Context, modals: &mut Modals, route: &mut AppRoute, dt: f32) {
    let Some(top) = modals.stack.last_mut() else {
        return;
    };

    if top.kind == ModalKind::Progress {
        top.progress = (top.progress + dt * 0.35).min(1.0);
    }

    let screen = ctx.screen_rect();
    egui::Area::new("modal_scrim".into())
        .order(Order::Middle)
        .fixed_pos(screen.min)
        .interactable(true)
        .show(ctx, |ui| {
            ui.painter()
                .rect_filled(screen, 0.0, Color32::from_black_alpha(150));
            ui.allocate_rect(screen, Sense::click_and_drag());
        });

    let mut close = false;
    let mut run_action = false;
    let width = (screen.width() * 0.6).clamp(260.0, 420.0);

    egui::Area::new("modal_dialog".into())
        .order(Order::Foreground)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .show(ctx, |ui| {
            egui::Frame::none()
                .fill(BG_PANEL)
                .stroke(hairline())
                .rounding(R)
                .inner_margin(Margin::same(16.0))
                .shadow(egui::epaint::Shadow {
                    offset: Vec2::new(0.0, 10.0),
                    blur: 28.0,
                    spread: 0.0,
                    color: Color32::from_black_alpha(160),
                })
                .show(ui, |ui| {
                    ui.set_width(width);
                    let accent = match top.kind {
                        ModalKind::Error => ERR,
                        ModalKind::Progress => ACCENT,
                        ModalKind::Confirm => WARN,
                        ModalKind::Info => TEXT_PRIMARY,
                    };
                    ui.label(RichText::new(&top.title).heading().strong().color(accent));
                    ui.add_space(6.0);
                    ui.label(RichText::new(&top.body).color(TEXT_SECONDARY));
                    ui.add_space(14.0);

                    if top.kind == ModalKind::Progress {
                        job_progress(ui, "Working…", top.progress);
                        ui.add_space(10.0);
                    }
                    // The row must be height-bounded: a bare `with_layout` would
                    // claim the Area's full screen height and stretch the dialog.
                    button_row(ui, width, |ui| match top.kind {
                        ModalKind::Progress => {
                            if top.progress >= 1.0 {
                                if pro_button(ui, "Done", true).clicked() {
                                    close = true;
                                    run_action = true;
                                }
                            } else if ghost_button(ui, "Cancel").clicked() {
                                close = true;
                            }
                        }
                        ModalKind::Confirm => {
                            if pro_button(ui, &top.confirm_label, true).clicked() {
                                close = true;
                                run_action = true;
                            }
                            if pro_button(ui, "Cancel", false).clicked() {
                                close = true;
                            }
                        }
                        _ => {
                            if pro_button(ui, "OK", true).clicked() {
                                close = true;
                                run_action = true;
                            }
                        }
                    });
                });
        });

    let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    if escape && top.kind != ModalKind::Progress {
        close = true;
    }

    if close {
        let action = if run_action { top.action } else { ModalAction::None };
        modals.close_top();
        match action {
            ModalAction::Navigate(r) => *route = r,
            ModalAction::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            ModalAction::None => {}
        }
    }
}
