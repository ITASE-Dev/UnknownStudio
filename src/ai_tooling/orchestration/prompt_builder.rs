//! Builds the system prompt: persona, the editor's current state, and the
//! commands the model is allowed to emit.
//!
//! Everything is written for the model, not for a human reader — short keys,
//! no prose, no repetition. The legend is what makes the dense state readable.

use crate::ai_tooling::orchestration::context_models::{
    MediaPoolContext, SelectionContext, TimelineContext,
};

/// Describes one action the model may return — the catalogue entry, not the
/// parsed command itself (see `dispatcher::ActionCommand` for that).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandSpec {
    /// Emitted name, e.g. `SPLIT`.
    pub name: &'static str,
    /// Argument list as the model must write it.
    pub signature: &'static str,
    /// Half a line at most — the name carries most of the meaning.
    pub summary: &'static str,
}

impl CommandSpec {
    const fn new(name: &'static str, signature: &'static str, summary: &'static str) -> Self {
        Self {
            name,
            signature,
            summary,
        }
    }
}

/// The instruction set, mirroring what the studio can actually execute.
/// Adding an executable action means adding it here, or the model will never
/// know it exists.
pub const ACTION_COMMANDS: &[CommandSpec] = &[
    CommandSpec::new("SPLIT", "clip,at", "cut a clip at a timeline second"),
    CommandSpec::new("DELETE", "clip", "remove a clip, closing the gap"),
    CommandSpec::new("TRIM", "clip,start,end", "keep only this range of a clip"),
    CommandSpec::new("SPEED", "clip,factor", "re-time a clip (0.125-8.0)"),
    CommandSpec::new("MUTE", "clip", "drop a clip's audio"),
    CommandSpec::new("EXTRACT_AUDIO", "clip", "write a clip's audio to the pool"),
    CommandSpec::new("CROP_VERTICAL", "clip", "reframe a clip to 9:16"),
    CommandSpec::new("OVERLAY", "base,top,start,end", "composite one clip over another"),
    CommandSpec::new("CONCAT", "track", "render a whole track into one clip"),
    CommandSpec::new("PLACE", "asset,track,at", "put a pool asset on a track"),
    CommandSpec::new("SET_PLAYHEAD", "at", "move the playhead"),
];

/// Explains the state notation once, so the state itself needs no labels.
const STATE_LEGEND: &str = "STATE FORMAT: `tN name kind [muted] [locked]` is a track; \
indented `cID \"name\" start-end [a]` is a clip in timeline seconds, `a`=has audio. \
Pool entries are `\"name\" duration kind [gen]`.";

const DEFAULT_PERSONA: &str = "You are the director inside Unknown Studio, a video editor. \
You see the current timeline and media pool. Answer as an editor: concrete and short.";

const OUTPUT_RULES: &str = "RULES: refer to clips by their cID and assets by name — never invent \
either. Never state a timecode that is not in STATE. When the user asks for an edit, reply with \
the ACTIONS you would run, one per line, using the exact signatures. When they ask a question, \
answer in prose with no ACTIONS.";

/// Everything the injector needs. Borrowed, so building costs one string.
pub struct PromptContext<'a> {
    pub timeline: &'a TimelineContext,
    pub pool: &'a MediaPoolContext,
    pub selection: Option<&'a SelectionContext>,
}

impl<'a> PromptContext<'a> {
    pub fn new(timeline: &'a TimelineContext, pool: &'a MediaPoolContext) -> Self {
        Self {
            timeline,
            pool,
            selection: None,
        }
    }

    pub fn with_selection(mut self, selection: Option<&'a SelectionContext>) -> Self {
        self.selection = selection;
        self
    }

    /// Cheap change detector: rebuilding the prompt every frame is wasted work,
    /// and re-sending an unchanged system prompt is wasted tokens.
    pub fn signature(&self) -> u64 {
        let mut signature = self.timeline.clip_count() as u64;
        signature = signature.wrapping_mul(31).wrapping_add(self.timeline.tracks.len() as u64);
        signature = signature
            .wrapping_mul(31)
            .wrapping_add((self.timeline.duration * 10.0) as u64);
        signature = signature
            .wrapping_mul(31)
            .wrapping_add((self.timeline.playhead * 10.0) as u64);
        signature = signature
            .wrapping_mul(31)
            .wrapping_add(self.pool.assets.len() as u64);
        signature
            .wrapping_mul(31)
            .wrapping_add(self.selection.and_then(|s| s.clip_id).unwrap_or(0))
    }
}

pub struct PromptBuilder {
    persona: String,
    commands: &'static [CommandSpec],
    include_legend: bool,
}

impl Default for PromptBuilder {
    fn default() -> Self {
        Self {
            persona: DEFAULT_PERSONA.to_string(),
            commands: ACTION_COMMANDS,
            include_legend: true,
        }
    }
}

