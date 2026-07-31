//! Director chat: conversation state, the async bridge, and the transcript UI.
//!
//! Nothing here blocks. A submitted prompt is handed to `ChatBridge`; replies
//! arrive on later frames through `poll`.

use crate::ai_tooling::chat::{ChatBridge, ChatEvent, ChatSession, Message, Role};
use crate::ui::components::chat::{
    typing_indicator, AiChatBubble, NoticeBubble, PromptInputArea, UserChatBubble,
};
use crate::ui::components::status::{service_status_bar, ServiceState};
use crate::ui::core::typography::{hairline_rule, section_header};
use crate::ui::theme::tokens::*;
use eframe::egui::{RichText, ScrollArea, Ui};

/// Standing instructions for the editing assistant.
const SYSTEM_PROMPT: &str = "You are the AI director inside Unknown Studio, a video editor. \
Answer as an editor would: concrete, short, and about the cut in front of you. \
Reply in the language the user writes in. Never invent timecodes or clip names you were not given.";

/// Height reserved for the composer so the transcript can claim the rest.
const COMPOSER_HEIGHT: f32 = 126.0;

pub struct ChatState {
    /// Conversation history, pruned to the model's context budget.
    pub session: ChatSession,
    pub prompt: String,
    /// A completion is in flight; the composer stays disabled until it lands.
    pub waiting: bool,
    /// Last transport or configuration failure, shown below the transcript.
    pub error: Option<String>,
    /// `None` when the assistant could not be configured (no API key).
    bridge: Option<ChatBridge>,
}

impl Default for ChatState {
    fn default() -> Self {
        let (bridge, error) = match ChatBridge::from_env() {
            Ok(bridge) => (Some(bridge), None),
            Err(err) => (None, Some(err.to_string())),
        };

        Self {
            session: ChatSession::with_system(SYSTEM_PROMPT),
            prompt: String::new(),
            waiting: false,
            error,
            bridge,
        }
    }
}

impl ChatState {
    pub fn is_online(&self) -> bool {
        self.bridge.is_some()
    }

    /// Turns rendered in the transcript, oldest first.
    pub fn history(&self) -> &[Message] {
        self.session.history()
    }

    /// Replaces the conversation, e.g. when a project is opened.
    pub fn restore(&mut self, messages: impl IntoIterator<Item = Message>) {
        self.session.restore(messages);
        self.session.set_system(SYSTEM_PROMPT);
        self.waiting = false;
    }

    /// Records the turn and asks the worker for a reply.
    pub fn submit(&mut self, prompt: String) {
        self.session.push_user(prompt);
        self.error = None;

        let Some(bridge) = &self.bridge else {
            self.error = Some("Assistant offline: set OPENAI_API_KEY in .env.".into());
            return;
        };

        // A dead worker must not leave the composer disabled forever.
        self.waiting = bridge.request(self.session.context());
        if !self.waiting {
            self.error = Some("Assistant stopped responding; restart the app.".into());
        }
    }

    /// Drains finished completions. Call once per frame, before drawing.
    pub fn poll(&mut self) {
        let Some(bridge) = &self.bridge else {
            return;
        };

        for event in bridge.poll().collect::<Vec<_>>() {
            self.waiting = false;
            match event {
                ChatEvent::Reply(message) => self.session.push(message),
                ChatEvent::Failed(reason) => self.error = Some(reason),
            }
        }
    }
}

pub fn show(ui: &mut Ui, state: &mut ChatState, time: f32) {
    state.poll();

    ui.horizontal(|ui| {
        section_header(ui, "Director");
    });
    service_status_bar(
        ui,
        &[(
            "LLM Director",
            if state.is_online() {
                ServiceState::Online
            } else {
                ServiceState::Error
            },
        )],
        time,
    );
    ui.add_space(6.0);
    hairline_rule(ui);
    ui.add_space(8.0);

    transcript(ui, state, time);

    ui.add_space(6.0);
    match &state.error {
        Some(error) => {
            NoticeBubble::show(ui, error);
        }
        None => {
            ui.label(
                RichText::new("Edits apply to the timeline; nothing renders until you export.")
                    .small()
                    .color(TEXT_DISABLED),
            );
        }
    }
    ui.add_space(4.0);

    // Disabled while a completion is in flight: one request at a time keeps
    // replies in the same order as the turns on screen.
    if let Some(prompt) = PromptInputArea::show(ui, &mut state.prompt, !state.waiting) {
        state.submit(prompt);
    }
}

fn transcript(ui: &mut Ui, state: &ChatState, time: f32) {
    let height = (ui.available_height() - COMPOSER_HEIGHT).max(80.0);

    ScrollArea::vertical()
        .auto_shrink([false, false])
        .max_height(height)
        .stick_to_bottom(true)
        .id_source("director_transcript")
        .show(ui, |ui| {
            if state.history().is_empty() {
                ui.label(
                    RichText::new("Ask for a cut, a pacing pass, or B-roll.")
                        .small()
                        .color(TEXT_DISABLED),
                );
            }

            for message in state.history() {
                match message.role {
                    Role::User => UserChatBubble::show(ui, &message.content),
                    Role::Assistant => AiChatBubble::show(ui, &message.content),
                    // The system prompt is context, not conversation.
                    Role::System => continue,
                };
                ui.add_space(8.0);
            }

            if state.waiting {
                typing_indicator(ui, time);
            }
        });
}
