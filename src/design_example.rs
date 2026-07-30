//! AI Video Director Studio — Component Showcase / Design System
//!
//! Pure `egui` implementation of the studio's reusable UI vocabulary.
//! Theme: "Apple Pro App" (Final Cut / Logic Pro), strict dark mode.
//!
//! egui / eframe 0.27-compatible.

use eframe::egui::{
    self, Align, Align2, Color32, ComboBox, FontFamily, FontId, Frame, Layout, Margin, Pos2, Rect,
    Response, RichText, Rounding, ScrollArea, Sense, Stroke, TextStyle, Ui, Vec2,
};

// ---------------------------------------------------------------------------
// Design tokens
// ---------------------------------------------------------------------------

pub mod tokens {
    use eframe::egui::{Color32, Rounding, Stroke};

    // Surfaces
    pub const BG_APP: Color32 = Color32::from_rgb(0x1e, 0x1e, 0x1e);
    pub const BG_PANEL: Color32 = Color32::from_rgb(0x25, 0x25, 0x26);
    pub const BG_ELEVATED: Color32 = Color32::from_rgb(0x2d, 0x2d, 0x30);
    pub const BG_SUNKEN: Color32 = Color32::from_rgb(0x17, 0x17, 0x18);

    // Lines
    pub const BORDER: Color32 = Color32::from_rgb(0x3e, 0x3e, 0x42);
    pub const BORDER_STRONG: Color32 = Color32::from_rgb(0x50, 0x50, 0x55);

    // Text
    pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(0xf2, 0xf2, 0xf7);
    pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(0x98, 0x98, 0x9f);
    pub const TEXT_DISABLED: Color32 = Color32::from_rgb(0x63, 0x63, 0x69);

    // Accent
    pub const ACCENT: Color32 = Color32::from_rgb(0x0a, 0x84, 0xff);
    pub const ACCENT_HOVER: Color32 = Color32::from_rgb(0x37, 0x9b, 0xff);
    pub const ACCENT_ACTIVE: Color32 = Color32::from_rgb(0x00, 0x6d, 0xd9);

    // Semantic
    pub const OK: Color32 = Color32::from_rgb(0x30, 0xd1, 0x58);
    pub const WARN: Color32 = Color32::from_rgb(0xff, 0xd6, 0x0a);
    pub const ERR: Color32 = Color32::from_rgb(0xff, 0x45, 0x3a);
    pub const IDLE: Color32 = Color32::from_rgb(0x8e, 0x8e, 0x93);

    // Timeline
    pub const CLIP_AROLL: Color32 = Color32::from_rgb(0x2c, 0x7c, 0xd6);
    pub const CLIP_BROLL: Color32 = Color32::from_rgb(0x7d, 0x54, 0xd1);
    pub const CLIP_AUDIO: Color32 = Color32::from_rgb(0x2f, 0x9e, 0x63);

    pub const R: Rounding = Rounding::same(6.0);
    pub const R_SM: Rounding = Rounding::same(4.0);
    pub const R_PILL: Rounding = Rounding::same(999.0);

    pub fn hairline() -> Stroke {
        Stroke::new(1.0, BORDER)
    }
}

use tokens::*;

// ---------------------------------------------------------------------------
// Theme
// ---------------------------------------------------------------------------

pub fn setup_custom_theme(ctx: &egui::Context) {
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
    style.spacing.interact_size.y = 26.0;
    style.spacing.slider_width = 220.0;
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
    v.window_stroke = tokens::hairline();
    v.window_rounding = R;
    v.menu_rounding = R;
    v.selection.bg_fill = ACCENT.linear_multiply(0.45);
    v.selection.stroke = Stroke::new(1.0, TEXT_PRIMARY);
    v.hyperlink_color = ACCENT;
    v.window_shadow = egui::epaint::Shadow {
        offset: Vec2::new(0.0, 8.0),
        blur: 24.0,
        spread: 0.0,
        color: Color32::from_black_alpha(140),
    };
    v.popup_shadow = v.window_shadow;

    // Widget states
    v.widgets.noninteractive.bg_fill = BG_PANEL;
    v.widgets.noninteractive.weak_bg_fill = BG_PANEL;
    v.widgets.noninteractive.bg_stroke = tokens::hairline();
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_SECONDARY);
    v.widgets.noninteractive.rounding = R;

    v.widgets.inactive.bg_fill = BG_ELEVATED;
    v.widgets.inactive.weak_bg_fill = BG_ELEVATED;
    v.widgets.inactive.bg_stroke = tokens::hairline();
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT_PRIMARY);
    v.widgets.inactive.rounding = R;
    v.widgets.inactive.expansion = 0.0;

    v.widgets.hovered.bg_fill = Color32::from_rgb(0x38, 0x38, 0x3c);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x38, 0x38, 0x3c);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, BORDER_STRONG);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT_PRIMARY);
    v.widgets.hovered.rounding = R;
    v.widgets.hovered.expansion = 0.0;

    v.widgets.active.bg_fill = ACCENT_ACTIVE;
    v.widgets.active.weak_bg_fill = ACCENT_ACTIVE;
    v.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    v.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    v.widgets.active.rounding = R;
    v.widgets.active.expansion = 0.0;

    v.widgets.open.bg_fill = BG_ELEVATED;
    v.widgets.open.weak_bg_fill = BG_ELEVATED;
    v.widgets.open.bg_stroke = Stroke::new(1.0, BORDER_STRONG);
    v.widgets.open.fg_stroke = Stroke::new(1.0, TEXT_PRIMARY);
    v.widgets.open.rounding = R;

    ctx.set_style(style);
}

