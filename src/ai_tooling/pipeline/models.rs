//! The contracts the three agents are held to.
//!
//! Every type here is both what the model must return and, through
//! [`strict_schema_for`](super::schema::strict_schema_for), the schema it is
//! constrained by. Doc comments are not decoration: `schemars` copies them into
//! `description` fields, where they steer the model at no prompt-token cost.

use crate::ai_tooling::revision::models::{Evidence, RevisionAction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// ------------------------------------------------------- agent 1 — the DNA

/// What makes a competitor video work, as the model reads it.
///
/// This is interpretation, not measurement. The numbers in
/// [`CompetitorVideo`](crate::ai_tooling::competitor::CompetitorVideo) say what
/// happened; the DNA says *why it held the audience*, in terms an edit can copy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CompetitorDNA {
    /// The video this describes.
    pub video_id: String,
    /// One paragraph: why this video kept its audience.
    pub verdict: String,
    pub hook: HookDna,
    pub pacing: PacingDna,
    pub audio: AudioDna,
    pub visual: VisualDna,
    /// Each retention movement, explained.
    pub retention_notes: Vec<RetentionNote>,
    /// Rules that transfer to a different video on a different topic. These are
    /// what the director agent actually works from.
    pub transferable_rules: Vec<String>,
}

/// How the opening earns the next thirty seconds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct HookDna {
    /// cold_open, question, provocation, promise or story.
    pub hook_type: String,
    /// The promise made to the viewer, in one sentence.
    pub promise: String,
    /// Seconds before the first substantive claim.
    pub time_to_value_sec: f32,
    /// Why this opening works, or where it is weak.
    pub assessment: String,
}

/// Cutting rhythm, as a rule rather than a measurement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PacingDna {
    /// Cuts per minute the edit sustains.
    pub cuts_per_minute: f32,
    /// Longest silence the edit tolerates before it becomes dead air.
    pub max_silence_sec: f32,
    /// Speaking rate across the body of the video.
    pub words_per_minute: f32,
    /// Whether the opening is cut faster than the body.
    pub front_loaded: bool,
}

/// The mix, described so it can be reproduced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AudioDna {
    /// Transition effects per minute.
    pub sfx_per_minute: f32,
    /// Effect names the edit leans on, most used first.
    pub signature_effects: Vec<String>,
    /// Whether loudness is used to punctuate, and how.
    pub dynamics_note: String,
}

/// Coverage and how it is used.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct VisualDna {
    /// Share of the runtime under a cutaway, 0.0 to 1.0.
    pub broll_coverage: f32,
    /// Longest the edit holds one shot.
    pub longest_static_hold_sec: f32,
    /// Recurring cutaway subjects.
    pub recurring_topics: Vec<String>,
    /// hard_cut, next_video_tease, loop_back or outro.
    pub ending_style: String,
}

/// One movement in the retention curve, with a cause an editor can act on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RetentionNote {
    pub start_sec: f32,
    pub end_sec: f32,
    /// True for a replay peak, false for a drop.
    pub is_peak: bool,
    /// What was on screen and in the mix across this span.
    pub what_happens: String,
    /// Why the audience reacted this way.
    pub cause: String,
}

// --------------------------------------------------- agent 2 — the revisions

/// One proposed change, as the model writes it.
///
/// Deliberately not [`RevisionTask`](crate::ai_tooling::revision::RevisionTask):
/// ids, status and the generation payload are ours to assign, and a model that
/// can invent them is a model that can contradict them. It supplies the
/// judgement — what to change, where, why, how much it matters — and nothing
/// else.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RevisionDraft {
    pub action: RevisionAction,
    /// Why this helps, in the editor's terms, citing the reference.
    pub rationale: String,
    pub evidence: Evidence,
    /// Estimated retention benefit, 0.0 to 1.0.
    pub impact: f32,
}

/// Agent 2's return value.
///
/// A bare array cannot be the root of a structured-output schema, so the list
/// is wrapped. The wrapper earns its keep: `overall_assessment` is where the
/// model says what it saw, which would otherwise have nowhere to go.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RevisionDraftList {
    /// Headline differences between the reference and the current edit.
    pub overall_assessment: Vec<String>,
    /// The changes, most valuable first.
    pub tasks: Vec<RevisionDraft>,
}

