use crate::ui::responsive::breakpoint;
use crate::ui::theme::tokens::*;
use eframe::egui::{self, Color32, Response, RichText, Stroke, Ui, Vec2};

/// Flat filled accent button when `is_primary`, hairline-outlined surface otherwise.
pub fn pro_button(ui: &mut Ui, text: &str, is_primary: bool) -> Response {
    pro_button_sized(ui, text, is_primary, Vec2::new(0.0, CTRL_H))
}

/// Same button stretched to the remaining row width — for compact stacked layouts.
pub fn pro_button_filled(ui: &mut Ui, text: &str, is_primary: bool) -> Response {
    let w = ui.available_width().max(64.0);
    pro_button_sized(ui, text, is_primary, Vec2::new(w, CTRL_H))
}

pub fn pro_button_sized(ui: &mut Ui, text: &str, is_primary: bool, min_size: Vec2) -> Response {
    let (fill, stroke, color) = if is_primary {
        (ACCENT, Stroke::NONE, Color32::WHITE)
    } else {
        (BG_ELEVATED, hairline(), TEXT_PRIMARY)
    };
    let label = if is_primary {
        RichText::new(text).color(color).strong()
    } else {
        RichText::new(text).color(color)
    };
    let resp = ui.add(
        egui::Button::new(label)
            .fill(fill)
            .stroke(stroke)
            .rounding(R)
            .wrap(false)
            .min_size(min_size),
    );
    if is_primary && resp.hovered() && ui.is_enabled() {
        ui.painter()
            .rect_filled(resp.rect, R, ACCENT_HOVER.linear_multiply(0.25));
    }
    resp
}

/// Purple variant reserved for generative / AI actions.
pub fn ai_button(ui: &mut Ui, text: &str) -> Response {
    let resp = ui.add(
        egui::Button::new(RichText::new(text).color(Color32::WHITE).strong())
            .fill(AI)
            .stroke(Stroke::NONE)
            .rounding(R)
            .wrap(false)
            .min_size(Vec2::new(0.0, CTRL_H)),
    );
    if resp.hovered() {
        ui.painter()
            .rect_filled(resp.rect, R, AI_HOVER.linear_multiply(0.25));
    }
    resp
}

pub fn danger_button(ui: &mut Ui, text: &str) -> Response {
    ui.add(
        egui::Button::new(RichText::new(text).color(ERR))
            .fill(BG_ELEVATED)
            .stroke(Stroke::new(1.0_f32, ERR.linear_multiply(0.55)))
            .rounding(R)
            .wrap(false)
            .min_size(Vec2::new(0.0, CTRL_H)),
    )
}

/// Square chromeless glyph button. `ghost` keeps a transparent plate until hover.
pub fn icon_button(ui: &mut Ui, glyph: &str, ghost: bool, active: bool) -> Response {
    let size = Vec2::splat(CTRL_H);
    let (fill, color) = match (active, ghost) {
        (true, _) => (ACCENT.linear_multiply(0.30), ACCENT),
        (false, true) => (Color32::TRANSPARENT, TEXT_SECONDARY),
        (false, false) => (BG_ELEVATED, TEXT_PRIMARY),
    };
    let stroke = if ghost { Stroke::NONE } else { hairline() };
    let resp = ui.add(
        egui::Button::new(RichText::new(glyph).color(color).size(14.0))
            .fill(fill)
            .stroke(stroke)
            .rounding(R_SM)
            .min_size(size),
    );
    if ghost && resp.hovered() {
        ui.painter()
            .rect_filled(resp.rect, R_SM, Color32::from_white_alpha(14));
    }
    resp
}

/// Icons painted from primitives, so they never depend on font coverage.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Play,
    Pause,
    Cut,
    Plus,
    Mic,
    Trash,
}

/// Square glyph button with a painted icon; `ghost` keeps the plate transparent.
pub fn icon_button_painted(ui: &mut Ui, icon: Icon, ghost: bool, active: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(CTRL_H), egui::Sense::click());
    let (fill, color) = match (active, ghost, resp.hovered()) {
        (true, _, _) => (ACCENT.linear_multiply(0.30), ACCENT),
        (false, true, true) => (Color32::from_white_alpha(14), TEXT_PRIMARY),
        (false, true, false) => (Color32::TRANSPARENT, TEXT_SECONDARY),
        (false, false, true) => (BG_HOVER, TEXT_PRIMARY),
        (false, false, false) => (BG_ELEVATED, TEXT_PRIMARY),
    };
    let p = ui.painter();
    p.rect(
        rect,
        R_SM,
        fill,
        if ghost { Stroke::NONE } else { hairline() },
    );
    paint_icon(p, rect.center(), icon, color);
    resp
}

