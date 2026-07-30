use crate::ui::theme::tokens::*;
use eframe::egui::{self, Color32, Pos2, Response, RichText, Sense, Stroke, Ui, Vec2};

/// iOS-style pill switch. Toggles `on` and marks the response changed.
pub fn pro_toggle(ui: &mut Ui, on: &mut bool) -> Response {
    let (rect, mut resp) = ui.allocate_exact_size(Vec2::new(40.0, 22.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool(resp.id, *on);
    let track = if *on {
        ACCENT.linear_multiply(0.35 + 0.65 * t)
    } else {
        BG_ELEVATED
    };
    let p = ui.painter();
    p.rect(rect, R_PILL, track, hairline());
    let r = rect.height() / 2.0 - 3.0;
    let cx = egui::lerp((rect.left() + r + 3.0)..=(rect.right() - r - 3.0), t);
    p.circle_filled(Pos2::new(cx, rect.center().y), r, Color32::WHITE);
    resp
}

/// Switch with a trailing label whose contrast follows the state.
pub fn pro_toggle_row(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    ui.horizontal(|ui| {
        let resp = pro_toggle(ui, on);
        ui.add_space(4.0);
        let color = if *on { TEXT_PRIMARY } else { TEXT_SECONDARY };
        ui.label(RichText::new(label).color(color));
        resp
    })
    .inner
}

/// Flat square checkbox, painted so the tick stays crisp at 13px.
pub fn pro_checkbox(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    ui.horizontal(|ui| {
        let (rect, mut resp) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::click());
        if resp.clicked() {
            *on = !*on;
            resp.mark_changed();
        }
        let p = ui.painter();
        let fill = if *on { ACCENT } else { BG_SUNKEN };
        let stroke = if *on {
            Stroke::new(1.0_f32, ACCENT)
        } else if resp.hovered() {
            Stroke::new(1.0_f32, BORDER_STRONG)
        } else {
            hairline()
        };
        p.rect(rect, R_SM, fill, stroke);
        if *on {
            let c = rect.center();
            p.line_segment(
                [Pos2::new(c.x - 4.0, c.y), Pos2::new(c.x - 1.0, c.y + 3.2)],
                Stroke::new(1.8_f32, Color32::WHITE),
            );
            p.line_segment(
                [Pos2::new(c.x - 1.0, c.y + 3.2), Pos2::new(c.x + 4.2, c.y - 3.0)],
                Stroke::new(1.8_f32, Color32::WHITE),
            );
        }
        ui.add_space(2.0);
        ui.label(RichText::new(label).color(TEXT_PRIMARY));
        resp
    })
    .inner
}
