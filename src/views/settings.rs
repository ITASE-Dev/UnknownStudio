//! Credentials form: the one place API keys are entered.
//!
//! Values are typed here, masked on screen, and written to `.env`. Nothing is
//! logged, and nothing but the last four characters is ever shown back.

use crate::app::credentials::{mask, Credentials};
use crate::ui::core::buttons::{ghost_button, icon_button, pro_button, segmented};
use crate::ui::core::icons;
use crate::ui::core::inputs::{pro_text_input, secret_input};
use crate::ui::core::typography::{hairline_rule, section_header};
use crate::ui::theme::tokens::*;
use eframe::egui::{self, Align, Layout, Margin, RichText, Stroke, Ui};

const PROVIDERS: [&str; 2] = ["openai", "anthropic"];

/// What the form asked the app to do when it closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsOutcome {
    /// Saved to disk; the app should reconnect anything that reads these.
    Saved,
    Cancelled,
}

pub struct SettingsState {
    pub open: bool,
    /// Working copy — edits only reach `.env` on save.
    pub draft: Credentials,
    /// Values as they were when the form opened, for the "currently set" line.
    stored: Credentials,
    reveal: bool,
    provider_index: usize,
    status: Option<Status>,
}

#[derive(Debug, Clone)]
enum Status {
    Saved(String),
    Failed(String),
}

impl Default for SettingsState {
    fn default() -> Self {
        let stored = Credentials::load();
        Self {
            open: false,
            provider_index: provider_index(&stored.provider),
            draft: stored.clone(),
            stored,
            reveal: false,
            status: None,
        }
    }
}

impl SettingsState {
    /// Re-reads from disk and opens the form.
    pub fn open(&mut self) {
        self.stored = Credentials::load();
        self.draft = self.stored.clone();
        self.provider_index = provider_index(&self.stored.provider);
        self.reveal = false;
        self.status = None;
        self.open = true;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.reveal = false;
    }

    fn save(&mut self) -> bool {
        self.draft.provider = PROVIDERS[self.provider_index.min(1)].to_string();

        match self.draft.save() {
            Ok(path) => {
                self.stored = self.draft.clone();
                self.status = Some(Status::Saved(path.display().to_string()));
                true
            }
            Err(err) => {
                self.status = Some(Status::Failed(err.to_string()));
                false
            }
        }
    }
}

/// Draws the form when open. Returns an outcome on the frame it closes.
pub fn show(ctx: &egui::Context, state: &mut SettingsState) -> Option<SettingsOutcome> {
    if !state.open {
        return None;
    }

    let mut outcome = None;
    let mut keep_open = true;

    egui::Window::new("Settings")
        .open(&mut keep_open)
        .collapsible(false)
        .resizable(false)
        .default_width(460.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .frame(
            egui::Frame::none()
                .fill(BG_PANEL)
                .stroke(Stroke::new(1.0_f32, BORDER))
                .rounding(R)
                .inner_margin(Margin::same(16.0)),
        )
        .show(ctx, |ui| {
            outcome = form(ui, state);
        });

    // The window's own close button counts as cancelling.
    if !keep_open {
        state.close();
        return Some(SettingsOutcome::Cancelled);
    }
    if outcome.is_some() {
        state.close();
    }
    outcome
}

fn form(ui: &mut Ui, state: &mut SettingsState) -> Option<SettingsOutcome> {
    let mut outcome = None;

    ui.label(
        RichText::new("Keys are stored in your local .env file and never leave this machine.")
            .small()
            .color(TEXT_SECONDARY),
    );
    ui.add_space(10.0);

    section_header(ui, "Assistant");
    ui.label(RichText::new("Provider").small().color(TEXT_SECONDARY));
    segmented(ui, &["OpenAI", "Anthropic"], &mut state.provider_index);
    ui.add_space(8.0);

    let anthropic = state.provider_index == 1;
    secret_field(
        ui,
        "OpenAI API key",
        "sk-…",
        &mut state.draft.openai_key,
        &state.stored.openai_key,
        state.reveal,
        !anthropic,
    );
    secret_field(
        ui,
        "Anthropic API key",
        "sk-ant-…",
        &mut state.draft.anthropic_key,
        &state.stored.anthropic_key,
        state.reveal,
        anthropic,
    );

    ui.add_space(2.0);
    labelled(ui, "Model", |ui| {
        pro_text_input(ui, &mut state.draft.openai_model, "gpt-4o");
    });

    ui.add_space(10.0);
    section_header(ui, "Services");
    labelled(ui, "ComfyUI address", |ui| {
        pro_text_input(ui, &mut state.draft.comfyui_url, "http://127.0.0.1:8188");
    });
    secret_field(
        ui,
        "YouTube Data API key",
        "AIza…",
        &mut state.draft.youtube_key,
        &state.stored.youtube_key,
        state.reveal,
        false,
    );

    ui.add_space(10.0);
    hairline_rule(ui);
    ui.add_space(8.0);

    if let Some(warning) = warning(state) {
        ui.label(RichText::new(warning).small().color(WARN));
        ui.add_space(6.0);
    }
    match &state.status {
        Some(Status::Saved(path)) => {
            ui.label(
                RichText::new(format!("Saved to {path}"))
                    .small()
                    .color(OK),
            );
            ui.add_space(6.0);
        }
        Some(Status::Failed(reason)) => {
            ui.label(
                RichText::new(format!("Could not save: {reason}"))
                    .small()
                    .color(ERR),
            );
            ui.add_space(6.0);
        }
        None => {}
    }

    ui.horizontal(|ui| {
        let (glyph, hint) = if state.reveal {
            (icons::HIDE_SECRET, "Hide keys")
        } else {
            (icons::SHOW_SECRET, "Show keys")
        };
        if icon_button(ui, glyph, true, state.reveal)
            .on_hover_text(hint)
            .clicked()
        {
            state.reveal = !state.reveal;
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if pro_button(ui, "Save", true).clicked() && state.save() {
                outcome = Some(SettingsOutcome::Saved);
            }
            if ghost_button(ui, "Cancel").clicked() {
                outcome = Some(SettingsOutcome::Cancelled);
            }
        });
    });

    outcome
}