// ---------------------------------------------------------------------------
// Primitives — layout helpers
// ---------------------------------------------------------------------------

pub mod layout {
    use super::*;

    /// Section header with a hairline rule underneath.
    pub fn section(ui: &mut Ui, title: &str, caption: &str) {
        ui.add_space(10.0);
        ui.label(RichText::new(title).heading().color(TEXT_PRIMARY).strong());
        if !caption.is_empty() {
            ui.label(RichText::new(caption).small().color(TEXT_SECONDARY));
        }
        ui.add_space(6.0);
        rule(ui);
        ui.add_space(10.0);
    }

    pub fn rule(ui: &mut Ui) {
        let w = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 1.0), Sense::hover());
        ui.painter().rect_filled(rect, Rounding::ZERO, BORDER);
    }

    /// A bordered panel used to group showcase specimens.
    pub fn panel<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
        Frame::none()
            .fill(BG_PANEL)
            .stroke(tokens::hairline())
            .rounding(R)
            .inner_margin(Margin::same(14.0))
            .show(ui, add)
            .inner
    }

    /// Small uppercase label describing a specimen.
    pub fn spec_label(ui: &mut Ui, text: &str) {
        ui.label(
            RichText::new(text.to_uppercase()).small().color(TEXT_DISABLED),
        );
    }
}

use layout::{panel, rule, section, spec_label};

// ---------------------------------------------------------------------------
// Components — core controls
// ---------------------------------------------------------------------------

pub mod controls {
    use super::*;

    /// Filled accent button — one per view, the destructive-free primary action.
    pub fn primary_button(ui: &mut Ui, label: &str) -> Response {
        let text = RichText::new(label).color(Color32::WHITE).strong();
        let btn = egui::Button::new(text)
            .fill(ACCENT)
            .stroke(Stroke::NONE)
            .rounding(R)
            .min_size(Vec2::new(0.0, 28.0));
        let r = ui.add(btn);
        if r.hovered() {
            ui.painter().rect_filled(r.rect, R, ACCENT_HOVER.linear_multiply(0.25));
        }
        r
    }

    /// Dark, hairline-outlined button for neutral or cancelling actions.
    pub fn secondary_button(ui: &mut Ui, label: &str) -> Response {
        let btn = egui::Button::new(RichText::new(label).color(TEXT_PRIMARY))
            .fill(BG_ELEVATED)
            .stroke(tokens::hairline())
            .rounding(R)
            .min_size(Vec2::new(0.0, 28.0));
        ui.add(btn)
    }

