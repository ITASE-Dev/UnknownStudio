mod action_engine;
mod app;
mod media;
mod ui;
mod views;

use eframe::egui;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 860.0])
            .with_min_inner_size([420.0, 480.0])
            .with_title("AI Video Director Studio"),
        ..Default::default()
    };
    eframe::run_native(
        "unknown_studio",
        options,
        Box::new(|cc| Box::new(app::AiDirectorApp::new(cc))),
    )
}
