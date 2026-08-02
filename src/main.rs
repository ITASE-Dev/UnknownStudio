mod action_engine;
mod ai_tooling;
mod audio_engine;
mod workspace;
mod app;
mod media;
mod models;
mod ui;
mod views;

use eframe::egui;

fn main() -> eframe::Result<()> {
    // Before any subsystem reads a key: action_engine and ai_tooling both use
    // plain `std::env::var` at construction time.
    ai_tooling::config::load_dotenv();

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
