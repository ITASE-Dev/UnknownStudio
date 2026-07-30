use super::{sparkle, waveform};
use crate::ui::theme::tokens::*;
use eframe::egui::{
    self, Color32, CursorIcon, FontFamily, FontId, Pos2, Rect, Response, Rounding, Sense, Stroke,
    Ui, Vec2,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ClipKind {
    ARoll,
    BRoll,
    Audio,
}

impl ClipKind {
    pub fn color(self) -> Color32 {
        match self {
            Self::ARoll => CLIP_AROLL,
            Self::BRoll => CLIP_BROLL,
            Self::Audio => CLIP_AUDIO,
        }
    }
}

/// Blue camera clip.
pub fn a_roll_clip(ui: &mut Ui, rect: Rect, label: &str, selected: bool) -> Response {
    clip_block(ui, rect, ClipKind::ARoll, label, selected)
}

/// Purple generative clip, sparkle-badged.
pub fn b_roll_clip(ui: &mut Ui, rect: Rect, label: &str, selected: bool) -> Response {
    clip_block(ui, rect, ClipKind::BRoll, label, selected)
}

/// Green audio clip with a mock waveform.
pub fn audio_waveform_clip(ui: &mut Ui, rect: Rect, label: &str, selected: bool) -> Response {
    clip_block(ui, rect, ClipKind::Audio, label, selected)
}

/// Reserve `rect`, sense click/drag, then paint the block directly.
pub fn clip_block(
    ui: &mut Ui,
    rect: Rect,
    kind: ClipKind,
    label: &str,
    selected: bool,
) -> Response {
    let resp = ui.allocate_rect(rect, Sense::click_and_drag());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::Grab);
    }
    paint_clip(ui.painter(), rect, kind, label, selected, resp.hovered());
    resp.on_hover_text(label)
}

/// Painter-only clip body. Badges, waveform density and the label all scale with
/// `rect`; the label elides and disappears entirely on very short clips.
pub fn paint_clip(
    p: &egui::Painter,
    rect: Rect,
    kind: ClipKind,
    label: &str,
    selected: bool,
    hovered: bool,
) {
    let base = kind.color();
    let body = if hovered { base.linear_multiply(1.12) } else { base };
    p.rect_filled(rect, R_SM, body);
    p.rect_filled(
        Rect::from_min_size(rect.min, Vec2::new(rect.width(), 3.0)),
        top_rounding(4.0),
        Color32::from_white_alpha(38),
    );

    let badge_w = match kind {
        ClipKind::BRoll if rect.width() >= 26.0 => {
            sparkle(
                p,
                Pos2::new(rect.left() + 13.0, rect.center().y + 1.0),
                (rect.height() * 0.22).clamp(4.0, 6.0),
                Color32::WHITE,
            );
            24.0
        }
        ClipKind::Audio => {
            waveform(
                p,
                rect.shrink2(Vec2::new(
                    (rect.width() * 0.06).clamp(2.0, 6.0),
                    (rect.height() * 0.2).clamp(4.0, 8.0),
                )),
                base,
            );
            0.0
        }
        _ => 0.0,
    };

    p.rect_stroke(
        rect,
        R_SM,
        Stroke::new(
            if selected { 2.0_f32 } else { 1.0_f32 },
            if selected { Color32::WHITE } else { Color32::from_black_alpha(90) },
        ),
    );

    let text_x = rect.left() + if badge_w > 0.0 { badge_w } else { 8.0 };
    let text_w = rect.right() - 6.0 - text_x;
    if text_w >= 26.0 && rect.height() >= 16.0 {
        let mut job = egui::text::LayoutJob::simple_singleline(
            label.to_owned(),
            FontId::new(11.0, FontFamily::Proportional),
            Color32::WHITE,
        );
        job.wrap = egui::text::TextWrapping {
            max_width: text_w,
            max_rows: 1,
            break_anywhere: true,
            overflow_character: Some('…'),
        };
        let galley = p.layout_job(job);
        p.galley(
            Pos2::new(text_x, rect.center().y - galley.size().y / 2.0),
            galley,
            Color32::WHITE,
        );
    }
}

/// Trim handles at both clip edges; returns the edge grab response, if any.
/// Handle width follows the clip so short clips stay clickable but not covered.
pub fn clip_trim_handles(ui: &mut Ui, rect: Rect, id: egui::Id) -> Option<Response> {
    let w = (rect.width() * 0.18).clamp(3.0, 6.0);
    let left = Rect::from_min_size(rect.min, Vec2::new(w, rect.height()));
    let right = Rect::from_min_size(
        Pos2::new(rect.right() - w, rect.top()),
        Vec2::new(w, rect.height()),
    );
    let mut grabbed = None;
    for (i, r) in [left, right].into_iter().enumerate() {
        let resp = ui.interact(r, id.with(i), Sense::drag());
        if resp.hovered() || resp.dragged() {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
            ui.painter()
                .rect_filled(r, Rounding::ZERO, Color32::from_white_alpha(70));
            grabbed = Some(resp);
        }
    }
    grabbed
}
