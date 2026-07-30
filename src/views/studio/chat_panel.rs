use crate::ui::components::chat::{
    ai_chat_bubble, prompt_input_area, typing_indicator, user_chat_bubble,
};
use crate::ui::components::status::{job_progress, service_status_bar, ServiceState};
use crate::ui::core::typography::{hairline_rule, section_header};
use crate::ui::theme::tokens::*;
use eframe::egui::{RichText, ScrollArea, Ui};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Author {
    User,
    Director,
}

pub struct Message {
    pub author: Author,
    pub body: String,
}

pub struct ChatState {
    pub messages: Vec<Message>,
    pub prompt: String,
    pub thinking: bool,
}

impl Default for ChatState {
    fn default() -> Self {
        Self {
            messages: vec![
                Message {
                    author: Author::Director,
                    body: "Ingested 34 clips and 2 audio stems. Rough assembly is on V1 with silences already pulled.".into(),
                },
                Message {
                    author: Author::User,
                    body: "Cut the first minute to 40 seconds, keep the hook.".into(),
                },
                Message {
                    author: Author::Director,
                    body: "Trimmed 22s of silence and dropped two redundant takes. Added generative B-Roll over the product mention at 00:18 — Identity Lock kept the presenter untouched.".into(),
                },
            ],
            prompt: String::new(),
            thinking: true,
        }
    }
}

impl ChatState {
    fn push_user(&mut self, body: String) {
        self.messages.push(Message { author: Author::User, body });
        self.messages.push(Message {
            author: Author::Director,
            body: "Queued. I'll re-time the affected range and report the new duration.".into(),
        });
    }
}

pub fn show(ui: &mut Ui, state: &mut ChatState, time: f32) {
    ui.horizontal(|ui| {
        section_header(ui, "Director");
    });
    service_status_bar(
        ui,
        &[
            ("LLM Director", ServiceState::Online),
            ("ComfyUI", ServiceState::Working),
        ],
        time,
    );
    ui.add_space(6.0);
    hairline_rule(ui);
    ui.add_space(8.0);

    // Composer is measured first so the transcript can claim the rest.
    let composer_h = 126.0;
    let transcript_h = (ui.available_height() - composer_h).max(80.0);

    ScrollArea::vertical()
        .auto_shrink([false, false])
        .max_height(transcript_h)
        .stick_to_bottom(true)
        .show(ui, |ui| {
            for m in &state.messages {
                match m.author {
                    Author::User => {
                        user_chat_bubble(ui, &m.body);
                    }
                    Author::Director => {
                        ai_chat_bubble(ui, "Director", &m.body);
                    }
                }
                ui.add_space(8.0);
            }
            if state.thinking {
                typing_indicator(ui, time);
                ui.add_space(4.0);
                job_progress(ui, "Re-timing V1 · 3 of 8 edits", 0.42);
            }
        });

    ui.add_space(6.0);
    ui.label(
        RichText::new("Edits apply to the timeline; nothing renders until you export.")
            .small()
            .color(TEXT_DISABLED),
    );
    ui.add_space(4.0);
    let mut sent: Option<String> = None;
    let prompt_snapshot = state.prompt.clone();
    if prompt_input_area(ui, &mut state.prompt) {
        sent = Some(prompt_snapshot);
    }
    if let Some(body) = sent {
        let body = body.trim().to_owned();
        if !body.is_empty() {
            state.push_user(body);
            state.thinking = true;
        }
    }
}
