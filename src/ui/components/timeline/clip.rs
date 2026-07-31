use super::{sparkle, waveform_peaks};
use crate::media::Poster;
use crate::ui::imaging::cover_uv;
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

/// Decoded frames sampled evenly across a source, plus the `[from, to)`
/// fraction of it a clip shows.
#[derive(Clone, Copy)]
pub struct Filmstrip<'a> {
    pub frames: &'a [Option<Poster>],
    pub from: f32,
    pub to: f32,
}

impl Filmstrip<'_> {
    /// Frame covering `fraction` of the clip, falling back to the nearest
    /// decoded neighbour while the strip is still filling in.
    fn at(&self, fraction: f32) -> Option<Poster> {
        if self.frames.is_empty() {
            return None;
        }
        let source = self.from + (self.to - self.from) * fraction.clamp(0.0, 1.0);
        let last = self.frames.len() - 1;
        let index = ((source * self.frames.len() as f32) as usize).min(last);

        if let Some(poster) = self.frames[index] {
            return Some(poster);
        }
        (0..=last)
            .flat_map(|offset| [index.saturating_sub(offset), (index + offset).min(last)])
            .find_map(|i| self.frames[i])
    }
}

/// Optional decoded content drawn inside a clip block.
#[derive(Default, Clone, Copy)]
pub struct ClipVisuals<'a> {
    /// Frames drawn along video clips.
    pub filmstrip: Option<Filmstrip<'a>>,
    /// Measured peaks plus the `[from, to)` fraction of the source this clip
    /// shows, for audio clips.
    pub peaks: Option<(&'a [f32], f32, f32)>,
}

/// Blue camera clip.
pub fn a_roll_clip(ui: &mut Ui, rect: Rect, label: &str, selected: bool) -> Response {
    clip_block(ui, rect, ClipKind::ARoll, label, selected, ClipVisuals::default())
}

/// Purple generative clip, sparkle-badged.
pub fn b_roll_clip(ui: &mut Ui, rect: Rect, label: &str, selected: bool) -> Response {
    clip_block(ui, rect, ClipKind::BRoll, label, selected, ClipVisuals::default())
}

/// Green audio clip with a mock waveform.
pub fn audio_waveform_clip(ui: &mut Ui, rect: Rect, label: &str, selected: bool) -> Response {
    clip_block(ui, rect, ClipKind::Audio, label, selected, ClipVisuals::default())
}

/// Reserve `rect`, sense click/drag, then paint the block directly.
pub fn clip_block(
    ui: &mut Ui,
    rect: Rect,
    kind: ClipKind,
    label: &str,
    selected: bool,
    visuals: ClipVisuals<'_>,
) -> Response {
    let resp = ui.allocate_rect(rect, Sense::click_and_drag());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::Grab);
    }
    paint_clip_visuals(ui.painter(), rect, kind, label, selected, resp.hovered(), visuals);
    resp.on_hover_text(label)
}

/// Filmstrip across the whole clip: every tile shows the frame at *its own*
/// position in time, at the source aspect so images are cropped, not squashed.
fn paint_filmstrip(p: &egui::Painter, rect: Rect, strip: Option<Filmstrip<'_>>) {
    let Some(strip) = strip else {
        return;
    };
    let band = rect.shrink(2.0);
    if band.height() < 12.0 || band.width() < 10.0 {
        return;
    }
    let Some(first) = strip.at(0.0) else {
        return;
    };

    let tile_w = (band.height() * first.aspect).max(8.0);
    // Enough tiles to cover the clip, capped so an extreme zoom can't flood
    // the painter with draw calls.
    let count = ((band.width() / tile_w).ceil() as usize).clamp(1, 64);

    for i in 0..count {
        let left = band.left() + i as f32 * tile_w;
        let tile = Rect::from_min_max(
            Pos2::new(left, band.top()),
            Pos2::new((left + tile_w).min(band.right()), band.bottom()),
        );
        if tile.width() < 4.0 {
            break;
        }

        let fraction = (tile.center().x - band.left()) / band.width().max(1.0);
        let Some(poster) = strip.at(fraction) else {
            continue;
        };
        let uv = cover_uv(tile_w / band.height(), poster.aspect);
        // Partial last tile: crop the UV instead of scaling the image down.
        let visible = tile.width() / tile_w;
        let tile_uv = Rect::from_min_max(
            uv.min,
            Pos2::new(uv.min.x + uv.width() * visible, uv.max.y),
        );
        p.image(poster.id, tile, tile_uv, Color32::from_white_alpha(210));
    }

    // Scrim so the label and badges stay readable over bright footage.
    p.rect_filled(band, R_SM, Color32::from_black_alpha(70));
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
    paint_clip_visuals(p, rect, kind, label, selected, hovered, ClipVisuals::default());
}

/// Same body, with decoded media drawn underneath the badges and label.
pub fn paint_clip_visuals(
    p: &egui::Painter,
    rect: Rect,
    kind: ClipKind,
    label: &str,
    selected: bool,
    hovered: bool,
    visuals: ClipVisuals<'_>,
) {
    let base = kind.color();
    let body = if hovered { base.linear_multiply(1.12) } else { base };
    p.rect_filled(rect, R_SM, body);
    paint_filmstrip(p, rect, visuals.filmstrip);
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
            waveform_peaks(
                p,
                rect.shrink2(Vec2::new(
                    (rect.width() * 0.06).clamp(2.0, 6.0),
                    (rect.height() * 0.2).clamp(4.0, 8.0),
                )),
                base,
                visuals.peaks,
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
