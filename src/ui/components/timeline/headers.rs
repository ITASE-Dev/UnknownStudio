use super::{track_height, HEADER_W};
use crate::ui::responsive::elided_galley;
use crate::ui::theme::tokens::*;
use eframe::egui::{
    self, Color32, FontFamily, FontId, Pos2, Rect, Response, Rounding, Sense, Stroke, Ui, Vec2,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    Video,
    Audio,
}

/// Header width for the current viewport; collapses toward an icon rail.
pub fn header_width(ui: &Ui) -> f32 {
    (ui.available_width() * 0.22).clamp(46.0, HEADER_W)
}

/// Lane header (`V1`, `A1`, …) with painted lock + mute toggles.
/// Compute the width once per timeline so every lane aligns.
pub fn track_header(
    ui: &mut Ui,
    name: &str,
    kind: TrackKind,
    locked: &mut bool,
    muted: &mut bool,
) -> Response {
    let w = header_width(ui);
    track_header_sized(ui, name, kind, locked, muted, w, false)
}

/// Header at an explicit width. Detail drops out progressively as `width` shrinks:
/// subtitle first, then the icon row moves beside the label.
pub fn track_header_sized(
    ui: &mut Ui,
    name: &str,
    kind: TrackKind,
    locked: &mut bool,
    muted: &mut bool,
    width: f32,
    selected: bool,
) -> Response {
    let width = width.max(40.0);
    let h = track_height(ui);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, h), Sense::click());
    let p = ui.painter().clone();

    let show_subtitle = width >= 96.0 && h >= 48.0;
    let icons_below = width >= 88.0 && h >= 48.0;
    let show_icons = width >= 62.0;

    let fill = if selected { BG_SUNKEN } else { BG_PANEL };
    p.rect_filled(rect, Rounding::ZERO, fill);
    p.line_segment(
        [rect.right_top(), rect.right_bottom()],
        Stroke::new(1.0_f32, BORDER),
    );
    p.line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0_f32, Color32::from_black_alpha(90)),
    );

    let accent = match kind {
        TrackKind::Video => CLIP_AROLL,
        TrackKind::Audio => CLIP_AUDIO,
    };
    p.rect_filled(
        // The accent rail widens on the selected track: the toolbar's
        // reorder and remove actions apply to it.
        Rect::from_min_size(rect.min, Vec2::new(if selected { 5.0 } else { 3.0 }, rect.height())),
        Rounding::ZERO,
        accent.linear_multiply(if *muted { 0.35 } else { 1.0 }),
    );

    // Icons are laid out first so the label knows how much room is left.
    let icon_y = if icons_below { rect.bottom() - 15.0 } else { rect.center().y };
    let (lock_c, mute_c) = (
        Pos2::new(rect.right() - 46.0, icon_y),
        Pos2::new(rect.right() - 22.0, icon_y),
    );
    let label_right = if show_icons && !icons_below {
        rect.right() - 62.0
    } else {
        rect.right() - 8.0
    };

    let name_color = if *muted { TEXT_DISABLED } else { TEXT_PRIMARY };
    let label_x = rect.left() + 12.0;
    let galley = elided_galley(
        ui,
        name,
        FontId::new(12.0, FontFamily::Proportional),
        name_color,
        (label_right - label_x).max(10.0),
    );
    let label_y = if show_subtitle {
        rect.top() + 9.0
    } else {
        rect.center().y - galley.size().y / 2.0
    };
    p.galley(Pos2::new(label_x, label_y), galley, name_color);

    if show_subtitle {
        p.text(
            Pos2::new(label_x, rect.top() + 26.0),
            egui::Align2::LEFT_TOP,
            match kind {
                TrackKind::Video => "VIDEO",
                TrackKind::Audio => "AUDIO",
            },
            FontId::new(9.0, FontFamily::Monospace),
            TEXT_DISABLED,
        );
    }

    if show_icons {
        let lock_rect = Rect::from_center_size(lock_c, Vec2::splat(20.0));
        let mute_rect = Rect::from_center_size(mute_c, Vec2::splat(20.0));

        let lock_resp = ui.interact(lock_rect, resp.id.with("lock"), Sense::click());
        if lock_resp.clicked() {
            *locked = !*locked;
        }
        let mute_resp = ui.interact(mute_rect, resp.id.with("mute"), Sense::click());
        if mute_resp.clicked() {
            *muted = !*muted;
        }

        icon_plate(&p, lock_rect, *locked, lock_resp.hovered());
        lock_glyph(&p, lock_rect.center(), *locked);
        icon_plate(&p, mute_rect, *muted, mute_resp.hovered());
        mute_glyph(&p, mute_rect.center(), *muted);
    }

    resp.on_hover_text(name)
}

fn icon_plate(p: &egui::Painter, rect: Rect, on: bool, hovered: bool) {
    let fill = if on {
        ACCENT.linear_multiply(0.30)
    } else if hovered {
        Color32::from_white_alpha(14)
    } else {
        Color32::TRANSPARENT
    };
    p.rect_filled(rect.shrink(2.0), R_SM, fill);
}

fn glyph_color(on: bool) -> Color32 {
    if on { ACCENT } else { TEXT_SECONDARY }
}

fn lock_glyph(p: &egui::Painter, c: Pos2, locked: bool) {
    let col = glyph_color(locked);
    let body = Rect::from_min_size(Pos2::new(c.x - 4.5, c.y - 1.0), Vec2::new(9.0, 7.0));
    p.rect(body, Rounding::same(1.5), Color32::TRANSPARENT, Stroke::new(1.3_f32, col));
    let shackle_y = if locked { c.y - 1.0 } else { c.y - 2.5 };
    p.line_segment([Pos2::new(c.x - 2.5, shackle_y), Pos2::new(c.x - 2.5, c.y - 5.0)], Stroke::new(1.3_f32, col));
    p.line_segment([Pos2::new(c.x - 2.5, c.y - 5.0), Pos2::new(c.x + 2.5, c.y - 5.0)], Stroke::new(1.3_f32, col));
    p.line_segment([Pos2::new(c.x + 2.5, c.y - 5.0), Pos2::new(c.x + 2.5, shackle_y)], Stroke::new(1.3_f32, col));
}

fn mute_glyph(p: &egui::Painter, c: Pos2, muted: bool) {
    let col = if muted { ERR } else { TEXT_SECONDARY };
    p.add(egui::Shape::convex_polygon(
        vec![
            Pos2::new(c.x - 5.0, c.y - 2.5),
            Pos2::new(c.x - 2.0, c.y - 2.5),
            Pos2::new(c.x + 1.0, c.y - 5.5),
            Pos2::new(c.x + 1.0, c.y + 5.5),
            Pos2::new(c.x - 2.0, c.y + 2.5),
            Pos2::new(c.x - 5.0, c.y + 2.5),
        ],
        col,
        Stroke::NONE,
    ));
    if muted {
        p.line_segment([Pos2::new(c.x + 3.0, c.y - 3.5), Pos2::new(c.x + 7.0, c.y + 3.5)], Stroke::new(1.4_f32, col));
        p.line_segment([Pos2::new(c.x + 7.0, c.y - 3.5), Pos2::new(c.x + 3.0, c.y + 3.5)], Stroke::new(1.4_f32, col));
    } else {
        p.circle_stroke(Pos2::new(c.x + 1.0, c.y), 5.5, Stroke::new(1.2_f32, col.linear_multiply(0.7)));
    }
}
