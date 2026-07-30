use eframe::egui::{self, Color32, FontId, Galley, Ui, Vec2};
use std::sync::Arc;

pub const COMPACT_MAX: f32 = 520.0;
pub const MEDIUM_MAX: f32 = 900.0;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Breakpoint {
    Compact,
    Medium,
    Wide,
}

impl Breakpoint {
    pub fn is_compact(self) -> bool {
        self == Self::Compact
    }
}

pub fn breakpoint_for(width: f32) -> Breakpoint {
    if width <= COMPACT_MAX {
        Breakpoint::Compact
    } else if width <= MEDIUM_MAX {
        Breakpoint::Medium
    } else {
        Breakpoint::Wide
    }
}

pub fn breakpoint(ui: &Ui) -> Breakpoint {
    breakpoint_for(ui.available_width())
}

/// Available width clamped into `[min, max]`, never inverted.
pub fn fill_w(ui: &Ui, min: f32, max: f32) -> f32 {
    let lo = min.min(max);
    ui.available_width().min(max).max(lo)
}

/// Width of the key column in inspector rows; shrinks with the panel.
pub fn label_col(ui: &Ui) -> f32 {
    (ui.available_width() * 0.36).clamp(64.0, 160.0)
}

/// Columns and item width that fill the row with items of at least `min_item`.
pub fn grid_metrics(ui: &Ui, min_item: f32, spacing: f32) -> (usize, f32) {
    let avail = ui.available_width().max(min_item);
    let cols = (((avail + spacing) / (min_item + spacing)).floor() as usize).max(1);
    let item = ((avail - spacing * (cols as f32 - 1.0)) / cols as f32).floor();
    (cols, item)
}

/// Wrapped grid that reflows on resize; the closure receives index and item width.
pub fn grid(
    ui: &mut Ui,
    count: usize,
    min_item: f32,
    max_item: f32,
    mut add: impl FnMut(&mut Ui, usize, f32),
) {
    if count == 0 {
        return;
    }
    let spacing = ui.spacing().item_spacing.x;
    let (cols, w) = grid_metrics(ui, min_item, spacing);
    let w = w.min(max_item);
    ui.vertical(|ui| {
        for row in 0..count.div_ceil(cols) {
            ui.horizontal(|ui| {
                for i in (row * cols)..((row + 1) * cols).min(count) {
                    add(ui, i, w);
                }
            });
        }
    });
}

/// Single-line galley truncated with an ellipsis at `max_width`.
pub fn elided_galley(
    ui: &Ui,
    text: &str,
    font: FontId,
    color: Color32,
    max_width: f32,
) -> Arc<Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = egui::text::TextWrapping {
        max_width: max_width.max(8.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    ui.fonts(|f| f.layout_job(job))
}

/// Two columns side by side when there is room, stacked when compact.
/// `cx` is handed to each side in turn so both halves can touch the same state
/// without the closures needing simultaneous mutable captures.
pub fn split<C>(
    ui: &mut Ui,
    left_frac: f32,
    cx: &mut C,
    left: impl FnOnce(&mut Ui, &mut C),
    right: impl FnOnce(&mut Ui, &mut C),
) {
    if breakpoint(ui).is_compact() {
        left(ui, cx);
        ui.add_space(10.0);
        right(ui, cx);
    } else {
        let spacing = ui.spacing().item_spacing.x;
        let total = ui.available_width() - spacing;
        let lw = (total * left_frac.clamp(0.15, 0.85)).floor();
        let td = egui::Layout::top_down(egui::Align::Min);
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(Vec2::new(lw, 0.0), td, |ui| {
                ui.set_width(lw);
                left(ui, cx);
            });
            ui.allocate_ui_with_layout(Vec2::new(total - lw, 0.0), td, |ui| {
                ui.set_width(total - lw);
                right(ui, cx);
            });
        });
    }
}
