use crate::ui::responsive::{breakpoint, label_col};
use crate::ui::theme::tokens::*;
use eframe::egui::{self, Align, Layout, Response, RichText, Rounding, Sense, Ui, Vec2};

/// Uppercase gray group label used above inspector blocks.
pub fn section_header(ui: &mut Ui, text: &str) -> Response {
    ui.add_space(2.0);
    let resp = ui.add(
        egui::Label::new(
            RichText::new(text.to_uppercase())
                .small()
                .strong()
                .color(TEXT_DISABLED),
        )
        .truncate(true),
    );
    ui.add_space(4.0);
    resp
}

/// Section title with caption and a hairline rule underneath.
/// The caption is dropped on compact widths where it would dominate.
pub fn section_title(ui: &mut Ui, title: &str, caption: &str) {
    let compact = breakpoint(ui).is_compact();
    ui.add_space(if compact { 6.0 } else { 10.0 });
    ui.label(RichText::new(title).heading().color(TEXT_PRIMARY).strong());
    if !caption.is_empty() && !compact {
        ui.label(RichText::new(caption).small().color(TEXT_SECONDARY));
    }
    ui.add_space(6.0);
    hairline_rule(ui);
    ui.add_space(if compact { 6.0 } else { 10.0 });
}

pub fn hairline_rule(ui: &mut Ui) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 1.0), Sense::hover());
    ui.painter().rect_filled(rect, Rounding::ZERO, BORDER);
}

/// Key on the left, value right-aligned; both truncate before overflowing.
pub fn property_row(ui: &mut Ui, key: &str, value: &str) -> Response {
    ui.horizontal(|ui| {
        let lw = label_col(ui);
        ui.add_sized(
            Vec2::new(lw, 20.0),
            egui::Label::new(RichText::new(key).color(TEXT_SECONDARY)).truncate(true),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add(
                egui::Label::new(RichText::new(value).monospace().color(TEXT_PRIMARY))
                    .truncate(true),
            )
        })
        .inner
    })
    .inner
}

/// Key on the left, caller-supplied control filling the rest of the row.
pub fn property_row_with<R>(ui: &mut Ui, key: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        let lw = label_col(ui);
        ui.add_sized(
            Vec2::new(lw, 20.0),
            egui::Label::new(RichText::new(key).color(TEXT_SECONDARY)).truncate(true),
        );
        add(ui)
    })
    .inner
}

pub fn caption(ui: &mut Ui, text: &str) -> Response {
    ui.label(RichText::new(text).small().color(TEXT_SECONDARY))
}

pub fn mono(ui: &mut Ui, text: &str) -> Response {
    ui.label(RichText::new(text).monospace().color(TEXT_SECONDARY))
}

/// Bordered panel grouping a block of controls; margin tightens when compact.
pub fn panel<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let m = if breakpoint(ui).is_compact() { 10.0 } else { 14.0 };
    egui::Frame::none()
        .fill(BG_PANEL)
        .stroke(hairline())
        .rounding(R)
        .inner_margin(egui::Margin::same(m))
        .show(ui, add)
        .inner
}

/// Centers content and caps its measure so text stays readable on wide windows.
pub fn content_column<R>(ui: &mut Ui, max_width: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let w = ui.available_width().min(max_width);
    let pad = ((ui.available_width() - w) * 0.5).max(0.0);
    ui.horizontal_top(|ui| {
        ui.add_space(pad);
        ui.allocate_ui_with_layout(
            Vec2::new(w, 0.0),
            Layout::top_down(Align::Min),
            |ui| {
                ui.set_width(w);
                add(ui)
            },
        )
        .inner
    })
    .inner
}
