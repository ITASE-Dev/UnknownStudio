pub mod clip;
pub mod headers;
pub mod markers;
pub mod tools;

use crate::ui::theme::tokens::*;
use eframe::egui::{
    self, Align2, Color32, FontFamily, FontId, Pos2, Rect, Response, Rounding, Sense, Stroke, Ui,
    Vec2,
};

pub const TRACK_H: f32 = 54.0;
pub const TRACK_H_COMPACT: f32 = 40.0;
pub const HEADER_W: f32 = 132.0;
pub const PX_PER_SEC: f32 = 26.0;
pub const PX_PER_SEC_MIN: f32 = 2.0;
pub const PX_PER_SEC_MAX: f32 = 240.0;

pub fn x_at(rect: Rect, seconds: f32, px_per_sec: f32) -> f32 {
    rect.left() + seconds * px_per_sec
}

pub fn seconds_at(rect: Rect, x: f32, px_per_sec: f32) -> f32 {
    ((x - rect.left()) / px_per_sec.max(0.001)).max(0.0)
}

/// Zoom that fits `seconds` into the available width.
pub fn fit_px_per_sec(ui: &Ui, seconds: f32) -> f32 {
    if seconds <= 0.0 {
        return PX_PER_SEC;
    }
    (ui.available_width() / seconds).clamp(PX_PER_SEC_MIN, PX_PER_SEC_MAX)
}

/// Zoom for a `zoom` factor, still never narrower than the visible width.
pub fn px_per_sec_for(ui: &Ui, seconds: f32, zoom: f32) -> f32 {
    (fit_px_per_sec(ui, seconds) * zoom.max(0.05)).clamp(PX_PER_SEC_MIN, PX_PER_SEC_MAX)
}

/// Lane height for the current width — shorter lanes on compact layouts.
pub fn track_height(ui: &Ui) -> f32 {
    if crate::ui::responsive::breakpoint(ui).is_compact() {
        TRACK_H_COMPACT
    } else {
        TRACK_H
    }
}

/// Timeline content width: the timeline span, but never less than the viewport.
pub fn content_width(ui: &Ui, seconds: f32, px_per_sec: f32) -> f32 {
    (seconds * px_per_sec).max(ui.available_width())
}

/// Smallest "nice" time step whose spacing is at least `min_px`.
fn nice_step(px_per_sec: f32, min_px: f32) -> f32 {
    const STEPS: [f32; 11] = [
        0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 300.0, 600.0,
    ];
    for s in STEPS {
        if s * px_per_sec >= min_px {
            return s;
        }
    }
    STEPS[STEPS.len() - 1]
}

fn timecode(s: f32, step: f32) -> String {
    if step < 1.0 {
        format!("{s:.1}s")
    } else {
        format!("{:02}:{:02}", (s as i32) / 60, (s as i32) % 60)
    }
}

/// Time ruler whose tick and label density adapt to the current zoom.
pub fn ruler(ui: &mut Ui, seconds: f32, px_per_sec: f32) -> Response {
    let w = content_width(ui, seconds, px_per_sec);
    // Drag as well as click: the ruler is the scrub surface.
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 20.0), Sense::click_and_drag());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    let p = ui.painter();
    p.rect_filled(rect, Rounding::ZERO, BG_SUNKEN);

    let minor = nice_step(px_per_sec, 7.0);
    let major = nice_step(px_per_sec, 56.0);
    // Step by index: accumulating floats drifts and drops major ticks.
    let per_major = (major / minor).round().max(1.0) as i32;

    for i in 0..=(seconds / minor).floor() as i32 {
        let s = i as f32 * minor;
        let x = x_at(rect, s, px_per_sec);
        let is_major = i % per_major == 0;
        p.line_segment(
            [
                Pos2::new(x, rect.bottom() - if is_major { 10.0 } else { 5.0 }),
                Pos2::new(x, rect.bottom()),
            ],
            Stroke::new(1.0_f32, if is_major { BORDER_STRONG } else { BORDER }),
        );
        if is_major {
            p.text(
                Pos2::new(x + 4.0, rect.top() + 2.0),
                Align2::LEFT_TOP,
                timecode(s, major),
                FontId::new(9.0, FontFamily::Monospace),
                TEXT_DISABLED,
            );
        }
    }
    p.line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0_f32, BORDER),
    );
    resp
}

/// Scrub position from a ruler interaction, clamped to the timeline span.
pub fn ruler_scrub(resp: &Response, seconds: f32, px_per_sec: f32) -> Option<f32> {
    if !(resp.clicked() || resp.dragged()) {
        return None;
    }
    let pointer = resp.interact_pointer_pos()?;
    Some(seconds_at(resp.rect, pointer.x, px_per_sec).clamp(0.0, seconds))
}