    /// Chromeless button for tertiary / inline actions.
    pub fn ghost_button(ui: &mut Ui, label: &str) -> Response {
        let btn = egui::Button::new(RichText::new(label).color(TEXT_SECONDARY))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::NONE)
            .rounding(R)
            .min_size(Vec2::new(0.0, 28.0));
        ui.add(btn)
    }

    pub fn danger_button(ui: &mut Ui, label: &str) -> Response {
        let btn = egui::Button::new(RichText::new(label).color(ERR))
            .fill(BG_ELEVATED)
            .stroke(Stroke::new(1.0, ERR.linear_multiply(0.55)))
            .rounding(R)
            .min_size(Vec2::new(0.0, 28.0));
        ui.add(btn)
    }

    /// macOS-style pill switch.
    pub fn switch(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
        let size = Vec2::new(40.0, 22.0);
        let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
        if resp.clicked() {
            *on = !*on;
            resp.mark_changed();
        }
        let t = ui.ctx().animate_bool(resp.id, *on);
        let track = if *on {
            ACCENT.linear_multiply(0.35 + 0.65 * t)
        } else {
            BG_ELEVATED
        };
        let p = ui.painter();
        p.rect(rect, R_PILL, track, tokens::hairline());
        let r = rect.height() / 2.0 - 3.0;
        let cx = egui::lerp((rect.left() + r + 3.0)..=(rect.right() - r - 3.0), t);
        p.circle_filled(Pos2::new(cx, rect.center().y), r, Color32::WHITE);

        if !label.is_empty() {
            ui.add_space(4.0);
            let color = if *on { TEXT_PRIMARY } else { TEXT_SECONDARY };
            ui.label(RichText::new(label).color(color));
        }
        resp
    }

    /// Labelled slider row with a value read-out on the right.
    pub fn slider_row(
        ui: &mut Ui,
        label: &str,
        value: &mut f32,
        range: std::ops::RangeInclusive<f32>,
        suffix: &str,
    ) {
        ui.horizontal(|ui| {
            ui.add_sized(
                Vec2::new(150.0, 20.0),
                egui::Label::new(RichText::new(label).color(TEXT_SECONDARY)).truncate(true),
            );
            ui.add(
                egui::Slider::new(value, range)
                    .show_value(false)
                    .trailing_fill(true),
            );
            ui.label(
                RichText::new(format!("{:.0}{}", value, suffix))
                    .monospace()
                    .color(TEXT_PRIMARY),
            );
        });
    }

    pub fn combo(ui: &mut Ui, id: &str, label: &str, options: &[&str], selected: &mut usize) {
        ui.horizontal(|ui| {
            ui.add_sized(
                Vec2::new(150.0, 20.0),
                egui::Label::new(RichText::new(label).color(TEXT_SECONDARY)),
            );
            ComboBox::from_id_source(id)
                .selected_text(RichText::new(options[*selected]).color(TEXT_PRIMARY))
                .show_ui(ui, |ui| {
                    for (i, opt) in options.iter().enumerate() {
                        ui.selectable_value(selected, i, *opt);
                    }
                });
        });
    }

    /// Segmented control (Final Cut style mode switch).
    pub fn segmented(ui: &mut Ui, options: &[&str], selected: &mut usize) {
        Frame::none()
            .fill(BG_SUNKEN)
            .stroke(tokens::hairline())
            .rounding(R)
            .inner_margin(Margin::same(2.0))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                ui.horizontal(|ui| {
                    for (i, opt) in options.iter().enumerate() {
                        let active = *selected == i;
                        let fill = if active { BG_ELEVATED } else { Color32::TRANSPARENT };
                        let color = if active { TEXT_PRIMARY } else { TEXT_SECONDARY };
                        let btn = egui::Button::new(RichText::new(*opt).color(color))
                            .fill(fill)
                            .stroke(Stroke::NONE)
                            .rounding(R_SM)
                            .min_size(Vec2::new(78.0, 24.0));
                        if ui.add(btn).clicked() {
                            *selected = i;
                        }
                    }
                });
            });
    }
}

// ---------------------------------------------------------------------------
// Components — AI & system indicators
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ServiceState {
    Online,
    Working,
    Error,
    Idle,
}

impl ServiceState {
    pub fn color(self) -> Color32 {
        match self {
            ServiceState::Online => OK,
            ServiceState::Working => WARN,
            ServiceState::Error => ERR,
            ServiceState::Idle => IDLE,
        }
    }
}

pub mod indicators {
    use super::*;

    /// Status pill: dot + label on a sunken rounded field.
    pub fn status_pill(ui: &mut Ui, label: &str, state: ServiceState, t: f32) -> Response {
        let text = RichText::new(label).small().color(TEXT_PRIMARY);
        let galley = ui.painter().layout_no_wrap(
            label.to_owned(),
            TextStyle::Small.resolve(ui.style()),
            TEXT_PRIMARY,
        );
        let size = Vec2::new(galley.size().x + 34.0, 22.0);
        let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
        let p = ui.painter();
        p.rect(
            rect,
            R_PILL,
            BG_SUNKEN,
            Stroke::new(1.0, state.color().linear_multiply(0.45)),
        );
        let dot = Pos2::new(rect.left() + 12.0, rect.center().y);
        if state == ServiceState::Working {
            let pulse = 0.45 + 0.55 * (t * 3.0).sin().abs();
            p.circle_filled(dot, 6.0, state.color().linear_multiply(0.20 * pulse));
        }
        p.circle_filled(dot, 3.5, state.color());
        p.galley(
            Pos2::new(rect.left() + 22.0, rect.center().y - galley.size().y / 2.0),
            galley,
            TEXT_PRIMARY,
        );
        let _ = text;
        resp
    }

    /// Thin progress bar for background AI jobs.
    pub fn job_progress(ui: &mut Ui, label: &str, progress: f32) {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(label).small().color(TEXT_SECONDARY));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("{:>3.0}%", progress * 100.0))
                            .monospace()
                            .small()
                            .color(TEXT_SECONDARY),
                    );
                });
            });
            let (rect, _) = ui.allocate_exact_size(
                Vec2::new(ui.available_width().min(260.0), 4.0),
                Sense::hover(),
            );
            let p = ui.painter();
            p.rect_filled(rect, R_PILL, BG_SUNKEN);
            let mut fill = rect;
            fill.set_width(rect.width() * progress.clamp(0.0, 1.0));
            p.rect_filled(fill, R_PILL, ACCENT);
        });
    }

    /// GPU / VRAM style meter.
    pub fn meter(ui: &mut Ui, label: &str, value: f32, unit: &str) {
        ui.vertical(|ui| {
            spec_label(ui, label);
            ui.label(
                RichText::new(format!("{value:.1}{unit}"))
                    .monospace()
                    .size(16.0)
                    .color(TEXT_PRIMARY),
            );
        });
    }
}

