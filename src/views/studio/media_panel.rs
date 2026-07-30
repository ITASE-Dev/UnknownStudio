use crate::ui::components::inspector::{inspector_group, thumbnail_preview_box};
use crate::ui::components::media::{media_pool_grid, AssetKind, PoolAsset};
use crate::ui::core::buttons::{ai_button, segmented};
use crate::ui::core::inputs::search_input;
use crate::ui::core::typography::{property_row, section_header};
use crate::ui::theme::tokens::*;
use eframe::egui::{RichText, ScrollArea, Ui};

pub struct Asset {
    pub name: String,
    pub meta: String,
    pub duration: String,
    pub generated: bool,
}

pub struct MediaState {
    pub assets: Vec<Asset>,
    pub search: String,
    pub filter: usize,
    pub selected: usize,
}

impl Default for MediaState {
    fn default() -> Self {
        Self {
            assets: vec![
                Asset { name: "Clip_01.mp4".into(), meta: "1920×1080 · 30p".into(), duration: "00:06".into(), generated: false },
                Asset { name: "Clip_02.mp4".into(), meta: "1920×1080 · 30p".into(), duration: "00:07".into(), generated: false },
                Asset { name: "gen_neon_city.mp4".into(), meta: "flux · seed 8812".into(), duration: "00:04".into(), generated: true },
                Asset { name: "gen_server_rack.mp4".into(), meta: "flux · seed 4410".into(), duration: "00:03".into(), generated: true },
                Asset { name: "VO_take3.wav".into(), meta: "48 kHz · mono".into(), duration: "00:11".into(), generated: false },
                Asset { name: "bed_music.wav".into(), meta: "48 kHz · stereo".into(), duration: "00:09".into(), generated: false },
            ],
            search: String::new(),
            filter: 0,
            selected: 0,
        }
    }
}

pub fn show(ui: &mut Ui, state: &mut MediaState) {
    section_header(ui, "Media pool");
    search_input(ui, &mut state.search, "Search assets…");
    ui.add_space(6.0);
    segmented(ui, &["All", "Camera", "Generated"], &mut state.filter);
    ui.add_space(8.0);

    let needle = state.search.trim().to_lowercase();
    let indices: Vec<usize> = state
        .assets
        .iter()
        .enumerate()
        .filter(|(_, a)| match state.filter {
            1 => !a.generated,
            2 => a.generated,
            _ => true,
        })
        .filter(|(_, a)| needle.is_empty() || a.name.to_lowercase().contains(&needle))
        .map(|(i, _)| i)
        .collect();

    // Split the remaining height so neither half can run off the panel edge.
    let avail = ui.available_height();
    let pool_h = (avail * 0.52).clamp(80.0, (avail - 120.0).max(80.0));

    let clicked = ScrollArea::vertical()
        .auto_shrink([false, false])
        .max_height(pool_h)
        .id_source("media_pool_scroll")
        .show(ui, |ui| {
            let items: Vec<PoolAsset<'_>> = indices
                .iter()
                .map(|&i| {
                    let a = &state.assets[i];
                    PoolAsset {
                        name: &a.name,
                        meta: &a.meta,
                        duration: &a.duration,
                        kind: if a.generated {
                            AssetKind::Generated
                        } else {
                            AssetKind::Ingested
                        },
                        selected: i == state.selected,
                    }
                })
                .collect();
            if items.is_empty() {
                ui.label(RichText::new("No assets match.").small().color(TEXT_DISABLED));
                return None;
            }
            media_pool_grid(ui, &items)
        })
        .inner;

    if let Some(pos) = clicked {
        if let Some(&idx) = indices.get(pos) {
            state.selected = idx;
        }
    }

    ui.add_space(8.0);
    ScrollArea::vertical()
        .auto_shrink([false, false])
        .id_source("media_inspector_scroll")
        .show(ui, |ui| inspector(ui, state));
}

fn inspector(ui: &mut Ui, state: &mut MediaState) {
    let Some(asset) = state.assets.get(state.selected) else {
        return;
    };
    let (name, meta, duration, generated) = (
        asset.name.clone(),
        asset.meta.clone(),
        asset.duration.clone(),
        asset.generated,
    );
    inspector_group(ui, "Inspector", |ui| {
        thumbnail_preview_box(ui, &name);
        ui.add_space(8.0);
        property_row(ui, "Duration", &duration);
        property_row(ui, "Format", &meta);
        property_row(ui, "Source", if generated { "generated" } else { "ingested" });
        if generated {
            ui.add_space(8.0);
            if ai_button(ui, "Re-roll shot").clicked() {
                // Regeneration is a backend concern; the button proves the surface.
            }
        }
    });
}
