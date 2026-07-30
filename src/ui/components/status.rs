use crate::ui::responsive::{breakpoint, elided_galley, grid};
use crate::ui::theme::tokens::*;
use eframe::egui::{
    self, Align, Color32, Layout, Pos2, Response, RichText, Sense, Stroke, TextStyle, Ui, Vec2,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ServiceState {
    Online,
    Working,
    Error,
    Idle,
}

impl ServiceState {
    pub fn color(self) -> Color32 {
        match self {
            Self::Online => OK,
            Self::Working => WARN,
            Self::Error => ERR,
            Self::Idle => IDLE,
        }
    }
}

/// Sunken pill: glowing state dot + label, elided to fit the row.
/// `time` drives the pulse for `Working`.
pub fn service_status_pill(ui: &mut Ui, label: &str, state: ServiceState, time: f32) -> Response {
    let text_max = (ui.available_width() - 34.0).max(18.0);
    let galley = elided_galley(
        ui,
        label,
        TextStyle::Small.resolve(ui.style()),
        TEXT_PRIMARY,
        text_max,
    );
    let size = Vec2::new(galley.size().x + 34.0, 22.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter();

    p.rect(
        rect,
        R_PILL,
        BG_SUNKEN,
        Stroke::new(1.0_f32, state.color().linear_multiply(0.45)),
    );
    glow_dot(p, Pos2::new(rect.left() + 12.0, rect.center().y), state, time);
    p.galley(
        Pos2::new(rect.left() + 22.0, rect.center().y - galley.size().y / 2.0),
        galley,
        TEXT_PRIMARY,
    );
    resp.on_hover_text(label)
}

/// Dot-only pill for very narrow chrome; the label moves to the tooltip.
pub fn service_status_dot(ui: &mut Ui, label: &str, state: ServiceState, time: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(22.0, 22.0), Sense::hover());
    let p = ui.painter();
    p.rect(
        rect,
        R_PILL,
        BG_SUNKEN,
        Stroke::new(1.0_f32, state.color().linear_multiply(0.45)),
    );
    glow_dot(p, rect.center(), state, time);
    resp.on_hover_text(label)
}

/// Bare dot with a soft halo; pulses while `Working`.
pub fn glow_dot(p: &egui::Painter, center: Pos2, state: ServiceState, time: f32) {
    let c = state.color();
    let pulse = if state == ServiceState::Working {
        0.45 + 0.55 * (time * 3.0).sin().abs()
    } else {
        1.0
    };
    p.circle_filled(center, 7.5, c.linear_multiply(0.10 * pulse));
    p.circle_filled(center, 5.5, c.linear_multiply(0.20 * pulse));
    p.circle_filled(center, 3.5, c);
}

/// Wrapped row of pills that collapse to dots when the row gets tight.
pub fn service_status_bar(ui: &mut Ui, services: &[(&str, ServiceState)], time: f32) {
    let dots_only = ui.available_width() < 260.0;
    ui.horizontal_wrapped(|ui| {
        for (label, state) in services {
            if dots_only {
                service_status_dot(ui, label, *state, time);
            } else {
                service_status_pill(ui, label, *state, time);
            }
        }
    });
}

/// Thin progress bar spanning the row, with a right-aligned percentage.
pub fn job_progress(ui: &mut Ui, label: &str, progress: f32) -> Response {
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            ui.add(
                egui::Label::new(RichText::new(label).small().color(TEXT_SECONDARY))
                    .truncate(true),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("{:>3.0}%", progress * 100.0))
                        .monospace()
                        .small()
                        .color(TEXT_SECONDARY),
                );
            });
        });
        let (rect, resp) =
            ui.allocate_exact_size(Vec2::new(ui.available_width().max(60.0), 4.0), Sense::hover());
        let p = ui.painter();
        p.rect_filled(rect, R_PILL, BG_SUNKEN);
        let mut fill = rect;
        fill.set_width(rect.width() * progress.clamp(0.0, 1.0));
        p.rect_filled(fill, R_PILL, ACCENT);
        resp
    })
    .inner
}

/// Compact GPU / VRAM style read-out; the value shrinks on narrow panels.
pub fn meter(ui: &mut Ui, label: &str, value: f32, unit: &str) {
    let size = if breakpoint(ui).is_compact() { 14.0 } else { 16.0 };
    ui.vertical(|ui| {
        ui.add(
            egui::Label::new(
                RichText::new(label.to_uppercase())
                    .small()
                    .color(TEXT_DISABLED),
            )
            .truncate(true),
        );
        ui.add(
            egui::Label::new(
                RichText::new(format!("{value:.1}{unit}"))
                    .monospace()
                    .size(size)
                    .color(TEXT_PRIMARY),
            )
            .truncate(true),
        );
    });
}

/// Meters in a grid that reflows from one row to several as width shrinks.
pub fn meter_grid(ui: &mut Ui, meters: &[(&str, f32, &str)]) {
    grid(ui, meters.len(), 92.0, 160.0, |ui, i, w| {
        let (label, value, unit) = meters[i];
        ui.allocate_ui_with_layout(Vec2::new(w, 0.0), Layout::top_down(Align::Min), |ui| {
            ui.set_width(w);
            meter(ui, label, value, unit);
        });
    });
}