// ---------------------------------------------------------------------------
// Components — media pool
// ---------------------------------------------------------------------------

pub mod media {
    use super::*;

    /// Mock video thumbnail card: 16:9 plate, play glyph, filename + duration.
    pub fn video_thumbnail(
        ui: &mut Ui,
        name: &str,
        duration: &str,
        selected: bool,
        width: f32,
    ) -> Response {
        let plate_h = (width * 9.0 / 16.0).round();
        let size = Vec2::new(width, plate_h + 34.0);
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        let p = ui.painter();

        let stroke = if selected {
            Stroke::new(2.0, ACCENT)
        } else if resp.hovered() {
            Stroke::new(1.0, BORDER_STRONG)
        } else {
            tokens::hairline()
        };
        p.rect(rect, R, BG_PANEL, stroke);

        let plate = Rect::from_min_size(rect.min + Vec2::splat(1.0), Vec2::new(width - 2.0, plate_h));
        p.rect_filled(plate, Rounding { nw: 5.0, ne: 5.0, sw: 0.0, se: 0.0 }, BG_SUNKEN);

        // Play glyph
        let c = plate.center();
        p.circle_filled(c, 15.0, Color32::from_black_alpha(120));
        p.circle_stroke(c, 15.0, Stroke::new(1.0, TEXT_SECONDARY));
        p.add(egui::Shape::convex_polygon(
            vec![
                Pos2::new(c.x - 4.0, c.y - 6.5),
                Pos2::new(c.x - 4.0, c.y + 6.5),
                Pos2::new(c.x + 7.0, c.y),
            ],
            TEXT_PRIMARY,
            Stroke::NONE,
        ));

        // Duration badge
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

        // Filename row
        p.text(
            Pos2::new(rect.left() + 10.0, plate.bottom() + 10.0),
            Align2::LEFT_TOP,
            name,
            FontId::new(12.0, FontFamily::Proportional),
            TEXT_PRIMARY,
        );
        resp
    }
}

// ---------------------------------------------------------------------------
// Components — chat assistant
// ---------------------------------------------------------------------------

pub mod chat {
    use super::*;

