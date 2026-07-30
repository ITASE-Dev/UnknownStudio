use crate::ui::responsive::{breakpoint, fill_w, label_col};
use crate::ui::theme::tokens::*;
use eframe::egui::{
    self, Align2, Color32, FontFamily, FontId, Margin, Response, RichText, Stroke, Ui, Vec2,
};

fn focus_ring(ui: &Ui, resp: &Response) {
    if resp.has_focus() {
        ui.painter()
            .rect_stroke(resp.rect.expand(1.0), R, Stroke::new(2.0_f32, ACCENT));
    }
}

/// Single-line field filling the row, with an accent focus ring.
pub fn pro_text_input(ui: &mut Ui, value: &mut String, hint: &str) -> Response {
    let w = fill_w(ui, 96.0, f32::INFINITY);
    pro_text_input_sized(ui, value, hint, w)
}

pub fn pro_text_input_sized(ui: &mut Ui, value: &mut String, hint: &str, width: f32) -> Response {
    let resp = ui.add(
        egui::TextEdit::singleline(value)
            .hint_text(RichText::new(hint).color(TEXT_DISABLED))
            .text_color(TEXT_PRIMARY)
            .desired_width(width)
            .margin(Margin::symmetric(9.0, 6.0))
            .frame(true),
    );
    focus_ring(ui, &resp);
    resp
}

/// Multi-line field filling the row; one row shorter when compact.
pub fn pro_text_area(ui: &mut Ui, value: &mut String, hint: &str, rows: usize) -> Response {
    let rows = if breakpoint(ui).is_compact() {
        rows.saturating_sub(1).max(2)
    } else {
        rows
    };
    let resp = ui.add(
        egui::TextEdit::multiline(value)
            .hint_text(RichText::new(hint).color(TEXT_DISABLED))
            .text_color(TEXT_PRIMARY)
            .desired_rows(rows)
            .desired_width(f32::INFINITY)
            .margin(Margin::symmetric(9.0, 7.0)),
    );
    focus_ring(ui, &resp);
    resp
}

/// Pill search field: grows with the row up to a comfortable reading measure.
pub fn search_input(ui: &mut Ui, value: &mut String, hint: &str) -> Response {
    let w = fill_w(ui, 132.0, 360.0);
    search_input_sized(ui, value, hint, w)
}

/// Pill field with a painted magnifier glyph and an inline clear affordance.
pub fn search_input_sized(ui: &mut Ui, value: &mut String, hint: &str, width: f32) -> Response {
    let width = width.max(96.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, CTRL_H), egui::Sense::hover());
    let p = ui.painter().clone();
    p.rect(rect, R_PILL, BG_SUNKEN, hairline());

    let c = egui::Pos2::new(rect.left() + 15.0, rect.center().y - 1.0);
    p.circle_stroke(c, 4.5, Stroke::new(1.3_f32, TEXT_DISABLED));
    p.line_segment(
        [
            egui::Pos2::new(c.x + 3.4, c.y + 3.4),
            egui::Pos2::new(c.x + 7.0, c.y + 7.0),
        ],
        Stroke::new(1.3_f32, TEXT_DISABLED),
    );

    let clear_w = if value.is_empty() { 10.0 } else { 24.0 };
    let mut field = rect.shrink2(Vec2::new(0.0, 4.0));
    field.set_left(rect.left() + 26.0);
    field.set_right(rect.right() - clear_w);

    let resp = ui
        .allocate_ui_at_rect(field, |ui| {
            ui.add(
                egui::TextEdit::singleline(value)
                    .hint_text(RichText::new(hint).color(TEXT_DISABLED))
                    .text_color(TEXT_PRIMARY)
                    .desired_width(field.width())
                    .margin(Margin::symmetric(0.0, 2.0))
                    .frame(false),
            )
        })
        .inner;

    if resp.has_focus() {
        p.rect_stroke(rect, R_PILL, Stroke::new(1.5_f32, ACCENT));
    }
    if !value.is_empty() {
        let x = egui::Pos2::new(rect.right() - 14.0, rect.center().y);
        let clear = ui.interact(
            egui::Rect::from_center_size(x, Vec2::splat(16.0)),
            resp.id.with("clear"),
            egui::Sense::click(),
        );
        let col = if clear.hovered() { TEXT_PRIMARY } else { TEXT_DISABLED };
        p.circle_filled(x, 6.5, Color32::from_white_alpha(18));
        p.text(x, Align2::CENTER_CENTER, "×", FontId::new(12.0, FontFamily::Proportional), col);
        if clear.clicked() {
            value.clear();
        }
    }
    resp
}

/// Thin-track slider that spans the row, with an optional mono read-out.
pub fn pro_slider(
    ui: &mut Ui,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    suffix: &str,
) -> Response {
    ui.horizontal(|ui| {
        let readout = if suffix.is_empty() { 0.0 } else { 52.0 };
        let track = (ui.available_width() - readout).max(56.0);
        ui.spacing_mut().slider_width = track;
        let resp = ui.add(
            egui::Slider::new(value, range)
                .show_value(false)
                .trailing_fill(true),
        );
        if !suffix.is_empty() {
            ui.label(
                RichText::new(format!("{:.0}{suffix}", *value))
                    .monospace()
                    .color(TEXT_PRIMARY),
            );
        }
        resp
    })
    .inner
}

/// `label ─ slider ─ value` row. Stacks the label above the track when compact.
pub fn slider_row(
    ui: &mut Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    suffix: &str,
) -> Response {
    if breakpoint(ui).is_compact() {
        return ui
            .vertical(|ui| {
                ui.add(
                    egui::Label::new(RichText::new(label).small().color(TEXT_SECONDARY))
                        .truncate(true),
                );
                pro_slider(ui, value, range, suffix)
            })
            .inner;
    }
    ui.horizontal(|ui| {
        let lw = label_col(ui);
        ui.add_sized(
            Vec2::new(lw, 20.0),
            egui::Label::new(RichText::new(label).color(TEXT_SECONDARY)).truncate(true),
        );
        pro_slider(ui, value, range, suffix)
    })
    .inner
}
