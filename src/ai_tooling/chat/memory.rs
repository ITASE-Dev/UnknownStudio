//! Conversation state and the pruning that keeps it inside the model's window.

use crate::ai_tooling::chat::models::{estimated_tokens, Message, Role};
use serde::{Deserialize, Serialize};

/// Limits a session is kept within. Defaults suit a 128k-context model with
/// room left for the completion itself.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ContextBudget {
    /// Estimated tokens the history may occupy, excluding the system prompt.
    pub max_tokens: usize,
    /// Hard cap on retained turns, so a long chat of tiny messages is bounded too.
    pub max_messages: usize,
}

impl Default for ContextBudget {
    fn default() -> Self {
        Self {
            max_tokens: 24_000,
            max_messages: 40,
        }
    }
}

/// One conversation: a pinned system prompt plus the turns after it.
///
/// The system prompt is stored apart from the history because it is never
/// pruned — dropping it would silently change the assistant's behaviour.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatSession {
    system: Option<Message>,
    history: Vec<Message>,
    budget: ContextBudget,
}

impl Default for ChatSession {
    fn default() -> Self {
        Self::new()
    }
}

impl ChatSession {
    pub fn new() -> Self {
        Self {
            system: None,
            history: Vec::new(),
            budget: ContextBudget::default(),
        }
    }

    pub fn with_system(prompt: impl Into<String>) -> Self {
        Self {
            system: Some(Message::system(prompt)),
            ..Self::new()
        }
    }

    pub fn with_budget(mut self, budget: ContextBudget) -> Self {
        self.budget = budget;
        self.prune();
        self
    }

    /// Replaces the standing instructions without touching the turns.
    pub fn set_system(&mut self, prompt: impl Into<String>) {
        self.system = Some(Message::system(prompt));
    }

    pub fn system(&self) -> Option<&Message> {
        self.system.as_ref()
    }

    pub fn history(&self) -> &[Message] {
        &self.history
    }

    pub fn budget(&self) -> ContextBudget {
        self.budget
    }

    pub fn is_empty(&self) -> bool {
        self.history.is_empty()
    }

    pub fn len(&self) -> usize {
        self.history.len()
    }

    pub fn push_user(&mut self, content: impl Into<String>) {
        self.push(Message::user(content));
    }

    pub fn push_assistant(&mut self, content: impl Into<String>) {
        self.push(Message::assistant(content));
    }

    /// Appends a turn and prunes back into budget. A `System` message replaces
    /// the pinned prompt rather than joining the history.
    pub fn push(&mut self, message: Message) {
        if message.role == Role::System {
            self.system = Some(message);
            return;
        }
        self.history.push(message);
        self.prune();
    }

    /// Everything to send, system prompt first.
    pub fn context(&self) -> Vec<Message> {
        let mut context = Vec::with_capacity(self.history.len() + 1);
        context.extend(self.system.clone());
        context.extend(self.history.iter().cloned());
        context
    }

    /// Estimated cost of the whole payload, system prompt included.
    pub fn estimated_tokens(&self) -> usize {
        estimated_tokens(&self.context())
    }

    /// Forgets every turn, keeping the system prompt.
    pub fn clear(&mut self) {
        self.history.clear();
    }

    /// Restores a stored conversation, pruning whatever no longer fits.
    pub fn restore(&mut self, messages: impl IntoIterator<Item = Message>) {
        self.history.clear();
        for message in messages {
            match message.role {
                Role::System => self.system = Some(message),
                _ => self.history.push(message),
            }
        }
        self.prune();
    }

    /// Drops the oldest turns until both limits hold.
    ///
    /// Oldest-first keeps the recent exchange — the part the next answer
    /// depends on — and the last message is always retained, even when a single
    /// message exceeds the budget on its own.
    fn prune(&mut self) {
        if self.history.len() > self.budget.max_messages {
            let excess = self.history.len() - self.budget.max_messages;
            self.history.drain(..excess);
        }

        let system_cost = self.system.as_ref().map_or(0, Message::estimated_tokens);
        let mut total = system_cost + estimated_tokens(&self.history);

        while total > self.budget.max_tokens && self.history.len() > 1 {
            let dropped = self.history.remove(0);
            total -= dropped.estimated_tokens();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(max_tokens: usize, max_messages: usize) -> ChatSession {
        ChatSession::with_system("be brief").with_budget(ContextBudget {
            max_tokens,
            max_messages,
        })
    }

    #[test]
    fn context_leads_with_the_system_prompt() {
        let mut chat = ChatSession::with_system("be brief");
        chat.push_user("hello");
        chat.push_assistant("hi");

        let context = chat.context();
        assert_eq!(context.len(), 3);
        assert_eq!(context[0].role, Role::System);
        assert_eq!(context[1].content, "hello");
        assert_eq!(context[2].role, Role::Assistant);
    }

    #[test]
    fn a_system_message_replaces_the_prompt_instead_of_queueing() {
        let mut chat = ChatSession::with_system("first");
        chat.push(Message::system("second"));
        chat.push_user("hello");

        assert_eq!(chat.system().expect("system").content, "second");
        assert_eq!(chat.len(), 1);
    }

    #[test]
    fn the_message_cap_drops_the_oldest_turns() {
        let mut chat = session(100_000, 4);
        for i in 0..10 {
            chat.push_user(format!("turn {i}"));
        }

        assert_eq!(chat.len(), 4);
        assert_eq!(chat.history()[0].content, "turn 6");
        assert_eq!(chat.history()[3].content, "turn 9");
    }

    #[test]
    fn the_token_budget_keeps_the_recent_exchange() {
        // Each message costs 25 + 4 tokens.
        let mut chat = session(100, 100);
        for i in 0..10 {
            chat.push_user(format!("{i}{}", "x".repeat(99)));
        }

        assert!(chat.estimated_tokens() <= 100);
        assert!(chat.history().last().expect("last").content.starts_with('9'));
    }

    #[test]
    fn the_newest_message_survives_even_when_oversized() {
        let mut chat = session(10, 100);
        chat.push_user("x".repeat(10_000));

        assert_eq!(chat.len(), 1);
        assert!(chat.system().is_some(), "the system prompt is never pruned");
    }

    #[test]
    fn restore_rebuilds_a_stored_conversation() {
        let mut chat = session(100_000, 3);
        chat.restore([
            Message::system("stored prompt"),
            Message::user("one"),
            Message::assistant("two"),
            Message::user("three"),
            Message::assistant("four"),
        ]);

        assert_eq!(chat.system().expect("system").content, "stored prompt");
        assert_eq!(chat.len(), 3, "pruned to the message cap");
        assert_eq!(chat.history()[0].content, "two");

        chat.clear();
        assert!(chat.is_empty());
        assert!(chat.system().is_some());
    }
}
