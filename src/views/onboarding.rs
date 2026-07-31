use crate::app::library::ProjectLibrary;
use crate::app::router::AppRoute;
use crate::workspace::TargetPlatform;
use crate::ui::core::buttons::{ghost_button, pro_button};
use crate::ui::core::inputs::pro_text_input;
use crate::ui::core::toggles::pro_toggle_row;
use crate::ui::core::typography::{panel, property_row_with, section_header};
use crate::ui::responsive::{breakpoint, elided_galley, grid};
use crate::ui::theme::tokens::*;
use eframe::egui::{
    self, Align, Align2, Color32, FontFamily, FontId, Layout, Pos2, Rect, Response, RichText,
    Sense, Stroke, Ui, Vec2,
};

pub const PLATFORMS: [(&str, &str); 3] = [
    ("YouTube 16:9", "Long-form, chapters, end screen"),
    ("Shorts 9:16", "Hook in 1.5s, captions burned in"),
    ("Reels 9:16", "Trend audio, tight 20–30s cut"),
];

pub const BLUEPRINTS: [(&str, &str); 4] = [
    ("Fireship", "Fast cuts, zero dead air, code overlays"),
    ("Hook-first", "Cold open payoff, then context"),
    ("Vlog", "Breathing room, ambient beds"),
    ("Podcast", "Speaker switching, highlight pulls"),
];

pub struct OnboardingState {
    pub step: usize,
    pub name: String,
    pub platform: usize,
    pub blueprint: usize,
    pub identity_lock: bool,
    pub auto_broll: bool,
    pub remove_silence: bool,
}

impl Default for OnboardingState {
    fn default() -> Self {
        Self {
            step: 0,
            name: "Untitled Project".into(),
            platform: 0,
            blueprint: 0,
            identity_lock: true,
            auto_broll: true,
            remove_silence: true,
        }
    }
}

const STEPS: [&str; 3] = ["Format", "Blueprint", "Automation"];

fn platform_target(index: usize) -> TargetPlatform {
    match index {
        1 => TargetPlatform::YouTubeShorts,
        2 => TargetPlatform::InstagramReels,
        _ => TargetPlatform::YouTubeLandscape,
    }
}

pub fn show(
    ctx: &egui::Context,
    route: &mut AppRoute,
    state: &mut OnboardingState,
    library: &mut ProjectLibrary,
) {
    super::page(ctx, 780.0, |ui| {
        content(ui, route, state, library);
    });
}

pub fn content(
    ui: &mut Ui,
    route: &mut AppRoute,
    state: &mut OnboardingState,
    library: &mut ProjectLibrary,
) {
    ui.label(RichText::new("New Project").heading().strong().color(TEXT_PRIMARY));
    ui.label(
        RichText::new("Three decisions and the director can start assembling.")
            .small()
            .color(TEXT_SECONDARY),
    );
    ui.add_space(12.0);
    step_indicator(ui, state.step);
    ui.add_space(14.0);

    panel(ui, |ui| match state.step {
        0 => step_format(ui, state),
        1 => step_blueprint(ui, state),
        _ => step_automation(ui, state),
    });

    ui.add_space(14.0);
    footer(ui, route, state, library);
}

fn footer(
    ui: &mut Ui,
    route: &mut AppRoute,
    state: &mut OnboardingState,
    library: &mut ProjectLibrary,
) {
    let last = state.step == STEPS.len() - 1;
    ui.horizontal(|ui| {
        if ghost_button(ui, "Cancel").clicked() {
            *route = AppRoute::Dashboard;
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if last {
                if pro_button(ui, "Start Studio", true).clicked() {
                    let name = match state.name.trim() {
                        "" => "Untitled Project",
                        name => name,
                    };
                    // Writes the project folder first; the studio opens once
                    // the creation lands, so nothing is listed that isn't saved.
                    library.create_and_open(
                        name,
                        platform_target(state.platform),
                        BLUEPRINTS[state.blueprint].0,
                    );
                    *state = OnboardingState::default();
                    *route = AppRoute::Dashboard;
                }
            } else if pro_button(ui, "Next", true).clicked() {
                state.step += 1;
            }
            if state.step > 0 && pro_button(ui, "Back", false).clicked() {
                state.step -= 1;
            }
        });
    });
}

fn step_format(ui: &mut Ui, state: &mut OnboardingState) {
    section_header(ui, "Project");
    property_row_with(ui, "Name", |ui| {
        pro_text_input(ui, &mut state.name, "Episode title")
    });
    ui.add_space(12.0);
    section_header(ui, "Target platform");
    let mut pick = state.platform;
    grid(ui, PLATFORMS.len(), 190.0, 260.0, |ui, i, w| {
        let (title, sub) = PLATFORMS[i];
        if choice_card(ui, title, sub, i == state.platform, w, Some(i)).clicked() {
            pick = i;
        }
    });
    state.platform = pick;
}

fn step_blueprint(ui: &mut Ui, state: &mut OnboardingState) {
    section_header(ui, "AI blueprint");
    ui.label(
        RichText::new("Sets pacing, cut density and how aggressively silences get removed.")
            .small()
            .color(TEXT_SECONDARY),
    );
    ui.add_space(8.0);
    let mut pick = state.blueprint;
    grid(ui, BLUEPRINTS.len(), 190.0, 260.0, |ui, i, w| {
        let (title, sub) = BLUEPRINTS[i];
        if choice_card(ui, title, sub, i == state.blueprint, w, None).clicked() {
            pick = i;
        }
    });
    state.blueprint = pick;
}

