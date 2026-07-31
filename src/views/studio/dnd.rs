//! Media pool → timeline drag payload.
//!
//! Panels are separate egui containers, so the drag is carried in studio state
//! rather than through a widget response: the pool starts it, the timeline
//! consumes it, and whatever is left is dropped at the end of the frame.

use crate::ui::components::timeline::clip::ClipKind;
use crate::ui::theme::tokens::*;
use eframe::egui::{
    self, Align2, Color32, FontFamily, FontId, LayerId, Order, Rect, Rounding, Stroke, Vec2,
};
use std::path::PathBuf;

#[derive(Clone)]
pub struct DragAsset {
    pub name: String,
    pub path: Option<PathBuf>,
    pub seconds: f32,
    pub kind: ClipKind,
    /// Whether the source carries an audio stream — a video clip is audible too.
    pub has_audio: bool,
}

impl DragAsset {
    pub fn is_audio(&self) -> bool {
        self.kind == ClipKind::Audio
    }
}

/// Chip that follows the cursor while a pool item is in flight.
pub fn ghost(ctx: &egui::Context, asset: &DragAsset) {
    let Some(pointer) = ctx.pointer_latest_pos() else {
        return;
    };

    let painter = ctx.layer_painter(LayerId::new(Order::Tooltip, egui::Id::new("pool_drag_ghost")));
    let text = format!("{}  {:.1}s", asset.name, asset.seconds);
    let font = FontId::new(11.0, FontFamily::Proportional);
    let galley = painter.layout_no_wrap(text, font, TEXT_PRIMARY);

    let rect = Rect::from_min_size(
        pointer + Vec2::new(12.0, 12.0),
        galley.size() + Vec2::new(18.0, 12.0),
    );
    painter.rect(
        rect,
        Rounding::same(6.0),
        BG_PANEL.gamma_multiply(0.96),
        Stroke::new(1.0_f32, asset.kind.color()),
    );
    painter.rect_filled(
        Rect::from_min_size(rect.min, Vec2::new(3.0, rect.height())),
        Rounding::same(2.0),
        asset.kind.color(),
    );
    painter.galley(rect.min + Vec2::new(12.0, 6.0), galley, TEXT_PRIMARY);
}

/// Insertion preview drawn inside the lane the cursor is over.
pub fn drop_preview(painter: &egui::Painter, rect: Rect, kind: ClipKind, valid: bool) {
    let color = if valid { kind.color() } else { TEXT_DISABLED };
    painter.rect(
        rect,
        R_SM,
        color.gamma_multiply(0.22),
        Stroke::new(1.5_f32, color),
    );
    if valid {
        painter.line_segment(
            [rect.left_top(), rect.left_bottom()],
            Stroke::new(2.0_f32, Color32::WHITE),
        );
    }
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        if valid { "drop" } else { "wrong track" },
        FontId::new(10.0, FontFamily::Proportional),
        if valid { TEXT_PRIMARY } else { TEXT_DISABLED },
    );
}
