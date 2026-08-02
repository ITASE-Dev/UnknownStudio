use crate::app::modals::{ModalAction, Modals};
use crate::app::router::AppRoute;
use crate::app::Project;
use crate::ui::components::inspector::inspector_group;
use crate::ui::core::buttons::{ai_button, ghost_button, pro_button};
use crate::ui::core::inputs::{pro_text_area, pro_text_input};
use crate::ui::core::selects::pro_dropdown_row;
use crate::ui::core::typography::{hairline_rule, panel, property_row, section_header};
use crate::ui::responsive::{elided_galley, grid, split};
use crate::ui::theme::tokens::*;
use eframe::egui::{
    self, Align, Align2, Color32, FontFamily, FontId, Layout, Pos2, Rect, Response, RichText,
    Sense, Stroke, Ui, Vec2,
};

pub struct GrowthState {
    pub title: String,
    pub description: String,
    pub thumb: usize,
    pub resolution: usize,
    pub codec: usize,
    pub container: usize,
    pub burn_captions: bool,
    /// Set when the user asks for a render; taken by `show`.
    pub export_requested: Option<String>,
    /// Progress or failure from the encoder job.
    pub export_status: Option<String>,
}

impl Default for GrowthState {
    fn default() -> Self {
        Self {
            title: "I rewrote my build in Rust — here's what broke".into(),
            description: "Chapters, links and the benchmark repo in the description.".into(),
            thumb: 1,
            resolution: 0,
            codec: 0,
            container: 0,
            burn_captions: true,
            export_requested: None,
            export_status: None,
        }
    }
}

const THUMBS: [(&str, &str); 3] = [
    ("Face · shock", "high CTR on returning viewers"),
    ("Code · red arrow", "best for search traffic"),
    ("Split · before/after", "strongest on Shorts feed"),
];

/// Mock hourly engagement curve, 0..1 per hour of day.
const HEAT: [f32; 24] = [
    0.12, 0.08, 0.05, 0.04, 0.05, 0.09, 0.18, 0.31, 0.44, 0.52, 0.58, 0.61, 0.66, 0.71, 0.78,
    0.86, 0.94, 1.0, 0.92, 0.81, 0.66, 0.48, 0.31, 0.19,
];

/// Returns an export preset when the user asked for one, for the app to
/// turn into an `AppCommand`. The view has no path to the worker.
pub fn show(
    ctx: &egui::Context,
    route: &mut AppRoute,
    state: &mut GrowthState,
    modals: &mut Modals,
    project: Option<&Project>,
) -> Option<String> {
    super::page(ctx, 1100.0, |ui| content(ui, route, state, modals, project));
    state.export_requested.take()
}

/// The preset name the encoder is asked for, from the form's own controls.
fn export_preset(state: &GrowthState) -> String {
    const CODECS: [&str; 3] = ["h264", "hevc", "prores"];
    const CONTAINERS: [&str; 3] = ["mp4", "mov", "mkv"];

    format!(
        "{}_{}{}",
        CODECS.get(state.codec).copied().unwrap_or("h264"),
        CONTAINERS.get(state.container).copied().unwrap_or("mp4"),
        if state.burn_captions { "_burned" } else { "" }
    )
}

pub fn content(
    ui: &mut Ui,
    route: &mut AppRoute,
    state: &mut GrowthState,
    modals: &mut Modals,
    project: Option<&Project>,
) {
    let name = project.map(|p| p.name.as_str()).unwrap_or("Untitled Project");
    ui.horizontal(|ui| {
        ui.label(RichText::new("Growth & Export").heading().strong().color(TEXT_PRIMARY));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if let Some(p) = project {
                if pro_button(ui, "Back to Studio", false).clicked() {
                    *route = AppRoute::Studio(p.id);
                }
            }
            if ghost_button(ui, "Projects").clicked() {
                *route = AppRoute::Dashboard;
            }
        });
    });
    ui.label(RichText::new(name).small().color(TEXT_SECONDARY));
    ui.add_space(8.0);
    hairline_rule(ui);
    ui.add_space(12.0);

    let cx = &mut (state, modals, project);
    split(
        ui,
        0.58,
        cx,
        |ui, cx| {
            thumbnails(ui, cx.0, cx.1);
            ui.add_space(12.0);
            metadata(ui, cx.0);
        },
        |ui, cx| {
            publish_window(ui);
            ui.add_space(12.0);
            export(ui, cx.0, cx.1, cx.2);
        },
    );
}

