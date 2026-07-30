use crate::ui::core::buttons::{icon_button_painted, pro_button, Icon};
use crate::ui::core::inputs::pro_text_area;
use crate::ui::responsive::breakpoint;
use crate::ui::theme::tokens::*;
use eframe::egui::{
    self, Align, Layout, Margin, Pos2, Response, RichText, Rounding, Sense, Stroke, Ui, Vec2,
};

/// Bubble measure: most of the row, capped so long lines stay readable.
pub fn bubble_width(ui: &Ui) -> f32 {
    let avail = ui.available_width();
    (avail * if breakpoint(ui).is_compact() { 0.96 } else { 0.82 }).clamp(120.0, 620.0).min(avail)
}

/// Left-aligned assistant bubble: elevated plate, accent hairline, squared top-left.
pub fn ai_chat_bubble(ui: &mut Ui, author: &str, body: &str) -> Response {
    let w = bubble_width(ui);
    ai_chat_bubble_sized(ui, author, body, w)
}

pub fn ai_chat_bubble_sized(ui: &mut Ui, author: &str, body: &str, max_width: f32) -> Response {
    let max_width = max_width.min(ui.available_width()).max(80.0);
    ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
        ui.set_max_width(max_width);
        egui::Frame::none()
            .fill(BG_ELEVATED)
            .stroke(Stroke::new(1.0_f32, ACCENT.linear_multiply(0.35)))
            .rounding(Rounding { nw: 2.0, ne: 6.0, sw: 6.0, se: 6.0 })
            .inner_margin(Margin::symmetric(12.0, 10.0))
            .show(ui, |ui| {
                ui.set_max_width(max_width - 24.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(author).small().strong().color(ACCENT));
                    ui.add_space(2.0);
                    ui.label(RichText::new(body).color(TEXT_PRIMARY));
                });
            })
            .response
    })
    .inner
}

/// Right-aligned user bubble: sunken plate, squared top-right.
pub fn user_chat_bubble(ui: &mut Ui, body: &str) -> Response {
    let w = bubble_width(ui);
    user_chat_bubble_sized(ui, body, w)
}

pub fn user_chat_bubble_sized(ui: &mut Ui, body: &str, max_width: f32) -> Response {
    let max_width = max_width.min(ui.available_width()).max(80.0);
    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
        egui::Frame::none()
            .fill(BG_SUNKEN)
            .stroke(hairline())
            .rounding(Rounding { nw: 6.0, ne: 2.0, sw: 6.0, se: 6.0 })
            .inner_margin(Margin::symmetric(12.0, 10.0))
            .show(ui, |ui| {
                ui.set_max_width(max_width - 24.0);
                ui.label(RichText::new(body).color(TEXT_SECONDARY));
            })
            .response
    })
    .inner
}

/// Three-dot "thinking" indicator; `time` drives the phase offsets.
pub fn typing_indicator(ui: &mut Ui, time: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(58.0, 26.0), Sense::hover());
    let p = ui.painter();
    p.rect(rect, R, BG_ELEVATED, hairline());
    for i in 0..3 {
        let phase = time * 4.0 - i as f32 * 0.5;
        let a = 0.35 + 0.65 * (phase.sin() * 0.5 + 0.5);
        p.circle_filled(
            Pos2::new(rect.left() + 16.0 + i as f32 * 13.0, rect.center().y),
            3.0,
            ACCENT.linear_multiply(a),
        );
    }
    resp
}

/// Composer: prompt field + action row. Secondary affordances drop out when the
/// row is too narrow to hold them beside Send.
/// Returns `true` on submit (Send pressed, or ⌘/Ctrl+Enter) and clears `text`.
pub fn prompt_input_area(ui: &mut Ui, text: &mut String) -> bool {
    let mut submitted = false;
    let compact = breakpoint(ui).is_compact();
    egui::Frame::none()
        .fill(BG_PANEL)
        .stroke(hairline())
        .rounding(R)
        .inner_margin(Margin::same(if compact { 6.0 } else { 8.0 }))
        .show(ui, |ui| {
            let field = pro_text_area(
                ui,
                text,
                "Direct the edit… e.g. “tighten the intro to 20s”",
                3,
            );
            let hotkey = field.has_focus()
                && ui.input(|i| {
                    i.key_pressed(egui::Key::Enter) && (i.modifiers.command || i.modifiers.ctrl)
                });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                icon_button_painted(ui, Icon::Plus, true, false).on_hover_text("Attach media");
                if !compact {
                    icon_button_painted(ui, Icon::Mic, true, false).on_hover_text("Dictate");
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add_enabled_ui(!text.trim().is_empty(), |ui| {
                        if pro_button(ui, "Send", true).clicked() {
                            submitted = true;
                        }
                    });
                    if ui.available_width() > 40.0 {
                        ui.label(RichText::new("⌘↩").small().color(TEXT_DISABLED));
                    }
                });
            });
            if hotkey && !text.trim().is_empty() {
                submitted = true;
            }
        });
    if submitted {
        text.clear();
    }
    submitted
}

/// Scrolling transcript that keeps the composer pinned to the bottom edge.
pub fn chat_column<R>(ui: &mut Ui, composer_h: f32, transcript: impl FnOnce(&mut Ui) -> R) {
    let h = (ui.available_height() - composer_h).max(80.0);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .max_height(h)
        .stick_to_bottom(true)
        .show(ui, transcript);
}