/// One secret row: label, what is currently stored, and the masked field.
fn secret_field(
    ui: &mut Ui,
    label: &str,
    hint: &str,
    value: &mut String,
    stored: &str,
    reveal: bool,
    required: bool,
) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).small().color(TEXT_SECONDARY));
        if required {
            ui.label(RichText::new("required").small().color(ACCENT));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(mask(stored)).small().color(TEXT_DISABLED));
        });
    });
    secret_input(ui, value, hint, reveal);
    ui.add_space(8.0);
}

fn labelled(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    ui.label(RichText::new(label).small().color(TEXT_SECONDARY));
    add(ui);
    ui.add_space(8.0);
}

/// Problems worth naming before the user presses Save.
fn warning(state: &SettingsState) -> Option<String> {
    let draft = &state.draft;
    let anthropic = state.provider_index == 1;

    if anthropic && draft.anthropic_key.trim().is_empty() {
        return Some("Anthropic is selected but its key is empty — the assistant will stay offline.".into());
    }
    if !anthropic && draft.openai_key.trim().is_empty() {
        return Some("OpenAI is selected but its key is empty — the assistant will stay offline.".into());
    }
    if !anthropic
        && !draft.openai_key.trim().is_empty()
        && !draft.openai_key.trim().starts_with("sk-")
    {
        return Some("That does not look like an OpenAI key (they start with 'sk-').".into());
    }

    let url = draft.comfyui_url.trim();
    if !url.is_empty() && !url.starts_with("http://") && !url.starts_with("https://") {
        return Some("The ComfyUI address needs an http:// or https:// prefix.".into());
    }
    None
}

fn provider_index(provider: &str) -> usize {
    usize::from(provider.trim() == "anthropic")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(provider: usize, openai: &str, anthropic: &str) -> SettingsState {
        SettingsState {
            open: true,
            provider_index: provider,
            draft: Credentials {
                openai_key: openai.into(),
                anthropic_key: anthropic.into(),
                provider: PROVIDERS[provider].into(),
                ..Credentials::default()
            },
            stored: Credentials::default(),
            reveal: false,
            status: None,
        }
    }

    #[test]
    fn the_selected_provider_decides_which_key_is_demanded() {
        assert!(warning(&state(0, "", "sk-ant")).is_some(), "openai key missing");
        assert!(warning(&state(1, "sk-x", "")).is_some(), "anthropic key missing");
        assert!(warning(&state(1, "", "sk-ant")).is_none());
    }

    #[test]
    fn an_obviously_wrong_openai_key_is_flagged_before_saving() {
        let message = warning(&state(0, "my-password", "")).expect("warning");
        assert!(message.contains("sk-"));

        assert!(warning(&state(0, "sk-proj-abc", "")).is_none());
    }

    #[test]
    fn a_comfyui_address_without_a_scheme_is_flagged() {
        let mut settings = state(0, "sk-x", "");
        settings.draft.comfyui_url = "127.0.0.1:8188".into();
        let message = warning(&settings).expect("warning");
        assert!(message.contains("http://"));

        settings.draft.comfyui_url = "http://127.0.0.1:8188".into();
        assert!(warning(&settings).is_none());
    }

    #[test]
    fn provider_names_map_to_the_segmented_control_both_ways() {
        assert_eq!(provider_index("anthropic"), 1);
        assert_eq!(provider_index("openai"), 0);
        assert_eq!(provider_index(""), 0, "unset defaults to OpenAI");
        assert_eq!(PROVIDERS[provider_index("anthropic")], "anthropic");
    }
}
