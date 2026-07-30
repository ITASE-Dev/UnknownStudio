use crate::ui::responsive::{breakpoint, elided_galley};
use crate::ui::theme::tokens::*;
use eframe::egui::{
    self, Align2, Color32, FontFamily, FontId, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2,
};

/// 16:9 sunken preview plate filling the available width.
pub fn thumbnail_preview_box(ui: &mut Ui, caption: &str) -> Response {
    let w = ui.available_width();
    thumbnail_preview_box_sized(ui, caption, w)
}

/// 16:9 plate that also respects a height budget, so it never overflows a panel.
pub fn thumbnail_preview_box_fit(ui: &mut Ui, caption: &str, max_height: f32) -> Response {
    let w = ui.available_width().min(max_height * 16.0 / 9.0);
    thumbnail_preview_box_sized(ui, caption, w)
}

/// 16:9 preview plate at an explicit width, with mock framing guides.
pub fn thumbnail_preview_box_sized(ui: &mut Ui, caption: &str, width: f32) -> Response {
    let width = width.max(64.0);
    let rect_size = Vec2::new(width, (width * 9.0 / 16.0).round());
    let (rect, resp) = ui.allocate_exact_size(rect_size, Sense::click());
    let p = ui.painter().clone();

    let stroke = if resp.hovered() {
        Stroke::new(1.0_f32, BORDER_STRONG)
    } else {
        hairline()
    };
    p.rect(rect, R, BG_SUNKEN, stroke);

    // Thirds guides.
    let g = Stroke::new(1.0_f32, Color32::from_white_alpha(8));
    for i in 1..3 {
        let f = i as f32 / 3.0;
        let x = egui::lerp(rect.left()..=rect.right(), f);
        let y = egui::lerp(rect.top()..=rect.bottom(), f);
        p.line_segment([Pos2::new(x, rect.top() + 4.0), Pos2::new(x, rect.bottom() - 4.0)], g);
        p.line_segment([Pos2::new(rect.left() + 4.0, y), Pos2::new(rect.right() - 4.0, y)], g);
    }

    // Center crosshair, scaled to the plate.
    let c = rect.center();
    let a = (rect.height() * 0.08).clamp(4.0, 10.0);
    let cross = Stroke::new(1.0_f32, Color32::from_white_alpha(22));
    p.line_segment([Pos2::new(c.x - a, c.y), Pos2::new(c.x + a, c.y)], cross);
    p.line_segment([Pos2::new(c.x, c.y - a), Pos2::new(c.x, c.y + a)], cross);

    if !caption.is_empty() && rect.height() > 62.0 {
        let tag = Rect::from_min_size(
            Pos2::new(rect.left() + 6.0, rect.bottom() - 22.0),
            Vec2::new(rect.width() - 12.0, 16.0),
        );
        p.rect_filled(tag, R_SM, Color32::from_black_alpha(150));
        let galley = elided_galley(
            ui,
            caption,
            FontId::new(10.0, FontFamily::Monospace),
            TEXT_SECONDARY,
            tag.width() - 12.0,
        );
        p.galley(
            Pos2::new(tag.left() + 6.0, tag.center().y - galley.size().y / 2.0),
            galley,
            TEXT_SECONDARY,
        );
    }
    if rect.width() > 120.0 {
        p.text(
            Pos2::new(rect.right() - 8.0, rect.top() + 8.0),
            Align2::RIGHT_TOP,
            "16:9",
            FontId::new(9.0, FontFamily::Monospace),
            TEXT_DISABLED,
        );
    }
    resp
}

/// Grouped inspector block: uppercase header, hairline plate, caller content.
pub fn inspector_group<R>(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    use crate::ui::core::typography::section_header;
    section_header(ui, title);
    let m = if breakpoint(ui).is_compact() {
        egui::Margin::symmetric(8.0, 8.0)
    } else {
        egui::Margin::symmetric(12.0, 10.0)
    };
    egui::Frame::none()
        .fill(BG_PANEL)
        .stroke(hairline())
        .rounding(R)
        .inner_margin(m)
        .show(ui, add)
        .inner
}

/// Sidebar width that tracks the window: a share of it, within sane bounds.
pub fn inspector_width(ui: &Ui) -> f32 {
    (ui.available_width() * 0.28).clamp(180.0, 340.0)
}
