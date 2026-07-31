//! Tool strip: the FFmpeg operations from `action_engine::tools`, as a row of
//! icon buttons on the timeline.
//!
//! The strip is presentational — it reports which tool was pressed and lets the
//! caller decide whether the current selection can satisfy it.

use crate::ui::core::buttons::icon_button;
use crate::ui::core::icons;
use crate::ui::theme::tokens::*;
use eframe::egui::{Response, RichText, Ui};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    /// Render the clip's visible range to its own file.
    Trim,
    /// Re-time the clip (factor comes from the caller).
    Speed,
    /// Reframe to a vertical short.
    Crop,
    /// Composite the selected clip over the one below it.
    Overlay,
    /// Join every clip on the track into one file.
    Concat,
    /// Write the clip's audio to a separate file and add it to the pool.
    ExtractAudio,
    /// Drop the clip's audio track.
    MuteAudio,
}

impl Tool {
    pub const ALL: [Tool; 7] = [
        Tool::Trim,
        Tool::Speed,
        Tool::Crop,
        Tool::Overlay,
        Tool::Concat,
        Tool::ExtractAudio,
        Tool::MuteAudio,
    ];

    /// Position in `ALL`, for indexing an availability table.
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Trim => icons::TRIM,
            Self::Speed => icons::SPEED,
            Self::Crop => icons::CROP,
            Self::Overlay => icons::OVERLAY,
            Self::Concat => icons::CONCAT,
            Self::ExtractAudio => icons::EXTRACT_AUDIO,
            Self::MuteAudio => icons::MUTE_AUDIO,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Trim => "Trim to selection",
            Self::Speed => "Change speed",
            Self::Crop => "Reframe to 9:16",
            Self::Overlay => "Overlay on track below",
            Self::Concat => "Join track into one clip",
            Self::ExtractAudio => "Extract audio",
            Self::MuteAudio => "Remove audio",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Self::Trim => "Renders the clip's visible range to a new file.",
            Self::Speed => "Re-times the clip; its timeline length follows.",
            Self::Crop => "Center cover-crop to 1080×1920.",
            Self::Overlay => "Composites the selected clip over the track below.",
            Self::Concat => "Renders the whole track into a single clip.",
            Self::ExtractAudio => "Writes the audio to a file and adds it to the pool.",
            Self::MuteAudio => "Re-renders the clip without its audio track.",
        }
    }
}

/// Draws the strip. `enabled` decides which tools the current selection can
/// satisfy; disabled buttons explain themselves on hover instead of vanishing.
pub fn tool_strip(
    ui: &mut Ui,
    busy: Option<Tool>,
    enabled: impl Fn(Tool) -> Result<(), &'static str>,
) -> Option<Tool> {
    let mut pressed = None;

    ui.horizontal(|ui| {
        for tool in Tool::ALL {
            let availability = enabled(tool);
            let running = busy == Some(tool);
            let resp = tool_button(ui, tool, running, availability.is_ok());

            let tip = match availability {
                _ if running => format!("{}\nrunning…", tool.label()),
                Ok(()) => format!("{}\n{}", tool.label(), tool.hint()),
                Err(reason) => format!("{}\n{reason}", tool.label()),
            };
            if resp.on_hover_text(tip).clicked() && availability.is_ok() && busy.is_none() {
                pressed = Some(tool);
            }
        }
    });

    pressed
}

fn tool_button(ui: &mut Ui, tool: Tool, running: bool, available: bool) -> Response {
    if available {
        return icon_button(ui, tool.icon(), true, running);
    }

    // Kept clickable so the hover text can explain why it is unavailable.
    let resp = icon_button(ui, tool.icon(), true, false);
    ui.painter().rect_filled(
        resp.rect,
        R_SM,
        BG_PANEL.gamma_multiply(0.55),
    );
    ui.painter().text(
        resp.rect.center(),
        eframe::egui::Align2::CENTER_CENTER,
        tool.icon(),
        eframe::egui::FontId::proportional(14.0),
        TEXT_DISABLED,
    );
    resp
}

/// Status line for the running or last-finished tool.
pub fn tool_status(ui: &mut Ui, status: Option<&str>) {
    let Some(status) = status else {
        return;
    };
    ui.label(RichText::new(status).small().color(TEXT_SECONDARY));
}
