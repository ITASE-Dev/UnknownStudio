use eframe::egui::{
    self, Color32, FontFamily, FontId, Margin, Stroke, TextStyle, Vec2,
};

pub mod tokens {
    use eframe::egui::{Color32, Rounding, Stroke};

    pub const BG_APP: Color32 = Color32::from_rgb(0x1e, 0x1e, 0x1e);
    pub const BG_PANEL: Color32 = Color32::from_rgb(0x25, 0x25, 0x26);
    pub const BG_ELEVATED: Color32 = Color32::from_rgb(0x2d, 0x2d, 0x30);
    pub const BG_SUNKEN: Color32 = Color32::from_rgb(0x17, 0x17, 0x18);
    pub const BG_HOVER: Color32 = Color32::from_rgb(0x38, 0x38, 0x3c);

    pub const BORDER: Color32 = Color32::from_rgb(0x3e, 0x3e, 0x42);
    pub const BORDER_STRONG: Color32 = Color32::from_rgb(0x50, 0x50, 0x55);

    pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(0xf2, 0xf2, 0xf7);
    pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(0x98, 0x98, 0x9f);
    pub const TEXT_DISABLED: Color32 = Color32::from_rgb(0x63, 0x63, 0x69);

    pub const ACCENT: Color32 = Color32::from_rgb(0x0a, 0x84, 0xff);
    pub const ACCENT_HOVER: Color32 = Color32::from_rgb(0x37, 0x9b, 0xff);
    pub const ACCENT_ACTIVE: Color32 = Color32::from_rgb(0x00, 0x6d, 0xd9);
    pub const AI: Color32 = Color32::from_rgb(0x7d, 0x54, 0xd1);
    pub const AI_HOVER: Color32 = Color32::from_rgb(0x96, 0x71, 0xe3);

    pub const OK: Color32 = Color32::from_rgb(0x30, 0xd1, 0x58);
    pub const WARN: Color32 = Color32::from_rgb(0xff, 0xd6, 0x0a);
    pub const ERR: Color32 = Color32::from_rgb(0xff, 0x45, 0x3a);
    pub const IDLE: Color32 = Color32::from_rgb(0x8e, 0x8e, 0x93);

    pub const CLIP_AROLL: Color32 = Color32::from_rgb(0x2c, 0x7c, 0xd6);
    pub const CLIP_BROLL: Color32 = Color32::from_rgb(0x7d, 0x54, 0xd1);
    pub const CLIP_AUDIO: Color32 = Color32::from_rgb(0x2f, 0x9e, 0x63);

    pub const R: Rounding = Rounding::same(6.0);
    pub const R_SM: Rounding = Rounding::same(4.0);
    pub const R_PILL: Rounding = Rounding::same(999.0);

    pub const CTRL_H: f32 = 28.0;

    pub fn hairline() -> Stroke {
        Stroke::new(1.0_f32, BORDER)
    }

    pub fn top_rounding(r: f32) -> Rounding {
        Rounding { nw: r, ne: r, sw: 0.0, se: 0.0 }
    }
}

use tokens::*;

/// Appends Phosphor to the proportional family, so icon glyphs can be used
/// inline in any `RichText` and fall back to the UI font for everything else.
fn install_icon_font(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    ctx.set_fonts(fonts);
}

pub fn apply(ctx: &egui::Context) {
    install_icon_font(ctx);
    let mut style = (*ctx.style()).clone();

    style.text_styles = [
        (TextStyle::Heading, FontId::new(19.0, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(13.0, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(13.0, FontFamily::Proportional)),
        (TextStyle::Small, FontId::new(11.0, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(12.0, FontFamily::Monospace)),
    ]
    .into();

    style.spacing.item_spacing = Vec2::new(8.0, 8.0);
    style.spacing.button_padding = Vec2::new(12.0, 6.0);
    style.spacing.window_margin = Margin::same(0.0);
    style.spacing.menu_margin = Margin::same(6.0);
    style.spacing.interact_size.y = 26.0;
    style.spacing.slider_width = 220.0;
    style.spacing.slider_rail_height = 4.0;
    style.spacing.combo_width = 200.0;
    style.spacing.scroll.bar_width = 9.0;
    style.animation_time = 0.10;

    let v = &mut style.visuals;
    v.dark_mode = true;
    v.override_text_color = None;
    v.panel_fill = BG_APP;
    v.window_fill = BG_PANEL;
    v.extreme_bg_color = BG_SUNKEN;
    v.faint_bg_color = BG_ELEVATED;
    v.window_stroke = hairline();
    v.window_rounding = R;
    v.menu_rounding = R;
    v.selection.bg_fill = ACCENT.linear_multiply(0.45);
    v.selection.stroke = Stroke::new(1.0_f32, TEXT_PRIMARY);
    v.hyperlink_color = ACCENT;
    v.slider_trailing_fill = true;
    v.window_shadow = egui::epaint::Shadow {
        offset: Vec2::new(0.0, 8.0),
        blur: 24.0,
        spread: 0.0,
        color: Color32::from_black_alpha(140),
    };
    v.popup_shadow = v.window_shadow;

    v.widgets.noninteractive.bg_fill = BG_PANEL;
    v.widgets.noninteractive.weak_bg_fill = BG_PANEL;
    v.widgets.noninteractive.bg_stroke = hairline();
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT_SECONDARY);
    v.widgets.noninteractive.rounding = R;

    v.widgets.inactive.bg_fill = BG_ELEVATED;
    v.widgets.inactive.weak_bg_fill = BG_ELEVATED;
    v.widgets.inactive.bg_stroke = hairline();
    v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT_PRIMARY);
    v.widgets.inactive.rounding = R;
    v.widgets.inactive.expansion = 0.0;

    v.widgets.hovered.bg_fill = BG_HOVER;
    v.widgets.hovered.weak_bg_fill = BG_HOVER;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, BORDER_STRONG);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT_PRIMARY);
    v.widgets.hovered.rounding = R;
    v.widgets.hovered.expansion = 0.0;

    v.widgets.active.bg_fill = ACCENT_ACTIVE;
    v.widgets.active.weak_bg_fill = ACCENT_ACTIVE;
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    v.widgets.active.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);
    v.widgets.active.rounding = R;
    v.widgets.active.expansion = 0.0;

    v.widgets.open.bg_fill = BG_ELEVATED;
    v.widgets.open.weak_bg_fill = BG_ELEVATED;
    v.widgets.open.bg_stroke = Stroke::new(1.0_f32, BORDER_STRONG);
    v.widgets.open.fg_stroke = Stroke::new(1.0_f32, TEXT_PRIMARY);
    v.widgets.open.rounding = R;

    ctx.set_style(style);
}