fn thumbnails(ui: &mut Ui, state: &mut GrowthState, modals: &mut Modals) {
    panel(ui, |ui| {
        ui.horizontal(|ui| {
            section_header(ui, "Thumbnail candidates");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ai_button(ui, "Generate 3 more").clicked() {
                    modals.progress(
                        "Generating thumbnails",
                        "Sampling 3 candidates with the identity-locked face model.",
                        ModalAction::None,
                    );
                }
            });
        });
        let mut pick = state.thumb;
        grid(ui, THUMBS.len(), 150.0, 240.0, |ui, i, w| {
            let (title, note) = THUMBS[i];
            if thumb_card(ui, title, note, i == state.thumb, w).clicked() {
                pick = i;
            }
        });
        state.thumb = pick;
        ui.add_space(6.0);
        ui.label(
            RichText::new(format!("Selected · {}", THUMBS[state.thumb].1))
                .small()
                .color(TEXT_DISABLED),
        );
    });
}

fn metadata(ui: &mut Ui, state: &mut GrowthState) {
    panel(ui, |ui| {
        section_header(ui, "Title & description");
        pro_text_input(ui, &mut state.title, "Video title");
        ui.add_space(6.0);
        let len = state.title.chars().count();
        let (color, hint) = if len > 70 {
            (ERR, "too long for mobile search results")
        } else if len < 30 {
            (WARN, "short — add the payoff keyword")
        } else {
            (OK, "good length for search and feed")
        };
        ui.label(RichText::new(format!("{len}/70 · {hint}")).small().color(color));
        ui.add_space(10.0);
        pro_text_area(ui, &mut state.description, "Description, chapters, links…", 4);
    });
}

fn publish_window(ui: &mut Ui) {
    inspector_group(ui, "Best publish window", |ui| {
        let best = HEAT
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(i, _)| i)
            .unwrap_or(17);
        heatmap(ui, best);
        ui.add_space(8.0);
        property_row(ui, "Peak hour", &format!("{best:02}:00 local"));
        property_row(ui, "Suggested slot", &format!("{:02}:30 – {:02}:30", best - 1, best));
        property_row(ui, "Audience", "62% returning · 38% new");
    });
}

/// Painted 24-hour engagement bar chart with the peak highlighted.
fn heatmap(ui: &mut Ui, best: usize) {
    let h = 74.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, R_SM, BG_SUNKEN);

    let n = HEAT.len() as f32;
    let gap = 2.0;
    let bw = ((rect.width() - 12.0 - gap * (n - 1.0)) / n).max(2.0);
    let base = rect.bottom() - 16.0;
    let top = rect.top() + 8.0;
    for (i, v) in HEAT.iter().enumerate() {
        let x = rect.left() + 6.0 + i as f32 * (bw + gap);
        let bar = Rect::from_min_max(
            Pos2::new(x, base - (base - top) * *v),
            Pos2::new(x + bw, base),
        );
        let c = if i == best {
            ACCENT
        } else {
            ACCENT.linear_multiply(0.22 + 0.3 * *v)
        };
        p.rect_filled(bar, top_rounding(2.0), c);
        if i % 6 == 0 {
            p.text(
                Pos2::new(x, base + 3.0),
                Align2::LEFT_TOP,
                format!("{i:02}"),
                FontId::new(9.0, FontFamily::Monospace),
                TEXT_DISABLED,
            );
        }
    }
    p.line_segment(
        [Pos2::new(rect.left() + 6.0, base), Pos2::new(rect.right() - 6.0, base)],
        Stroke::new(1.0_f32, BORDER),
    );
}