    /// AI bubble: elevated surface, accent hairline, left aligned.
    pub fn ai_message(ui: &mut Ui, author: &str, body: &str, max_width: f32) {
        ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
            ui.set_max_width(max_width);
            Frame::none()
                .fill(BG_ELEVATED)
                .stroke(Stroke::new(1.0, ACCENT.linear_multiply(0.35)))
                .rounding(Rounding { nw: 2.0, ne: 6.0, sw: 6.0, se: 6.0 })
                .inner_margin(Margin::symmetric(12.0, 10.0))
                .show(ui, |ui| {
                    ui.set_max_width(max_width - 24.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(author).small().strong().color(ACCENT));
                        ui.add_space(2.0);
                        ui.label(RichText::new(body).color(TEXT_PRIMARY));
                    });
                });
        });
    }

    /// User bubble: sunken surface, right aligned.
    pub fn user_message(ui: &mut Ui, body: &str, max_width: f32) {
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            Frame::none()
                .fill(BG_SUNKEN)
                .stroke(tokens::hairline())
                .rounding(Rounding { nw: 6.0, ne: 2.0, sw: 6.0, se: 6.0 })
                .inner_margin(Margin::symmetric(12.0, 10.0))
                .show(ui, |ui| {
                    ui.set_max_width(max_width - 24.0);
                    ui.label(RichText::new(body).color(TEXT_SECONDARY));
                });
        });
    }

    /// "Thinking" indicator with three animated dots.
    pub fn typing(ui: &mut Ui, t: f32) {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(58.0, 26.0), Sense::hover());
        let p = ui.painter();
        p.rect(rect, R, BG_ELEVATED, tokens::hairline());
        for i in 0..3 {
            let phase = t * 4.0 - i as f32 * 0.5;
            let a = 0.35 + 0.65 * (phase.sin() * 0.5 + 0.5);
            p.circle_filled(
                Pos2::new(rect.left() + 16.0 + i as f32 * 13.0, rect.center().y),
                3.0,
                ACCENT.linear_multiply(a),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Components — timeline (painter-driven)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ClipKind {
    ARoll,
    BRoll,
    Audio,
}

#[derive(Clone)]
pub struct TimelineClip {
    pub label: String,
    pub kind: ClipKind,
    /// Start position in seconds.
    pub start: f32,
    /// Duration in seconds.
    pub len: f32,
    pub selected: bool,
}

impl TimelineClip {
    pub fn new(label: &str, kind: ClipKind, start: f32, len: f32) -> Self {
        Self { label: label.into(), kind, start, len, selected: false }
    }
}

pub mod timeline {
    use super::*;

    pub const TRACK_H: f32 = 54.0;
    pub const PX_PER_SEC: f32 = 26.0;

    fn base_color(kind: ClipKind) -> Color32 {
        match kind {
            ClipKind::ARoll => CLIP_AROLL,
            ClipKind::BRoll => CLIP_BROLL,
            ClipKind::Audio => CLIP_AUDIO,
        }
    }

    /// Ruler with second ticks and labels.
    pub fn ruler(ui: &mut Ui, seconds: f32) {
        let (rect, _) =
            ui.allocate_exact_size(Vec2::new(seconds * PX_PER_SEC, 20.0), Sense::hover());
        let p = ui.painter();
        p.rect_filled(rect, Rounding::ZERO, BG_SUNKEN);
        let mut s = 0.0f32;
        while s <= seconds {
            let x = rect.left() + s * PX_PER_SEC;
            let major = (s as i32) % 5 == 0;
            p.line_segment(
                [Pos2::new(x, rect.bottom() - if major { 10.0 } else { 5.0 }), Pos2::new(x, rect.bottom())],
                Stroke::new(1.0, if major { BORDER_STRONG } else { BORDER }),
            );
            if major {
                p.text(
                    Pos2::new(x + 4.0, rect.top() + 2.0),
                    Align2::LEFT_TOP,
                    format!("{:02}:{:02}", (s as i32) / 60, (s as i32) % 60),
                    FontId::new(9.0, FontFamily::Monospace),
                    TEXT_DISABLED,
                );
            }
            s += 1.0;
        }
        p.line_segment(
            [rect.left_bottom(), rect.right_bottom()],
            Stroke::new(1.0, BORDER),
        );
    }

    /// One track lane: header label + painted clips.
    pub fn track(ui: &mut Ui, name: &str, clips: &[TimelineClip], seconds: f32, playhead: f32) {
        let (rect, _) = ui.allocate_exact_size(
            Vec2::new(seconds * PX_PER_SEC, TRACK_H),
            Sense::hover(),
        );
        let p = ui.painter().clone();

        p.rect_filled(rect, Rounding::ZERO, BG_APP);
        // Vertical grid
        let mut s = 0.0f32;
        while s <= seconds {
            let x = rect.left() + s * PX_PER_SEC;
            p.line_segment(
                [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                Stroke::new(1.0, Color32::from_white_alpha(6)),
            );
            s += 5.0;
        }

        // Lane label
        p.text(
            Pos2::new(rect.left() + 6.0, rect.top() + 4.0),
            Align2::LEFT_TOP,
            name,
            FontId::new(9.0, FontFamily::Monospace),
            TEXT_DISABLED,
        );

        for clip in clips {
            let r = Rect::from_min_size(
                Pos2::new(rect.left() + clip.start * PX_PER_SEC, rect.top() + 14.0),
                Vec2::new(clip.len * PX_PER_SEC - 2.0, TRACK_H - 20.0),
            );
            draw_clip(&p, r, clip);
        }

        // Playhead
        let px = rect.left() + playhead * PX_PER_SEC;
        p.line_segment(
            [Pos2::new(px, rect.top()), Pos2::new(px, rect.bottom())],
            Stroke::new(1.0, ERR),
        );
    }

    /// Draw a single clip block — used by the gallery and the real timeline.
    pub fn draw_clip(p: &egui::Painter, rect: Rect, clip: &TimelineClip) {
        let base = base_color(clip.kind);
        p.rect_filled(rect, R_SM, base);
        // Top light band for a flat but readable body.
        let band = Rect::from_min_size(rect.min, Vec2::new(rect.width(), 3.0));
        p.rect_filled(band, Rounding { nw: 4.0, ne: 4.0, sw: 0.0, se: 0.0 }, Color32::from_white_alpha(38));
        p.rect_stroke(
            rect,
            R_SM,
            Stroke::new(
                if clip.selected { 2.0 } else { 1.0 },
                if clip.selected { Color32::WHITE } else { Color32::from_black_alpha(90) },
            ),
        );

        match clip.kind {
            ClipKind::Audio => waveform(p, rect.shrink2(Vec2::new(6.0, 8.0)), base),
            ClipKind::BRoll => {
                sparkle(p, Pos2::new(rect.left() + 13.0, rect.center().y + 1.0), 5.0, Color32::WHITE);
            }
            ClipKind::ARoll => {}
        }

        let text_x = match clip.kind {
            ClipKind::BRoll => rect.left() + 24.0,
            _ => rect.left() + 8.0,
        };
        if rect.width() > 54.0 {
            p.text(
                Pos2::new(text_x, rect.center().y),
                Align2::LEFT_CENTER,
                &clip.label,
                FontId::new(11.0, FontFamily::Proportional),
                Color32::WHITE,
            );
        }
    }

    /// Deterministic simulated waveform drawn as mirrored vertical bars.
    pub fn waveform(p: &egui::Painter, rect: Rect, base: Color32) {
        let mid = rect.center().y;
        let bar = 3.0;
        let gap = 2.0;
        let n = ((rect.width() + gap) / (bar + gap)).floor().max(1.0) as usize;
        let col = Color32::from_white_alpha(200);
        for i in 0..n {
            let f = i as f32;
            // Cheap pseudo-random envelope, stable across frames.
            let a = (f * 0.7).sin() * 0.5 + (f * 0.23).cos() * 0.35 + (f * 1.9).sin() * 0.15;
            let h = (a.abs() * rect.height() * 0.5).max(1.5);
            let x = rect.left() + f * (bar + gap);
            p.rect_filled(
                Rect::from_min_max(Pos2::new(x, mid - h), Pos2::new(x + bar, mid + h)),
                Rounding::same(1.0),
                col,
            );
        }
        p.line_segment(
            [Pos2::new(rect.left(), mid), Pos2::new(rect.right(), mid)],
            Stroke::new(1.0, base.linear_multiply(1.4)),
        );
    }

    /// Four-point sparkle used to mark generative content.
    pub fn sparkle(p: &egui::Painter, c: Pos2, r: f32, color: Color32) {
        let pts = vec![
            Pos2::new(c.x, c.y - r),
            Pos2::new(c.x + r * 0.28, c.y - r * 0.28),
            Pos2::new(c.x + r, c.y),
            Pos2::new(c.x + r * 0.28, c.y + r * 0.28),
            Pos2::new(c.x, c.y + r),
            Pos2::new(c.x - r * 0.28, c.y + r * 0.28),
            Pos2::new(c.x - r, c.y),
            Pos2::new(c.x - r * 0.28, c.y - r * 0.28),
        ];
        p.add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
    }
}

// ---------------------------------------------------------------------------
// Gallery app
// ---------------------------------------------------------------------------

pub struct ComponentGalleryApp {
    identity_lock: bool,
    auto_broll: bool,
    remove_silence: bool,
    platform: usize,
    model: usize,
    view_mode: usize,
    silence_threshold: f32,
    broll_density: f32,
    music_duck: f32,
    time: f32,
    clips_video: Vec<TimelineClip>,
    clips_audio: Vec<TimelineClip>,
}

impl Default for ComponentGalleryApp {
    fn default() -> Self {
        Self {
            identity_lock: true,
            auto_broll: true,
            remove_silence: false,
            platform: 0,
            model: 1,
            view_mode: 0,
            silence_threshold: 420.0,
            broll_density: 35.0,
            music_duck: 18.0,
            time: 0.0,
            clips_video: vec![
                TimelineClip::new("Clip_01.mp4", ClipKind::ARoll, 0.0, 6.0),
                {
                    let mut c = TimelineClip::new("Neon City", ClipKind::BRoll, 6.0, 4.0);
                    c.selected = true;
                    c
                },
                TimelineClip::new("Clip_02.mp4", ClipKind::ARoll, 10.0, 7.0),
                TimelineClip::new("Aerial Pan", ClipKind::BRoll, 17.0, 3.0),
            ],
            clips_audio: vec![
                TimelineClip::new("VO_take3.wav", ClipKind::Audio, 0.0, 11.0),
                TimelineClip::new("bed_music.wav", ClipKind::Audio, 11.0, 9.0),
            ],
        }
    }
}

impl eframe::App for ComponentGalleryApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.time = ctx.input(|i| i.time) as f32;
        ctx.request_repaint_after(std::time::Duration::from_millis(33));

        self.title_bar(ctx);

        egui::CentralPanel::default()
            .frame(Frame::none().fill(BG_APP).inner_margin(Margin::same(0.0)))
            .show(ctx, |ui| {
                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        Frame::none()
                            .inner_margin(Margin::symmetric(28.0, 20.0))
                            .show(ui, |ui| {
                                ui.set_max_width(1000.0);
                                self.typography(ui);
                                self.swatches(ui);
                                self.core_controls(ui);
                                self.system_indicators(ui);
                                self.media_pool(ui);
                                self.assistant(ui);
                                self.timeline_blocks(ui);
                                ui.add_space(40.0);
                            });
                    });
            });
    }
}