// ----------------------------------------------- agent 3 — the asset prompt

// NOTE — deliberately a plain comment, not a doc comment.
//
// `schemars` copies `///` text into the schema's `description`, and the schema
// is sent to the model on every call. Documenting the stealth mechanism in a
// doc comment would hand the model the very rule it is not supposed to know
// about. Anything below this line stays out of the wire format; the `///`
// comments on the fields are written for the model to read.
//
// The model reports whether the shot contains the presenter; the constraint is
// appended afterwards by `agents::prompt_engineer::compose`. A rule the model
// could paraphrase is a rule that can be paraphrased away.

/// A shot description for an image or video generation model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AssetPromptDraft {
    /// The detailed prompt: subject, framing, lens, lighting, motion, grade.
    pub prompt: String,
    /// Whether a person's face would be visible in the finished shot.
    pub involves_human_subject: bool,
    /// Things to keep out of frame, comma separated.
    pub avoid: String,
    /// One line on why this shot serves the beat it covers.
    pub intent: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_tooling::pipeline::schema::strict_schema_for;
    use serde_json::Value;

    #[test]
    fn the_dna_schema_covers_every_section_the_director_reads() {
        let schema = strict_schema_for::<CompetitorDNA>("dna").schema;
        let properties = schema["properties"].as_object().expect("properties");

        for section in ["hook", "pacing", "audio", "visual", "retention_notes"] {
            assert!(properties.contains_key(section), "{section} missing");
        }
    }

    #[test]
    fn the_revision_list_is_an_object_at_the_root_not_an_array() {
        let schema = strict_schema_for::<RevisionDraftList>("revisions").schema;

        assert_eq!(schema["type"], "object", "an array root is rejected");
        assert_eq!(schema["properties"]["tasks"]["type"], "array");
    }

    #[test]
    fn the_action_enum_reaches_the_schema_as_anyof_over_its_variants() {
        let schema = strict_schema_for::<RevisionDraftList>("revisions").schema;
        let json = schema.to_string();

        // Every variant tag the dispatcher understands must be offered.
        for tag in [
            "generate_and_insert_b_roll",
            "add_transition_audio",
            "fix_retention_drop",
            "revise_ending",
            "cut_silence",
        ] {
            assert!(json.contains(tag), "{tag} is not in the schema");
        }
        assert!(!json.contains("oneOf"));
    }

    #[test]
    fn a_draft_the_model_could_return_deserializes_into_our_types() {
        // Shaped exactly as the schema demands: every key present, tag included.
        let raw = r#"{
            "overall_assessment": ["cuts too slowly"],
            "tasks": [{
                "action": {
                    "kind": "generate_and_insert_b_roll",
                    "timestamp": 12.0,
                    "duration": 3.5,
                    "semantic_topic": "overhead desk",
                    "generation_prompt": "",
                    "track_index": 1
                },
                "rationale": "static shot over a replay peak",
                "evidence": {
                    "competitor_video_id": "ref1",
                    "competitor_time_sec": 24.0,
                    "observation": "cutaway at the peak"
                },
                "impact": 0.8
            }]
        }"#;

        let list: RevisionDraftList = serde_json::from_str(raw).expect("deserialize");
        assert_eq!(list.tasks.len(), 1);
        assert!(matches!(
            list.tasks[0].action,
            RevisionAction::GenerateAndInsertBRoll { track_index: 1, .. }
        ));
    }

    #[test]
    fn the_cut_silence_variant_round_trips_through_its_own_schema_shape() {
        let action = RevisionAction::CutSilence {
            clip_id: "c12".into(),
            start_time: 4.0,
            end_time: 5.2,
        };
        let json = serde_json::to_value(&action).expect("serialize");

        assert_eq!(json["kind"], "cut_silence");
        let back: RevisionAction = serde_json::from_value(json).expect("deserialize");
        assert_eq!(back, action);
    }

    #[test]
    fn the_asset_prompt_never_asks_the_model_about_the_constraint() {
        let schema = strict_schema_for::<AssetPromptDraft>("asset").schema;
        let json = schema.to_string().to_lowercase();

        assert!(
            !json.contains("facial") && !json.contains("landmark"),
            "the constraint must not be something the model can restate: {json}"
        );
        assert!(schema["properties"]["involves_human_subject"]["type"] == Value::from("boolean"));
    }
}
