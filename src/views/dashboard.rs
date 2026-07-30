use crate::app::modals::Modals;
use crate::app::router::AppRoute;
use crate::app::Project;
use crate::ui::core::buttons::{ghost_button, pro_button};
use crate::ui::core::inputs::search_input;
use crate::ui::core::typography::{hairline_rule, panel, section_title};
use crate::ui::responsive::{breakpoint, elided_galley, grid};
use crate::ui::theme::tokens::*;
use eframe::egui::{
    self, Align, Align2, Color32, FontFamily, FontId, Layout, Pos2, Rect, Response, RichText,
    Sense, Stroke, Ui, Vec2,
};

#[derive(Default)]
pub struct DashboardState {
    pub search: String,
    pub selected: Option<u32>,
}

pub fn show(
    ctx: &egui::Context,
    route: &mut AppRoute,
    state: &mut DashboardState,
    projects: &[Project],
    modals: &mut Modals,
) {
    super::page(ctx, 1100.0, |ui| content(ui, route, state, projects, modals));
}

pub fn content(
    ui: &mut Ui,
    route: &mut AppRoute,
    state: &mut DashboardState,
    projects: &[Project],
    modals: &mut Modals,
) {
    header(ui, route, state);
    ui.add_space(14.0);

    let needle = state.search.trim().to_lowercase();
    let visible: Vec<&Project> = projects
        .iter()
        .filter(|p| needle.is_empty() || p.name.to_lowercase().contains(&needle))
        .collect();

    if visible.is_empty() {
        empty_state(ui, route, &state.search);
        return;
    }

    let mut open: Option<u32> = None;
    let mut clicked: Option<u32> = None;
    grid(ui, visible.len(), 230.0, 320.0, |ui, i, w| {
        let p = visible[i];
        let resp = project_card(ui, p, w, state.selected == Some(p.id));
        if resp.clicked() {
            clicked = Some(p.id);
        }
        if resp.double_clicked() {
            open = Some(p.id);
        }
    });
    if let Some(id) = clicked {
        state.selected = Some(id);
    }
    if let Some(id) = open {
        *route = AppRoute::Studio(id);
        return;
    }

    ui.add_space(16.0);
    if let Some(id) = state.selected {
        if let Some(p) = visible.iter().find(|p| p.id == id) {
            selected_actions(ui, route, modals, p);
        }
    } else {
        ui.label(
            RichText::new("Select a project — double-click a card to jump straight into the cut.")
                .small()
                .color(TEXT_DISABLED),
        );
    }
}

fn header(ui: &mut Ui, route: &mut AppRoute, state: &mut DashboardState) {
    if breakpoint(ui).is_compact() {
        section_title(ui, "Projects", "");
        ui.horizontal(|ui| {
            search_input(ui, &mut state.search, "Search projects…");
        });
        ui.add_space(6.0);
        if pro_button(ui, "New Project", true).clicked() {
            *route = AppRoute::Onboarding;
        }
        return;
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new("Projects").heading().strong().color(TEXT_PRIMARY));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if pro_button(ui, "New Project", true).clicked() {
                *route = AppRoute::Onboarding;
            }
            search_input(ui, &mut state.search, "Search projects…");
        });
    });
    ui.add_space(8.0);
    hairline_rule(ui);
}

fn selected_actions(ui: &mut Ui, route: &mut AppRoute, modals: &mut Modals, p: &Project) {
    panel(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::Label::new(RichText::new(&p.name).strong().color(TEXT_PRIMARY))
                    .truncate(true),
            );
            ui.label(RichText::new(format!("· {} · {}", p.platform, p.duration))
                .small()
                .color(TEXT_SECONDARY));
        });
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            if pro_button(ui, "Open Studio", true).clicked() {
                *route = AppRoute::Studio(p.id);
            }
            if pro_button(ui, "Growth & Export", false).clicked() {
                *route = AppRoute::Growth(p.id);
            }
            if ghost_button(ui, "Duplicate").clicked() {
                modals.info("Duplicate", "Project duplication is not wired up yet.");
            }
        });
    });
}