/// Empty lane spanning the viewport, with a grid that thins out as you zoom out.
pub fn track_lane(ui: &mut Ui, seconds: f32, px_per_sec: f32) -> (Rect, Response) {
    let size = Vec2::new(content_width(ui, seconds, px_per_sec), track_height(ui));
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let p = ui.painter();
    p.rect_filled(rect, Rounding::ZERO, BG_APP);

    let step = nice_step(px_per_sec, 40.0);
    for i in 0..=(seconds / step).floor() as i32 {
        let x = x_at(rect, i as f32 * step, px_per_sec);
        p.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            Stroke::new(1.0_f32, Color32::from_white_alpha(6)),
        );
    }
    p.line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0_f32, Color32::from_black_alpha(90)),
    );
    (rect, resp)
}

/// Clip rect inside a lane, in seconds — the geometry every clip helper expects.
pub fn clip_rect(lane: Rect, start: f32, len: f32, px_per_sec: f32) -> Rect {
    let inset = (lane.height() * 0.22).clamp(6.0, 14.0);
    Rect::from_min_size(
        Pos2::new(x_at(lane, start, px_per_sec), lane.top() + inset * 0.6),
        Vec2::new((len * px_per_sec - 2.0).max(2.0), lane.height() - inset),
    )
}

/// Deterministic mirrored-bar waveform; bar density follows the clip width.
pub fn waveform(p: &egui::Painter, rect: Rect, base: Color32) {
    waveform_peaks(p, rect, base, None);
}

/// Waveform from measured peaks when they are available, falling back to the
/// synthetic pattern while analysis is still running.
///
/// `peaks` covers the whole source file; `window` is the `[from, to)` fraction
/// of it this clip shows, so a trimmed clip draws its own section.
pub fn waveform_peaks(
    p: &egui::Painter,
    rect: Rect,
    base: Color32,
    peaks: Option<(&[f32], f32, f32)>,
) {
    let mid = rect.center().y;
    let (bar, gap) = if rect.width() < 90.0 { (2.0, 1.0) } else { (3.0, 2.0) };
    let n = ((rect.width() + gap) / (bar + gap)).floor().max(1.0) as usize;
    let col = Color32::from_white_alpha(200);
    for i in 0..n {
        let f = i as f32;
        let amplitude = match peaks {
            // Each bar takes the loudest peak in the slice it covers, so
            // transients survive downsampling to bar resolution.
            Some((peaks, from, to)) if !peaks.is_empty() => {
                let span = (to - from).max(f32::EPSILON);
                let lo = from + span * (i as f32 / n as f32);
                let hi = from + span * ((i + 1) as f32 / n as f32);
                let start = ((lo * peaks.len() as f32) as usize).min(peaks.len() - 1);
                let end = ((hi * peaks.len() as f32).ceil() as usize).clamp(start + 1, peaks.len());
                peaks[start..end].iter().fold(0.0f32, |m, v| m.max(*v))
            }
            _ => (f * 0.7).sin() * 0.5 + (f * 0.23).cos() * 0.35 + (f * 1.9).sin() * 0.15,
        };
        let h = (amplitude.abs() * rect.height() * 0.5).max(1.0);
        let x = rect.left() + f * (bar + gap);
        p.rect_filled(
            Rect::from_min_max(Pos2::new(x, mid - h), Pos2::new(x + bar, mid + h)),
            Rounding::same(1.0),
            col,
        );
    }
    p.line_segment(
        [Pos2::new(rect.left(), mid), Pos2::new(rect.right(), mid)],
        Stroke::new(1.0_f32, base.linear_multiply(1.4)),
    );
}

/// Four-point sparkle marking generative content.
pub fn sparkle(p: &egui::Painter, c: Pos2, r: f32, color: Color32) {
    p.add(egui::Shape::convex_polygon(
        vec![
            Pos2::new(c.x, c.y - r),
            Pos2::new(c.x + r * 0.28, c.y - r * 0.28),
            Pos2::new(c.x + r, c.y),
            Pos2::new(c.x + r * 0.28, c.y + r * 0.28),
            Pos2::new(c.x, c.y + r),
            Pos2::new(c.x - r * 0.28, c.y + r * 0.28),
            Pos2::new(c.x - r, c.y),
            Pos2::new(c.x - r * 0.28, c.y - r * 0.28),
        ],
        color,
        Stroke::NONE,
    ));
}
