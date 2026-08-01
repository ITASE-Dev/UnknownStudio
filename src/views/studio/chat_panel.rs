//! Director chat: conversation state, the async bridge, and the transcript UI.
//!
//! Nothing here blocks. A submitted prompt is handed to `ChatBridge`; replies
//! arrive on later frames through `poll`.

use crate::ai_tooling::chat::{ChatBridge, ChatEvent, ChatSession, Message, Role};
use crate::ai_tooling::orchestration::{ActionCommand, PromptBuilder, PromptContext};
use crate::models::MediaSelection;
use crate::ui::components::chat::{
    typing_indicator, AiChatBubble, NoticeBubble, PromptInputArea, UserChatBubble,
};
use crate::ui::components::status::{service_status_bar, ServiceState};
use crate::ui::core::typography::{hairline_rule, section_header};
use crate::ui::theme::tokens::*;
use eframe::egui::{RichText, ScrollArea, Ui};

/// Persona handed to the prompt builder; the editor's state and the action list
/// are appended to it whenever the timeline changes.
const PERSONA: &str = "You are UnknownStudio AI, an advanced video editing director. \
YOU HAVE DIRECT ACCESS to the video timeline via tool calls. \
NEVER say 'I cannot add a marker' or 'I cannot edit video'. \
When the user asks you to cut, mark, or edit, you MUST respond by calling the appropriate tool \
function. Answer as an editor would: concrete and short, in the language the user writes in.";

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
    /// Signature of the editor state currently injected, so an unchanged
    /// timeline does not rebuild the prompt every frame.
    injected: Option<u64>,
    /// Tool calls waiting to be dispatched against the timeline.
    pending_actions: Vec<ActionCommand>,
}

impl Default for ChatState {
    fn default() -> Self {
        let (bridge, error) = match ChatBridge::from_env() {
            Ok(bridge) => (Some(bridge), None),
            Err(err) => (None, Some(err.to_string())),
        };

        Self {
            session: ChatSession::with_system(PERSONA),
            prompt: String::new(),
            waiting: false,
            error,
            bridge,
            injected: None,
            pending_actions: Vec::new(),
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
        self.injected = None;
        self.waiting = false;
    }

    /// Injects the editor's current state into the system prompt. Called every
    /// frame; rebuilds only when what the model would see actually changed.
    pub fn inject_state(&mut self, context: &PromptContext<'_>) {
        let signature = context.signature();
        if self.injected == Some(signature) {
            return;
        }
        self.injected = Some(signature);
        self.session
            .set_system(PromptBuilder::new().with_persona(PERSONA).build(context));
    }

    /// Adds a right-clicked item to the conversation as context. No completion
    /// is requested: the user still has to say what to do with it.
    pub fn attach_selection(&mut self, selection: &MediaSelection) {
        self.session
            .push_user(format!("{} Awaiting instructions.", selection.context_line()));
        self.error = None;
    }

    /// Tool calls the model made, handed over for dispatch. Draining them here
    /// keeps the panel free of any knowledge of the timeline.
    pub fn take_actions(&mut self) -> Vec<ActionCommand> {
        std::mem::take(&mut self.pending_actions)
    }

    /// Reports back what the dispatcher did, so the next turn has it in context.
    pub fn report_dispatch(&mut self, feedback: &str) {
        let feedback = feedback.trim();
        if !feedback.is_empty() {
            self.session.push_assistant(feedback.to_string());
        }
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
                ChatEvent::Reply(response) => {
                    if let Some(message) = response.message {
                        self.session.push(message);
                    }
                    // A tool call with no prose still deserves a transcript
                    // line, or the panel looks like nothing happened.
                    if !response.actions.is_empty() {
                        let names: Vec<&str> = response
                            .actions
                            .iter()
                            .map(ActionCommand::name)
                            .collect();
                        self.session
                            .push_assistant(format!("▸ {}", names.join(", ")));
                    }
                    for rejected in response.rejected {
                        self.error = Some(format!("tool call ignored: {rejected}"));
                    }
                    self.pending_actions.extend(response.actions);
                }
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