fn export(ui: &mut Ui, state: &mut GrowthState, modals: &mut Modals, project: Option<&Project>) {
    inspector_group(ui, "Export", |ui| {
        pro_dropdown_row(
            ui,
            "res",
            "Resolution",
            &["1080p", "1440p", "2160p"],
            &mut state.resolution,
        );
        pro_dropdown_row(ui, "codec", "Codec", &["H.264", "HEVC", "ProRes"], &mut state.codec);
        pro_dropdown_row(ui, "container", "Container", &["MP4", "MOV", "MKV"], &mut state.container);
        ui.add_space(6.0);
        crate::ui::core::toggles::pro_toggle_row(ui, &mut state.burn_captions, "Burn captions");
        ui.add_space(10.0);
        property_row(ui, "Estimated size", "412 MB");
        property_row(ui, "Estimated time", "3m 20s");
        ui.add_space(10.0);
        // Queues a real job. It used to open a progress modal that counted
        // to a hundred and navigated — an animation, not an export.
        if pro_button(ui, "Render & Export", true).clicked() {
            state.export_requested = Some(export_preset(state));
        }
        if let Some(status) = &state.export_status {
            ui.add_space(6.0);
            ui.label(RichText::new(status).small().color(TEXT_SECONDARY));
        }
    });
}

/// Painted thumbnail candidate: 16:9 plate with a mock composition + AI border.
fn thumb_card(ui: &mut Ui, title: &str, note: &str, selected: bool, width: f32) -> Response {
    let width = width.max(120.0);
    let plate_h = (width * 9.0 / 16.0).round();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, plate_h + 34.0), Sense::click());
    let p = ui.painter().clone();

    let stroke = if selected {
        Stroke::new(2.0_f32, AI_HOVER)
    } else if resp.hovered() {
        Stroke::new(1.0_f32, AI)
    } else {
        Stroke::new(1.0_f32, AI.linear_multiply(0.5))
    };
    p.rect(rect, R, BG_PANEL, stroke);

    let plate = Rect::from_min_size(rect.min + Vec2::splat(1.0), Vec2::new(width - 2.0, plate_h));
    p.rect_filled(plate, top_rounding(5.0), BG_SUNKEN);
    // Mock composition: subject blob left, headline bars right.
    p.circle_filled(
        Pos2::new(plate.left() + plate.width() * 0.28, plate.center().y + 4.0),
        plate.height() * 0.26,
        AI.linear_multiply(0.55),
    );
    for i in 0..2 {
        let bar = Rect::from_min_size(
            Pos2::new(plate.left() + plate.width() * 0.52, plate.top() + plate.height() * (0.32 + 0.2 * i as f32)),
            Vec2::new(plate.width() * (0.36 - 0.1 * i as f32), 6.0),
        );
        p.rect_filled(bar, R_SM, Color32::from_white_alpha(if i == 0 { 150 } else { 80 }));
    }
    if selected {
        crate::ui::components::timeline::sparkle(
            &p,
            Pos2::new(plate.left() + 12.0, plate.top() + 12.0),
            6.0,
            Color32::WHITE,
        );
    }

    let t = elided_galley(
        ui,
        title,
        FontId::new(11.0, FontFamily::Proportional),
        TEXT_PRIMARY,
        width - 16.0,
    );
    p.galley(Pos2::new(rect.left() + 8.0, plate.bottom() + 9.0), t, TEXT_PRIMARY);
    resp.on_hover_text(note)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_export_preset_reflects_the_form_rather_than_a_hardcoded_string() {
        let mut state = GrowthState::default();
        state.codec = 1;
        state.container = 2;
        state.burn_captions = false;
        assert_eq!(export_preset(&state), "hevc_mkv");

        state.burn_captions = true;
        assert_eq!(export_preset(&state), "hevc_mkv_burned");
    }

    #[test]
    fn an_out_of_range_selection_falls_back_rather_than_panicking() {
        let mut state = GrowthState::default();
        state.codec = 99;
        state.container = 99;
        state.burn_captions = false;
        assert_eq!(export_preset(&state), "h264_mp4");
    }

    #[test]
    fn a_render_request_is_taken_once_and_only_once() {
        let mut state = GrowthState::default();
        state.export_requested = Some("h264_mp4".into());

        assert_eq!(state.export_requested.take(), Some("h264_mp4".into()));
        assert_eq!(state.export_requested.take(), None, "no duplicate job");
    }
}