/// Draw an icon centred on `c` inside a nominal 14px box.
pub fn paint_icon(p: &egui::Painter, c: egui::Pos2, icon: Icon, color: Color32) {
    use egui::Pos2;
    let s = Stroke::new(1.6_f32, color);
    match icon {
        Icon::Play => {
            p.add(egui::Shape::convex_polygon(
                vec![
                    Pos2::new(c.x - 3.5, c.y - 5.5),
                    Pos2::new(c.x - 3.5, c.y + 5.5),
                    Pos2::new(c.x + 5.5, c.y),
                ],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Pause => {
            for dx in [-3.0, 1.5] {
                p.rect_filled(
                    egui::Rect::from_min_size(Pos2::new(c.x + dx, c.y - 5.0), Vec2::new(2.5, 10.0)),
                    egui::Rounding::same(1.0),
                    color,
                );
            }
        }
        Icon::Cut => {
            p.line_segment([Pos2::new(c.x - 4.0, c.y - 6.0), Pos2::new(c.x + 3.5, c.y + 3.0)], s);
            p.line_segment([Pos2::new(c.x + 3.5, c.y - 6.0), Pos2::new(c.x - 4.0, c.y + 3.0)], s);
            p.circle_stroke(Pos2::new(c.x - 4.0, c.y + 5.0), 2.4, Stroke::new(1.4_f32, color));
            p.circle_stroke(Pos2::new(c.x + 4.0, c.y + 5.0), 2.4, Stroke::new(1.4_f32, color));
        }
        Icon::Plus => {
            p.line_segment([Pos2::new(c.x - 5.5, c.y), Pos2::new(c.x + 5.5, c.y)], s);
            p.line_segment([Pos2::new(c.x, c.y - 5.5), Pos2::new(c.x, c.y + 5.5)], s);
        }
        Icon::Mic => {
            p.rect(
                egui::Rect::from_center_size(Pos2::new(c.x, c.y - 2.0), Vec2::new(6.0, 9.0)),
                R_PILL,
                Color32::TRANSPARENT,
                Stroke::new(1.4_f32, color),
            );
            p.line_segment([Pos2::new(c.x, c.y + 4.0), Pos2::new(c.x, c.y + 6.5)], s);
            p.line_segment([Pos2::new(c.x - 3.5, c.y + 6.5), Pos2::new(c.x + 3.5, c.y + 6.5)], s);
        }
        Icon::Trash => {
            p.line_segment([Pos2::new(c.x - 5.0, c.y - 3.5), Pos2::new(c.x + 5.0, c.y - 3.5)], s);
            p.rect(
                egui::Rect::from_min_size(Pos2::new(c.x - 4.0, c.y - 3.0), Vec2::new(8.0, 9.0)),
                R_SM,
                Color32::TRANSPARENT,
                Stroke::new(1.4_f32, color),
            );
            p.line_segment([Pos2::new(c.x - 1.5, c.y - 6.0), Pos2::new(c.x + 1.5, c.y - 6.0)], s);
        }
    }
}

/// Chromeless text button for tertiary / inline actions.
pub fn ghost_button(ui: &mut Ui, text: &str) -> Response {
    ui.add(
        egui::Button::new(RichText::new(text).color(TEXT_SECONDARY))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::NONE)
            .rounding(R)
            .wrap(false)
            .min_size(Vec2::new(0.0, CTRL_H)),
    )
}

/// Final Cut style mode switch; segments share the row width evenly.
pub fn segmented(ui: &mut Ui, options: &[&str], selected: &mut usize) -> Response {
    let n = options.len().max(1) as f32;
    let outer = ui.available_width();
    egui::Frame::none()
        .fill(BG_SUNKEN)
        .stroke(hairline())
        .rounding(R)
        .inner_margin(egui::Margin::same(2.0))
        .show(ui, |ui| {
            let gap = 2.0;
            ui.spacing_mut().item_spacing.x = gap;
            let seg = ((outer - 8.0 - gap * (n - 1.0)) / n).clamp(46.0, 148.0);
            ui.horizontal(|ui| {
                for (i, opt) in options.iter().enumerate() {
                    let is_on = *selected == i;
                    let fill = if is_on { BG_ELEVATED } else { Color32::TRANSPARENT };
                    let color = if is_on { TEXT_PRIMARY } else { TEXT_SECONDARY };
                    let btn = egui::Button::new(RichText::new(*opt).color(color))
                        .fill(fill)
                        .stroke(Stroke::NONE)
                        .rounding(R_SM)
                        .wrap(false)
                        .min_size(Vec2::new(seg, 24.0));
                    if ui.add(btn).clicked() {
                        *selected = i;
                    }
                }
            })
            .response
        })
        .inner
}

/// Action row: side by side normally, stacked full-width when compact.
/// Each entry is `(label, is_primary)`; returns the index that was clicked.
pub fn action_row(ui: &mut Ui, actions: &[(&str, bool)]) -> Option<usize> {
    let mut clicked = None;
    if breakpoint(ui).is_compact() {
        ui.vertical(|ui| {
            for (i, (label, primary)) in actions.iter().enumerate() {
                if pro_button_filled(ui, label, *primary).clicked() {
                    clicked = Some(i);
                }
            }
        });
    } else {
        ui.horizontal_wrapped(|ui| {
            for (i, (label, primary)) in actions.iter().enumerate() {
                if pro_button(ui, label, *primary).clicked() {
                    clicked = Some(i);
                }
            }
        });
    }
    clicked
}
