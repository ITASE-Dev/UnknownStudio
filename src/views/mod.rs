//! Smart screens. Each view owns its local UI state and navigates by writing to
//! `&mut AppRoute`; none of them know about each other.
#![allow(dead_code)]

pub mod dashboard;
pub mod gallery;
pub mod growth;
pub mod onboarding;
pub mod studio;

use crate::ui::core::typography::content_column;
use crate::ui::theme::tokens::*;
use eframe::egui::{self, Margin, ScrollArea};

/// Shared page chrome for the non-editor views: app-fill background, responsive
/// gutters and a capped reading measure.
pub fn page<R>(ctx: &egui::Context, max_width: f32, add: impl FnOnce(&mut egui::Ui) -> R) {
    egui::CentralPanel::default()
        .frame(egui::Frame::none().fill(BG_APP))
        .show(ctx, |ui| {
            ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let pad = if ui.available_width() < 520.0 { 12.0 } else { 28.0 };
                    egui::Frame::none()
                        .inner_margin(Margin::symmetric(pad, 18.0))
                        .show(ui, |ui| {
                            content_column(ui, max_width, |ui| {
                                add(ui);
                                ui.add_space(32.0);
                            });
                        });
                });
        });
}