fn empty_state(ui: &mut Ui, route: &mut AppRoute, search: &str) {
    panel(ui, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(26.0);
            if search.trim().is_empty() {
                ui.label(RichText::new("No projects yet").heading().color(TEXT_PRIMARY));
                ui.label(
                    RichText::new("Start with a blueprint and the director will assemble the cut.")
                        .color(TEXT_SECONDARY),
                );
                ui.add_space(14.0);
                if pro_button(ui, "New Project", true).clicked() {
                    *route = AppRoute::Onboarding;
                }
            } else {
                ui.label(RichText::new("No matches").heading().color(TEXT_PRIMARY));
                ui.label(
                    RichText::new(format!("Nothing in the library matches “{search}”."))
                        .color(TEXT_SECONDARY),
                );
            }
            ui.add_space(26.0);
        });
    });
}

/// Painted project card: 16:9 plate, platform badge, name and metadata row.
fn project_card(ui: &mut Ui, p: &Project, width: f32, selected: bool) -> Response {
    let width = width.max(150.0);
    let plate_h = (width * 9.0 / 16.0).round();
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(width, plate_h + 56.0), Sense::click());
    let pt = ui.painter().clone();

    let stroke = if selected {
        Stroke::new(2.0_f32, ACCENT)
    } else if resp.hovered() {
        Stroke::new(1.0_f32, BORDER_STRONG)
    } else {
        hairline()
    };
    pt.rect(rect, R, BG_PANEL, stroke);

    let plate = Rect::from_min_size(rect.min + Vec2::splat(1.0), Vec2::new(width - 2.0, plate_h));
    pt.rect_filled(plate, top_rounding(5.0), BG_SUNKEN);

    // Mock filmstrip: a few frame cells to suggest a timeline.
    let cells = 4;
    let gap = 3.0;
    let cw = (plate.width() - gap * (cells as f32 + 1.0)) / cells as f32;
    for i in 0..cells {
        let cell = Rect::from_min_size(
            Pos2::new(plate.left() + gap + i as f32 * (cw + gap), plate.top() + gap),
            Vec2::new(cw, plate.height() - gap * 2.0),
        );
        let shade = 14 + i as u8 * 6;
        pt.rect_filled(cell, R_SM, Color32::from_rgb(shade + 20, shade + 20, shade + 26));
    }

    // Platform badge.
    let short = if p.platform.contains("9:16") { "9:16" } else { "16:9" };
    let badge = Rect::from_min_size(
        Pos2::new(plate.right() - 42.0, plate.top() + 6.0),
        Vec2::new(36.0, 15.0),
    );
    pt.rect_filled(badge, R_SM, Color32::from_black_alpha(180));
    pt.text(
        badge.center(),
        Align2::CENTER_CENTER,
        short,
        FontId::new(9.0, FontFamily::Monospace),
        TEXT_SECONDARY,
    );

    let name = elided_galley(
        ui,
        &p.name,
        FontId::new(13.0, FontFamily::Proportional),
        TEXT_PRIMARY,
        width - 20.0,
    );
    pt.galley(
        Pos2::new(rect.left() + 10.0, plate.bottom() + 9.0),
        name,
        TEXT_PRIMARY,
    );
    let meta = elided_galley(
        ui,
        &format!("{} · {} · {} clips", p.blueprint, p.duration, p.clips),
        FontId::new(10.0, FontFamily::Monospace),
        TEXT_DISABLED,
        width - 20.0,
    );
    pt.galley(
        Pos2::new(rect.left() + 10.0, plate.bottom() + 28.0),
        meta,
        TEXT_DISABLED,
    );
    pt.text(
        Pos2::new(rect.right() - 10.0, plate.bottom() + 30.0),
        Align2::RIGHT_TOP,
        &p.modified,
        FontId::new(9.0, FontFamily::Proportional),
        TEXT_DISABLED,
    );
    resp.on_hover_text(format!("{}\nDouble-click to open the studio", p.name))
}
