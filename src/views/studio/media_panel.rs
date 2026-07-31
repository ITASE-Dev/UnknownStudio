use crate::media::{self, ImportedMedia, MediaKind, Textures};
use crate::ui::components::inspector::{inspector_group, preview_plate};
use crate::ui::components::media::{media_pool_grid, AssetKind, PoolAsset};
use crate::ui::components::timeline::clip::ClipKind;
use crate::ui::core::buttons::{ai_button, pro_button, segmented};
use crate::ui::core::inputs::search_input;
use crate::ui::core::typography::{property_row, section_header};
use crate::ui::theme::tokens::*;
use crate::views::studio::dnd::DragAsset;
use eframe::egui::{RichText, ScrollArea, Ui};
use std::path::PathBuf;

pub struct Asset {
    pub name: String,
    pub meta: String,
    pub duration: String,
    pub generated: bool,
    /// Real file behind this card. Seeded demo assets have none.
    pub path: Option<PathBuf>,
    pub kind: MediaKind,
    pub seconds: f32,
    pub has_audio: bool,
}

impl Asset {
    fn demo(name: &str, meta: &str, seconds: f32, kind: MediaKind, generated: bool) -> Self {
        Self {
            name: name.into(),
            meta: meta.into(),
            duration: media::format_duration(seconds),
            generated,
            path: None,
            kind,
            seconds,
            has_audio: kind == MediaKind::Audio,
        }
    }

    fn imported(media: ImportedMedia) -> Self {
        Self {
            name: media.name,
            meta: media.meta,
            duration: media::format_duration(media.seconds),
            generated: false,
            path: Some(media.path),
            kind: media.kind,
            seconds: media.seconds,
            has_audio: media.has_audio,
        }
    }

    /// Timeline block type this asset lands on.
    pub fn clip_kind(&self) -> ClipKind {
        match self.kind {
            MediaKind::Audio => ClipKind::Audio,
            _ if self.generated => ClipKind::BRoll,
            _ => ClipKind::ARoll,
        }
    }

    fn drag_payload(&self) -> DragAsset {
        DragAsset {
            name: self.name.clone(),
            path: self.path.clone(),
            seconds: self.seconds.max(0.2),
            kind: self.clip_kind(),
            has_audio: self.has_audio,
        }
    }
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
            // Real files only — a card in the pool always has media behind it.
            assets: Vec::new(),
            search: String::new(),
            filter: 0,
            selected: 0,
        }
    }
}

impl MediaState {
    /// Probes and appends files; unsupported ones are skipped. Selection lands
    /// on the first new item so the inspector shows what just arrived.
    pub fn import_paths(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        let first_new = self.assets.len();
        for path in paths {
            if self.assets.iter().any(|a| a.path.as_deref() == Some(path.as_path())) {
                continue;
            }
            if let Some(imported) = media::import(path) {
                self.assets.push(Asset::imported(imported));
            }
        }
        if self.assets.len() > first_new {
            self.selected = first_new;
        }
    }
}

/// Returns a drag payload on the frame a card starts being dragged.
pub fn show(ui: &mut Ui, state: &mut MediaState, textures: &Textures) -> Option<DragAsset> {
    section_header(ui, "Media pool");

    if pro_button(ui, "Import media…", true).clicked() {
        state.import_paths(media::pick_files());
    }
    ui.add_space(6.0);

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

    let events = ScrollArea::vertical()
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
                        thumb: a
                            .path
                            .as_ref()
                            .and_then(|p| textures.get(&p.to_string_lossy())),
                    }
                })
                .collect();
            if items.is_empty() {
                ui.label(
                    RichText::new("No assets match. Import media to get started.")
                        .small()
                        .color(TEXT_DISABLED),
                );
                return Default::default();
            }
            media_pool_grid(ui, &items)
        })
        .inner;

    if let Some(&idx) = events.clicked.and_then(|pos| indices.get(pos)) {
        state.selected = idx;
    }

    let drag = events
        .drag_started
        .and_then(|pos| indices.get(pos).copied())
        .inspect(|&idx| state.selected = idx)
        .and_then(|idx| state.assets.get(idx))
        .map(Asset::drag_payload);

    ui.add_space(8.0);
    ui.label(
        RichText::new("Drag an asset onto a timeline track to place it.")
            .small()
            .color(TEXT_DISABLED),
    );
    ui.add_space(6.0);
    ScrollArea::vertical()
        .auto_shrink([false, false])
        .id_source("media_inspector_scroll")
        .show(ui, |ui| inspector(ui, state, textures));

    drag
}

fn inspector(ui: &mut Ui, state: &mut MediaState, textures: &Textures) {
    let Some(asset) = state.assets.get(state.selected) else {
        return;
    };
    let (name, meta, duration, generated) = (
        asset.name.clone(),
        asset.meta.clone(),
        asset.duration.clone(),
        asset.generated,
    );
    let source = match &asset.path {
        Some(path) => path.to_string_lossy().into_owned(),
        None if generated => "generated".to_string(),
        None => "demo".to_string(),
    };

    let image = asset
        .path
        .as_ref()
        .and_then(|p| textures.get(&p.to_string_lossy()));

    inspector_group(ui, "Inspector", |ui| {
        let width = ui.available_width();
        preview_plate(ui, &name, width, image);
        ui.add_space(8.0);
        property_row(ui, "Duration", &duration);
        property_row(ui, "Format", &meta);
        property_row(ui, "Source", &source);
        if generated {
            ui.add_space(8.0);
            if ai_button(ui, "Re-roll shot").clicked() {
                // Regeneration is a backend concern; the button proves the surface.
            }
        }
    });
}