fn step_automation(ui: &mut Ui, state: &mut OnboardingState) {
    section_header(ui, "Automation");
    pro_toggle_row(ui, &mut state.identity_lock, "Identity Lock — never restyle the presenter");
    pro_toggle_row(ui, &mut state.auto_broll, "Auto B-Roll — generate shots for named concepts");
    pro_toggle_row(ui, &mut state.remove_silence, "Remove silences before the first pass");
    ui.add_space(12.0);
    section_header(ui, "Summary");
    ui.label(
        RichText::new(format!(
            "{} · {} · {}",
            state.name.trim(),
            PLATFORMS[state.platform].0,
            BLUEPRINTS[state.blueprint].0
        ))
        .monospace()
        .color(TEXT_SECONDARY),
    );
}

fn step_indicator(ui: &mut Ui, step: usize) {
    let compact = breakpoint(ui).is_compact();
    let h = 22.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::hover());
    let p = ui.painter();
    let n = STEPS.len();
    let seg = rect.width() / n as f32;
    for i in 0..n {
        let cx = rect.left() + seg * (i as f32 + 0.5);
        let done = i <= step;
        let c = if done { ACCENT } else { BORDER_STRONG };
        if i + 1 < n {
            p.line_segment(
                [
                    Pos2::new(cx + 9.0, rect.center().y),
                    Pos2::new(cx + seg - 9.0, rect.center().y),
                ],
                Stroke::new(1.0_f32, if i < step { ACCENT } else { BORDER }),
            );
        }
        p.circle_filled(Pos2::new(cx, rect.center().y), 7.0, if done { c } else { BG_ELEVATED });
        p.circle_stroke(Pos2::new(cx, rect.center().y), 7.0, Stroke::new(1.0_f32, c));
        p.text(
            Pos2::new(cx, rect.center().y),
            Align2::CENTER_CENTER,
            format!("{}", i + 1),
            FontId::new(9.0, FontFamily::Monospace),
            if done { Color32::WHITE } else { TEXT_SECONDARY },
        );
        if !compact {
            p.text(
                Pos2::new(cx + 12.0, rect.center().y),
                Align2::LEFT_CENTER,
                STEPS[i],
                FontId::new(11.0, FontFamily::Proportional),
                if done { TEXT_PRIMARY } else { TEXT_DISABLED },
            );
        }
    }
}

/// Selectable option tile; `accent_index` tints the plate for platform choices.
fn choice_card(
    ui: &mut Ui,
    title: &str,
    subtitle: &str,
    selected: bool,
    width: f32,
    accent_index: Option<usize>,
) -> Response {
    let width = width.max(140.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 74.0), Sense::click());
    let p = ui.painter().clone();

    let stroke = if selected {
        Stroke::new(2.0_f32, ACCENT)
    } else if resp.hovered() {
        Stroke::new(1.0_f32, BORDER_STRONG)
    } else {
        hairline()
    };
    let fill = if selected { ACCENT.linear_multiply(0.10) } else { BG_ELEVATED };
    p.rect(rect, R, fill, stroke);

    // Aspect swatch on the left: portrait for 9:16, landscape for 16:9.
    if let Some(i) = accent_index {
        let portrait = i > 0;
        let (w, h) = if portrait { (16.0, 28.0) } else { (30.0, 17.0) };
        let plate = Rect::from_center_size(
            Pos2::new(rect.left() + 26.0, rect.center().y),
            Vec2::new(w, h),
        );
        p.rect(plate, R_SM, BG_SUNKEN, Stroke::new(1.0_f32, BORDER_STRONG));
    }
    let text_x = rect.left() + if accent_index.is_some() { 48.0 } else { 12.0 };
    let text_w = rect.right() - 12.0 - text_x;

    let t = elided_galley(
        ui,
        title,
        FontId::new(13.0, FontFamily::Proportional),
        TEXT_PRIMARY,
        text_w,
    );
    p.galley(Pos2::new(text_x, rect.top() + 18.0), t, TEXT_PRIMARY);
    let s = elided_galley(
        ui,
        subtitle,
        FontId::new(10.0, FontFamily::Proportional),
        TEXT_SECONDARY,
        text_w,
    );
    p.galley(Pos2::new(text_x, rect.top() + 38.0), s, TEXT_SECONDARY);

    if selected {
        let c = Pos2::new(rect.right() - 14.0, rect.top() + 14.0);
        p.circle_filled(c, 6.0, ACCENT);
        p.line_segment(
            [Pos2::new(c.x - 2.6, c.y), Pos2::new(c.x - 0.6, c.y + 2.2)],
            Stroke::new(1.6_f32, Color32::WHITE),
        );
        p.line_segment(
            [Pos2::new(c.x - 0.6, c.y + 2.2), Pos2::new(c.x + 2.8, c.y - 2.2)],
            Stroke::new(1.6_f32, Color32::WHITE),
        );
    }
    resp.on_hover_text(subtitle)
}
