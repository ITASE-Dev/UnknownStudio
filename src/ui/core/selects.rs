use crate::ui::responsive::{breakpoint, fill_w, label_col};
use crate::ui::theme::tokens::*;
use eframe::egui::{self, ComboBox, Response, RichText, Ui, Vec2};

/// Rounded ComboBox spanning the row up to a sane maximum.
pub fn pro_dropdown(ui: &mut Ui, id: &str, options: &[&str], selected: &mut usize) -> Response {
    let w = fill_w(ui, 88.0, 260.0);
    pro_dropdown_sized(ui, id, options, selected, w)
}

/// Rounded ComboBox at an explicit width; writes the picked index into `selected`.
pub fn pro_dropdown_sized(
    ui: &mut Ui,
    id: &str,
    options: &[&str],
    selected: &mut usize,
    width: f32,
) -> Response {
    let current = options.get(*selected).copied().unwrap_or("—");
    let inner = ComboBox::from_id_source(id)
        .selected_text(RichText::new(current).color(TEXT_PRIMARY))
        .width(width.max(72.0) - 20.0)
        .show_ui(ui, |ui| {
            ui.set_min_width(width.max(120.0));
            let mut changed = false;
            for (i, opt) in options.iter().enumerate() {
                if ui
                    .selectable_label(*selected == i, RichText::new(*opt).color(TEXT_PRIMARY))
                    .clicked()
                {
                    *selected = i;
                    changed = true;
                }
            }
            changed
        });
    let mut resp = inner.response;
    if inner.inner.unwrap_or(false) {
        resp.mark_changed();
    }
    resp
}

/// `label ─ dropdown` row; stacks when compact.
pub fn pro_dropdown_row(
    ui: &mut Ui,
    id: &str,
    label: &str,
    options: &[&str],
    selected: &mut usize,
) -> Response {
    if breakpoint(ui).is_compact() {
        return ui
            .vertical(|ui| {
                ui.add(
                    egui::Label::new(RichText::new(label).small().color(TEXT_SECONDARY))
                        .truncate(true),
                );
                pro_dropdown(ui, id, options, selected)
            })
            .inner;
    }
    ui.horizontal(|ui| {
        let lw = label_col(ui);
        ui.add_sized(
            Vec2::new(lw, 20.0),
            egui::Label::new(RichText::new(label).color(TEXT_SECONDARY)).truncate(true),
        );
        pro_dropdown(ui, id, options, selected)
    })
    .inner
}
