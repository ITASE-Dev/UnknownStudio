use super::{seconds_at, x_at};
use crate::ui::theme::tokens::*;
use eframe::egui::{
    self, Align2, Color32, CursorIcon, FontFamily, FontId, Pos2, Rect, Response, Rounding, Sense,
    Stroke, Ui, Vec2,
};

/// Vertical red playhead with a draggable top handle.
/// Position is kept in **seconds**, so it survives resize, zoom and scrolling.
pub fn playhead_marker(ui: &mut Ui, area: Rect, seconds: &mut f32, px_per_sec: f32) -> Response {
    let x = x_at(area, *seconds, px_per_sec);
    let handle = Rect::from_center_size(Pos2::new(x, area.top() + 6.0), Vec2::new(16.0, 14.0));
    let mut resp = ui.interact(handle, ui.id().with("playhead"), Sense::click_and_drag());
    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
    if resp.dragged() {
        let max_s = seconds_at(area, area.right(), px_per_sec);
        *seconds = (*seconds + resp.drag_delta().x / px_per_sec.max(0.001)).clamp(0.0, max_s);
        resp.mark_changed();
    }

    let px = x_at(area, *seconds, px_per_sec).round();
    let p = ui.painter();
    p.line_segment(
        [Pos2::new(px, area.top()), Pos2::new(px, area.bottom())],
        Stroke::new(1.0_f32, ERR),
    );
    p.add(egui::Shape::convex_polygon(
        vec![
            Pos2::new(px - 6.0, area.top()),
            Pos2::new(px + 6.0, area.top()),
            Pos2::new(px + 6.0, area.top() + 8.0),
            Pos2::new(px, area.top() + 13.0),
            Pos2::new(px - 6.0, area.top() + 8.0),
        ],
        ERR,
        Stroke::NONE,
    ));
    resp
}

/// Timecode chip anchored to the playhead, clamped to stay inside `area`.
pub fn playhead_timecode(ui: &mut Ui, area: Rect, seconds: f32, px_per_sec: f32) {
    let label = format!(
        "{:02}:{:02}:{:02}",
        (seconds as i32) / 60,
        (seconds as i32) % 60,
        ((seconds.fract() * 30.0) as i32).min(29)
    );
    let w = 62.0;
    let cx = x_at(area, seconds, px_per_sec).clamp(area.left() + w * 0.5, area.right() - w * 0.5);
    let chip = Rect::from_center_size(Pos2::new(cx, area.top() - 10.0), Vec2::new(w, 16.0));
    let p = ui.painter();
    p.rect(chip, R_SM, BG_SUNKEN, Stroke::new(1.0_f32, ERR.linear_multiply(0.6)));
    p.text(
        chip.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::new(10.0, FontFamily::Monospace),
        ERR,
    );
}

/// Vertical dashed red line marking a pending ripple cut.
/// `index` keys the interaction so hover survives layout changes.
pub fn ripple_cut_marker(
    ui: &mut Ui,
    area: Rect,
    seconds: f32,
    px_per_sec: f32,
    index: usize,
) -> Response {
    let px = x_at(area, seconds, px_per_sec).round();
    let hit = Rect::from_center_size(Pos2::new(px, area.center().y), Vec2::new(9.0, area.height()));
    let resp = ui.interact(hit, ui.id().with(("ripple", index)), Sense::click());
    let color = if resp.hovered() { ERR } else { ERR.linear_multiply(0.75) };

    let p = ui.painter();
    for shape in egui::Shape::dashed_line(
        &[Pos2::new(px, area.top()), Pos2::new(px, area.bottom())],
        Stroke::new(1.0_f32, color),
        4.0,
        3.0,
    ) {
        p.add(shape);
    }
    let tag = Rect::from_min_size(Pos2::new(px - 7.0, area.top()), Vec2::new(14.0, 10.0));
    p.rect_filled(tag, Rounding { nw: 0.0, ne: 0.0, sw: 2.0, se: 2.0 }, color);
    p.line_segment(
        [Pos2::new(px - 3.0, area.top() + 3.0), Pos2::new(px + 3.0, area.top() + 7.0)],
        Stroke::new(1.2_f32, Color32::BLACK),
    );
    p.line_segment(
        [Pos2::new(px + 3.0, area.top() + 3.0), Pos2::new(px - 3.0, area.top() + 7.0)],
        Stroke::new(1.2_f32, Color32::BLACK),
    );
    resp.on_hover_text("Ripple cut")
}

/// In / out range shading with solid yellow edges, in seconds.
pub fn range_marker(ui: &mut Ui, area: Rect, in_s: f32, out_s: f32, px_per_sec: f32) {
    let x_in = x_at(area, in_s.min(out_s), px_per_sec);
    let x_out = x_at(area, out_s.max(in_s), px_per_sec);
    let band = Rect::from_min_max(
        Pos2::new(x_in, area.top()),
        Pos2::new(x_out, area.bottom()),
    );
    let p = ui.painter();
    p.rect_filled(band, Rounding::ZERO, WARN.linear_multiply(0.10));
    for x in [x_in, x_out] {
        p.line_segment(
            [Pos2::new(x, area.top()), Pos2::new(x, area.bottom())],
            Stroke::new(1.5_f32, WARN),
        );
    }
}