impl ComponentGalleryApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_custom_theme(&cc.egui_ctx);
        Self::default()
    }

    // -- chrome -------------------------------------------------------------

    fn title_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("titlebar")
            .exact_height(44.0)
            .frame(
                Frame::none()
                    .fill(BG_PANEL)
                    .inner_margin(Margin::symmetric(16.0, 0.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.label(
                        RichText::new("AI Video Director Studio")
                            .strong()
                            .color(TEXT_PRIMARY),
                    );
                    ui.label(RichText::new("/").color(TEXT_DISABLED));
                    ui.label(RichText::new("Component Gallery").color(TEXT_SECONDARY));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        controls::primary_button(ui, "Render Project");
                        controls::secondary_button(ui, "Preview");
                        indicators::status_pill(ui, "GPU Ready", ServiceState::Online, self.time);
                    });
                });
            });
        egui::TopBottomPanel::top("titlebar_rule")
            .exact_height(1.0)
            .frame(Frame::none().fill(BORDER))
            .show(ctx, |_| {});
    }

    // -- 1. typography & colors --------------------------------------------

    fn typography(&mut self, ui: &mut Ui) {
        section(ui, "Typography", "Single family, three weights of emphasis.");
        panel(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 10.0;
            ui.label(
                RichText::new("Display / 28 — Project Overview")
                    .size(28.0)
                    .strong()
                    .color(TEXT_PRIMARY),
            );
            ui.label(
                RichText::new("Heading / 19 — Timeline Automation")
                    .heading()
                    .color(TEXT_PRIMARY),
            );
            ui.label(RichText::new("Body / 13 — Primary text sits at full contrast for anything the editor reads while working.").color(TEXT_PRIMARY));
            ui.label(RichText::new("Secondary / 13 — Supporting copy, field labels and metadata.").color(TEXT_SECONDARY));
            ui.label(RichText::new("Accent / 13 — Links, active state and AI attribution.").color(ACCENT));
            ui.label(RichText::new("Mono / 12 — 00:04:12:18   ffmpeg -i clip_01.mp4").monospace().color(TEXT_SECONDARY));
            ui.label(RichText::new("Disabled / 13 — unavailable action").color(TEXT_DISABLED));
        });
    }

    fn swatches(&mut self, ui: &mut Ui) {
        section(ui, "Color", "Charcoal surfaces, hairline borders, one accent.");
        panel(ui, |ui| {
            let items: [(&str, &str, Color32); 8] = [
                ("bg/app", "#1E1E1E", BG_APP),
                ("bg/panel", "#252526", BG_PANEL),
                ("bg/elevated", "#2D2D30", BG_ELEVATED),
                ("bg/sunken", "#171718", BG_SUNKEN),
                ("border", "#3E3E42", BORDER),
                ("accent", "#0A84FF", ACCENT),
                ("success", "#30D158", OK),
                ("warning", "#FFD60A", WARN),
            ];
            ui.horizontal_wrapped(|ui| {
                for (name, hex, color) in items {
                    ui.allocate_ui(Vec2::new(104.0, 92.0), |ui| {
                        ui.vertical(|ui| {
                            let (rect, _) =
                                ui.allocate_exact_size(Vec2::new(96.0, 48.0), Sense::hover());
                            ui.painter().rect(rect, R, color, tokens::hairline());
                            ui.add_space(4.0);
                            ui.label(RichText::new(name).small().color(TEXT_PRIMARY));
                            ui.label(RichText::new(hex).monospace().small().color(TEXT_DISABLED));
                        });
                    });
                }
            });
        });
    }

    // -- 2. core controls ---------------------------------------------------

    fn core_controls(&mut self, ui: &mut Ui) {
        section(ui, "Core Controls", "Buttons, switches, pickers, sliders.");
        panel(ui, |ui| {
            spec_label(ui, "Buttons");
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                controls::primary_button(ui, "Render Project");
                controls::secondary_button(ui, "Cancel");
                controls::ghost_button(ui, "Reset defaults");
                controls::danger_button(ui, "Delete Take");
                ui.add_enabled_ui(false, |ui| {
                    controls::secondary_button(ui, "Export (no media)");
                });
            });

            ui.add_space(16.0);
            rule(ui);
            ui.add_space(14.0);

            spec_label(ui, "Switches & checkboxes");
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                controls::switch(ui, &mut self.identity_lock, "Identity Lock");
                ui.add_space(18.0);
                controls::switch(ui, &mut self.auto_broll, "Auto B-Roll");
                ui.add_space(18.0);
                ui.checkbox(&mut self.remove_silence, "Remove silences");
            });

            ui.add_space(16.0);
            rule(ui);
            ui.add_space(14.0);

            spec_label(ui, "Segmented control");
            ui.add_space(6.0);
            controls::segmented(ui, &["Assemble", "Refine", "Grade"], &mut self.view_mode);

            ui.add_space(16.0);
            rule(ui);
            ui.add_space(14.0);

            spec_label(ui, "Pickers");
            ui.add_space(6.0);
            controls::combo(
                ui,
                "platform",
                "Platform",
                &["YouTube 16:9", "Shorts 9:16", "Reels 9:16", "Square 1:1"],
                &mut self.platform,
            );
            controls::combo(
                ui,
                "model",
                "Generative model",
                &["SDXL Turbo", "ComfyUI · Flux", "Runway Gen-3"],
                &mut self.model,
            );

            ui.add_space(16.0);
            rule(ui);
            ui.add_space(14.0);

            spec_label(ui, "Sliders");
            ui.add_space(6.0);
            controls::slider_row(ui, "Silence threshold", &mut self.silence_threshold, 50.0..=1500.0, " ms");
            controls::slider_row(ui, "B-Roll density", &mut self.broll_density, 0.0..=100.0, " %");
            controls::slider_row(ui, "Music ducking", &mut self.music_duck, 0.0..=40.0, " dB");
        });
    }

    // -- 3. AI & system indicators -----------------------------------------

    fn system_indicators(&mut self, ui: &mut Ui) {
        section(ui, "AI & System", "Microservice health and background jobs.");
        panel(ui, |ui| {
            spec_label(ui, "Service pills");
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                indicators::status_pill(ui, "Audio Engine Online", ServiceState::Online, self.time);
                indicators::status_pill(ui, "ComfyUI Rendering…", ServiceState::Working, self.time);
                indicators::status_pill(ui, "Whisper ASR Online", ServiceState::Online, self.time);
                indicators::status_pill(ui, "LLM Director Offline", ServiceState::Error, self.time);
                indicators::status_pill(ui, "Upscaler Idle", ServiceState::Idle, self.time);
            });

            ui.add_space(16.0);
            rule(ui);
            ui.add_space(14.0);

            spec_label(ui, "Background jobs");
            ui.add_space(6.0);
            indicators::job_progress(ui, "Generating B-Roll · shot 4 of 11", 0.36);
            ui.add_space(10.0);
            indicators::job_progress(ui, "Transcribing VO_take3.wav", 0.82);

            ui.add_space(16.0);
            rule(ui);
            ui.add_space(14.0);

            ui.horizontal(|ui| {
                indicators::meter(ui, "VRAM", 14.2, " GB");
                ui.add_space(28.0);
                indicators::meter(ui, "GPU", 71.0, " %");
                ui.add_space(28.0);
                indicators::meter(ui, "Queue", 3.0, " jobs");
            });
        });
    }

    // -- 4. media pool ------------------------------------------------------

    fn media_pool(&mut self, ui: &mut Ui) {
        section(ui, "Media Pool", "Ingested clips, generated shots and stems.");
        panel(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                media::video_thumbnail(ui, "Clip_01.mp4", "00:06", true, 176.0);
                media::video_thumbnail(ui, "Clip_02.mp4", "00:07", false, 176.0);
                media::video_thumbnail(ui, "gen_neon_city.mp4", "00:04", false, 176.0);
                media::video_thumbnail(ui, "VO_take3.wav", "00:11", false, 176.0);
            });
        });
    }

    // -- 5. chat assistant --------------------------------------------------

    fn assistant(&mut self, ui: &mut Ui) {
        section(ui, "Director Assistant", "Conversation surface for AI edits.");
        panel(ui, |ui| {
            let w = ui.available_width().min(560.0);
            chat::user_message(ui, "Cut the first minute down to 40 seconds and keep the hook.", w);
            ui.add_space(8.0);
            chat::ai_message(
                ui,
                "Director",
                "Trimmed 22s of silence and dropped two redundant takes. Added a generative B-Roll over the product mention at 00:18. Identity Lock kept the presenter untouched.",
                w,
            );
            ui.add_space(8.0);
            chat::user_message(ui, "Good. Push the music 6 dB under the VO.", w);
            ui.add_space(8.0);
            chat::typing(ui, self.time);
        });
    }

    // -- 6. timeline blocks -------------------------------------------------

    fn timeline_blocks(&mut self, ui: &mut Ui) {
        section(ui, "Timeline Blocks", "Painter-drawn clips; the primitives the NLE reuses.");
        panel(ui, |ui| {
            spec_label(ui, "Clip specimens");
            ui.add_space(8.0);
            let specs = [
                ("A-Roll", TimelineClip::new("Clip_01.mp4", ClipKind::ARoll, 0.0, 0.0)),
                ("Generative B-Roll", TimelineClip::new("Neon City", ClipKind::BRoll, 0.0, 0.0)),
                ("Audio", TimelineClip::new("VO_take3.wav", ClipKind::Audio, 0.0, 0.0)),
            ];
            ui.horizontal_wrapped(|ui| {
                for (label, clip) in specs {
                    ui.allocate_ui(Vec2::new(210.0, 62.0), |ui| {
                        ui.vertical(|ui| {
                            spec_label(ui, label);
                            ui.add_space(4.0);
                            let (rect, _) =
                                ui.allocate_exact_size(Vec2::new(200.0, 34.0), Sense::hover());
                            timeline::draw_clip(ui.painter(), rect, &clip);
                        });
                    });
                }
            });

            ui.add_space(16.0);
            rule(ui);
            ui.add_space(14.0);

            spec_label(ui, "Assembled sequence");
            ui.add_space(8.0);
            let seconds = 22.0;
            let playhead = 8.0 + (self.time * 0.6).sin() * 2.0;
            ScrollArea::horizontal()
                .id_source("timeline_scroll")
                .show(ui, |ui| {
                    Frame::none()
                        .fill(BG_SUNKEN)
                        .stroke(tokens::hairline())
                        .rounding(R)
                        .inner_margin(Margin::same(1.0))
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing = Vec2::ZERO;
                            ui.vertical(|ui| {
                                timeline::ruler(ui, seconds);
                                timeline::track(ui, "V1 · VIDEO", &self.clips_video, seconds, playhead);
                                timeline::track(ui, "A1 · AUDIO", &self.clips_audio, seconds, playhead);
                            });
                        });
                });
        });
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn run() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1120.0, 860.0])
            .with_min_inner_size([880.0, 600.0])
            .with_title("AI Video Director Studio — Component Gallery"),
        ..Default::default()
    };
    eframe::run_native(
        "ai_video_director_studio",
        options,
        Box::new(|cc| Box::new(ComponentGalleryApp::new(cc))),
    )
}
