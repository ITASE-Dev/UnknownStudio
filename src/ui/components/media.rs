use crate::ui::components::timeline::sparkle;
use crate::ui::responsive::{elided_galley, grid};
use crate::ui::theme::tokens::*;
use eframe::egui::{
    self, Align2, Color32, FontFamily, FontId, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2,
};

pub const ITEM_MIN_W: f32 = 132.0;
pub const ITEM_MAX_W: f32 = 220.0;

/// Ingested clip card: 16:9 mock plate, play glyph, duration badge, filename.
/// Everything inside scales with `width`; the name elides rather than overflowing.
pub fn media_pool_item(
    ui: &mut Ui,
    name: &str,
    duration: &str,
    selected: bool,
    width: f32,
) -> Response {
    let width = width.max(72.0);
    let plate_h = (width * 9.0 / 16.0).round();
    let name_h = 34.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, plate_h + name_h), Sense::click());
    let p = ui.painter().clone();

    let stroke = if selected {
        Stroke::new(2.0_f32, ACCENT)
    } else if resp.hovered() {
        Stroke::new(1.0_f32, BORDER_STRONG)
    } else {
        hairline()
    };
    p.rect(rect, R, BG_PANEL, stroke);

    let plate = Rect::from_min_size(rect.min + Vec2::splat(1.0), Vec2::new(width - 2.0, plate_h));
    p.rect_filled(plate, top_rounding(5.0), BG_SUNKEN);
    play_glyph(&p, plate);
    duration_badge(&p, plate, duration);

    let galley = elided_galley(
        ui,
        name,
        FontId::new(12.0, FontFamily::Proportional),
        TEXT_PRIMARY,
        width - 20.0,
    );
    p.galley(
        Pos2::new(rect.left() + 10.0, plate.bottom() + 9.0),
        galley,
        TEXT_PRIMARY,
    );
    resp.on_hover_text(name)
}

/// Generated asset card: purple highlighted border, sparkle badge, model caption.
/// The model line is dropped when the card is too narrow to carry two rows.
pub fn generated_asset_item(
    ui: &mut Ui,
    name: &str,
    model: &str,
    duration: &str,
    selected: bool,
    width: f32,
) -> Response {
    let width = width.max(72.0);
    let plate_h = (width * 9.0 / 16.0).round();
    let show_model = width >= 118.0;
    let text_h = if show_model { 44.0 } else { 30.0 };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, plate_h + text_h), Sense::click());
    let p = ui.painter().clone();

    let stroke = if selected {
        Stroke::new(2.0_f32, AI_HOVER)
    } else if resp.hovered() {
        Stroke::new(1.5_f32, AI)
    } else {
        Stroke::new(1.0_f32, AI.linear_multiply(0.65))
    };
    p.rect(rect, R, BG_PANEL, stroke);

    let plate = Rect::from_min_size(rect.min + Vec2::splat(1.0), Vec2::new(width - 2.0, plate_h));
    p.rect_filled(plate, top_rounding(5.0), BG_SUNKEN);
    p.rect_filled(plate, top_rounding(5.0), AI.linear_multiply(0.10));
    play_glyph(&p, plate);
    duration_badge(&p, plate, duration);

    let badge = Rect::from_min_size(
        Pos2::new(plate.left() + 6.0, plate.top() + 6.0),
        Vec2::splat(20.0),
    );
    p.rect_filled(badge, R_SM, AI.linear_multiply(0.85));
    sparkle(&p, badge.center(), 6.0, Color32::WHITE);

    let name_g = elided_galley(
        ui,
        name,
        FontId::new(12.0, FontFamily::Proportional),
        TEXT_PRIMARY,
        width - 20.0,
    );
    p.galley(
        Pos2::new(rect.left() + 10.0, plate.bottom() + 8.0),
        name_g,
        TEXT_PRIMARY,
    );
    if show_model {
        let model_g = elided_galley(
            ui,
            model,
            FontId::new(10.0, FontFamily::Monospace),
            AI_HOVER,
            width - 20.0,
        );
        p.galley(
            Pos2::new(rect.left() + 10.0, plate.bottom() + 24.0),
            model_g,
            AI_HOVER,
        );
    }
    resp.on_hover_text(format!("{name}\n{model}"))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AssetKind {
    Ingested,
    Generated,
}

pub struct PoolAsset<'a> {
    pub name: &'a str,
    pub meta: &'a str,
    pub duration: &'a str,
    pub kind: AssetKind,
    pub selected: bool,
}

/// Media pool laid out as a grid that reflows column count on resize.
/// Returns the index of a clicked card, if any.
pub fn media_pool_grid(ui: &mut Ui, assets: &[PoolAsset<'_>]) -> Option<usize> {
    let mut clicked = None;
    grid(ui, assets.len(), ITEM_MIN_W, ITEM_MAX_W, |ui, i, w| {
        let a = &assets[i];
        let resp = match a.kind {
            AssetKind::Ingested => media_pool_item(ui, a.name, a.duration, a.selected, w),
            AssetKind::Generated => {
                generated_asset_item(ui, a.name, a.meta, a.duration, a.selected, w)
            }
        };
        if resp.clicked() {
            clicked = Some(i);
        }
    });
    clicked
}

fn play_glyph(p: &egui::Painter, plate: Rect) {
    let c = plate.center();
    let r = (plate.height() * 0.24).clamp(9.0, 18.0);
    p.circle_filled(c, r, Color32::from_black_alpha(120));
    p.circle_stroke(c, r, Stroke::new(1.0_f32, TEXT_SECONDARY));
    let s = r * 0.44;
    p.add(egui::Shape::convex_polygon(
        vec![
            Pos2::new(c.x - s * 0.6, c.y - s),
            Pos2::new(c.x - s * 0.6, c.y + s),
            Pos2::new(c.x + s, c.y),
        ],
        TEXT_PRIMARY,
        Stroke::NONE,
    ));
}

fn duration_badge(p: &egui::Painter, plate: Rect, duration: &str) {
    if plate.width() < 84.0 {
        return;
    }
    let badge = Rect::from_min_size(
        Pos2::new(plate.right() - 46.0, plate.bottom() - 20.0),
        Vec2::new(40.0, 14.0),
    );
    p.rect_filled(badge, R_SM, Color32::from_black_alpha(170));
    p.text(
        badge.center(),
        Align2::CENTER_CENTER,
        duration,
        FontId::new(10.0, FontFamily::Monospace),
        TEXT_PRIMARY,
    );
}