impl PromptBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_persona(mut self, persona: impl Into<String>) -> Self {
        self.persona = persona.into();
        self
    }

    /// Restricts the instruction set, e.g. for a read-only conversation.
    pub fn with_commands(mut self, commands: &'static [CommandSpec]) -> Self {
        self.commands = commands;
        self
    }

    /// Assembles the system prompt. Sections are ordered persona → state →
    /// tools → rules: the model reads the rules last, closest to its answer.
    pub fn build(&self, context: &PromptContext<'_>) -> String {
        let mut prompt = String::with_capacity(1024);

        prompt.push_str(&self.persona);
        prompt.push_str("\n\n");

        if self.include_legend {
            prompt.push_str(STATE_LEGEND);
            prompt.push_str("\n\n");
        }

        prompt.push_str("STATE:\n");
        prompt.push_str(&context.timeline.to_llm_context_string());
        prompt.push_str(&context.pool.to_llm_context_string());
        if let Some(selection) = context.selection {
            prompt.push_str(&selection.to_llm_context_string());
        }

        if !self.commands.is_empty() {
            prompt.push_str("\nACTIONS:\n");
            for command in self.commands {
                prompt.push_str(&format!(
                    "{}({}) {}\n",
                    command.name, command.signature, command.summary
                ));
            }
        }

        prompt.push('\n');
        prompt.push_str(OUTPUT_RULES);
        prompt
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_tooling::orchestration::context_models::{
        AssetContext, ClipContext, TrackContext, TrackKind,
    };

    fn timeline() -> TimelineContext {
        TimelineContext {
            duration: 10.0,
            playhead: 2.0,
            tracks: vec![TrackContext {
                index: 0,
                name: "V1".into(),
                kind: TrackKind::Video,
                muted: false,
                locked: false,
                clips: vec![ClipContext {
                    id: 4,
                    name: "intro.mp4".into(),
                    start: 0.0,
                    end: 6.0,
                    has_audio: true,
                }],
            }],
        }
    }

    fn pool() -> MediaPoolContext {
        MediaPoolContext {
            assets: vec![AssetContext {
                name: "vo.wav".into(),
                seconds: 8.0,
                kind: TrackKind::Audio,
                generated: false,
            }],
        }
    }

    #[test]
    fn the_prompt_carries_persona_state_tools_and_rules_in_order() {
        let (timeline, pool) = (timeline(), pool());
        let prompt = PromptBuilder::new().build(&PromptContext::new(&timeline, &pool));

        let state = prompt.find("STATE:").expect("state section");
        let actions = prompt.find("ACTIONS:").expect("actions section");
        let rules = prompt.find("RULES:").expect("rules section");

        assert!(prompt.starts_with(DEFAULT_PERSONA));
        assert!(state < actions && actions < rules, "rules read last");
        assert!(prompt.contains("c4 \"intro.mp4\" 0.0-6.0 a"));
        assert!(prompt.contains("\"vo.wav\" 8.0s audio"));
        assert!(prompt.contains("SPLIT(clip,at)"));
    }

    #[test]
    fn a_selection_is_injected_only_when_present() {
        let (timeline, pool) = (timeline(), pool());
        let without = PromptBuilder::new().build(&PromptContext::new(&timeline, &pool));
        assert!(!without.contains("selection:"));

        let selection = SelectionContext {
            kind: "clip".into(),
            description: "\"intro.mp4\" on V1".into(),
            clip_id: Some(4),
        };
        let with = PromptBuilder::new()
            .build(&PromptContext::new(&timeline, &pool).with_selection(Some(&selection)));
        assert!(with.contains("selection: clip c4"));
    }

    #[test]
    fn the_persona_and_instruction_set_are_replaceable() {
        let (timeline, pool) = (timeline(), pool());
        const READ_ONLY: &[CommandSpec] = &[];

        let prompt = PromptBuilder::new()
            .with_persona("You are a colourist.")
            .with_commands(READ_ONLY)
            .build(&PromptContext::new(&timeline, &pool));

        assert!(prompt.starts_with("You are a colourist."));
        assert!(!prompt.contains("ACTIONS:"), "no actions, none offered");
        assert!(prompt.contains("RULES:"));
    }

    #[test]
    fn the_whole_prompt_stays_small_for_a_typical_project() {
        let mut timeline = timeline();
        timeline.tracks[0].clips = (0..40)
            .map(|i| ClipContext {
                id: i,
                name: format!("clip_{i}.mp4"),
                start: i as f32,
                end: i as f32 + 1.0,
                has_audio: false,
            })
            .collect();

        let prompt = PromptBuilder::new().build(&PromptContext::new(&timeline, &pool()));
        // ~4 chars per token: a 40-clip timeline must not cost thousands.
        assert!(prompt.len() / 4 < 900, "prompt was {} chars", prompt.len());
    }

    #[test]
    fn the_signature_tracks_what_the_model_would_see() {
        let (timeline, pool) = (timeline(), pool());
        let base = PromptContext::new(&timeline, &pool);
        let unchanged = PromptContext::new(&timeline, &pool);
        assert_eq!(base.signature(), unchanged.signature());

        let mut moved = timeline.clone();
        moved.playhead = 5.0;
        assert_ne!(base.signature(), PromptContext::new(&moved, &pool).signature());

        let selection = SelectionContext {
            kind: "clip".into(),
            description: "x".into(),
            clip_id: Some(4),
        };
        assert_ne!(
            base.signature(),
            PromptContext::new(&timeline, &pool)
                .with_selection(Some(&selection))
                .signature()
        );
    }
}
