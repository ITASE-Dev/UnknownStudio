//! Agent 3 — the Asset Prompt Engineer.
//!
//! Runs only for a `GenerateAndInsertBRoll` action. Turns a semantic topic
//! ("overhead desk shot") into a prompt a diffusion model can render.
//!
//! # The stealth constraint
//!
//! The model writing the prompt is never shown the facial identity rule and is
//! never asked to reproduce it. It answers one question about the face —
//! `involves_human_subject`, a boolean — and [`compose`] does the rest. A rule
//! the model can restate is a rule it can paraphrase, shorten, or drop under
//! instruction-following pressure; a boolean it cannot. The constraint is
//! appended in Rust, after the model has finished, by
//! [`GenerationRequest::new`].

use crate::ai_tooling::pipeline::models::AssetPromptDraft;
use crate::ai_tooling::pipeline::schema::{strict_schema_for, SchemaSpec};
use crate::ai_tooling::revision::generation::{GenerationRequest, IdentityMode};
use serde_json::{json, Value};

pub const SCHEMA_NAME: &str = "asset_prompt";

pub const SYSTEM_PROMPT: &str = "\
You write prompts for image and video generation models. You are given a shot to \
produce: its subject, how long it runs, and the moment in the edit it has to cover.

Return one prompt that a generator can render without further interpretation. Name the \
subject, the framing, the lens, the lighting, the camera motion and the colour grade. \
Write it as a description of the finished frame, not as an instruction.

Set involves_human_subject to true if a person's face would be visible in the shot, and \
false otherwise. Answer it accurately — downstream handling depends on it.

Rules:
- No on-screen text, captions, watermarks or logos.
- The shot must cut cleanly against a talking head; avoid anything that pulls focus.
- Keep it to a single paragraph.";

pub fn schema() -> SchemaSpec {
    strict_schema_for::<AssetPromptDraft>(SCHEMA_NAME)
}

/// What the shot has to do, as sent to the model.
///
/// Note what is absent: nothing about faces beyond the topic itself, and no
/// mention of a reference image. The model cannot leak a rule it was not told.
pub fn payload(semantic_topic: &str, duration_sec: f32, beat: &str) -> Value {
    json!({
        "shot_topic": semantic_topic,
        "duration_sec": (duration_sec * 10.0).round() / 10.0,
        "covers_this_beat": beat,
    })
}

/// Turns the model's draft into a validated request.
///
/// This is the enforcement point. The model's boolean chooses the identity
/// mode; the mode makes `GenerationRequest::new` append the constraint. The
/// request returned here is already validated, so a caller cannot forget to.
pub fn compose(
    draft: &AssetPromptDraft,
    semantic_topic: &str,
    duration_sec: f32,
    presenter_reference: Option<&str>,
) -> GenerationRequest {
    let identity = match (draft.involves_human_subject, presenter_reference) {
        // A face, and something to match it against: the constraint applies.
        (true, Some(reference)) if !reference.trim().is_empty() => IdentityMode::PresenterFace {
            reference_asset: reference.trim().to_string(),
        },
        // A face, but no reference. The constraint cannot be honoured without
        // one, so the shot is re-framed rather than left to invent a likeness.
        (true, _) => IdentityMode::NoPresenter,
        (false, _) => IdentityMode::NoPresenter,
    };

    let mut base = draft.prompt.trim().to_string();
    if !draft.avoid.trim().is_empty() {
        base.push_str(&format!("\n\nKeep out of frame: {}.", draft.avoid.trim()));
    }
    if draft.involves_human_subject && !matches!(identity, IdentityMode::PresenterFace { .. }) {
        base.push_str(
            "\n\nFrame from behind or over the shoulder — no face visible, as no presenter \
             reference is available.",
        );
    }

    GenerationRequest::new(semantic_topic, base, identity, duration_sec)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_tooling::revision::generation::FACIAL_IDENTITY_CONSTRAINT;

    fn draft(involves_human_subject: bool) -> AssetPromptDraft {
        AssetPromptDraft {
            prompt: "Medium shot, 50mm, warm key light, slow push in, teal grade.".into(),
            involves_human_subject,
            avoid: "logos, text".into(),
            intent: "covers the claim at 12s".into(),
        }
    }

    #[test]
    fn a_face_shot_with_a_reference_gets_the_constraint_verbatim() {
        let request = compose(&draft(true), "presenter reacting", 3.5, Some("me.png"));

        assert!(request.prompt().contains(FACIAL_IDENTITY_CONSTRAINT));
        assert!(matches!(
            request.identity(),
            IdentityMode::PresenterFace { .. }
        ));
        assert!(request.validate().is_ok());
    }

    #[test]
    fn the_constraint_text_is_the_one_that_was_specified() {
        // Guards the exact wording against a well-meaning edit.
        assert!(FACIAL_IDENTITY_CONSTRAINT.starts_with("Treat the subject’s facial identity as a hard constraint."));
        assert!(FACIAL_IDENTITY_CONSTRAINT.contains(
            "(eye shape, nose structure, jawline, and unique asymmetry)"
        ));
        assert!(FACIAL_IDENTITY_CONSTRAINT
            .ends_with("Do not beautify or blend these features with generic models."));
    }

    #[test]
    fn a_shot_with_no_person_is_not_burdened_with_it() {
        let request = compose(&draft(false), "overhead desk", 3.0, Some("me.png"));

        assert!(!request.prompt().contains(FACIAL_IDENTITY_CONSTRAINT));
        assert_eq!(*request.identity(), IdentityMode::NoPresenter);
    }

    #[test]
    fn a_face_shot_without_a_reference_is_reframed_not_invented() {
        let request = compose(&draft(true), "presenter reacting", 3.0, None);

        assert_eq!(*request.identity(), IdentityMode::NoPresenter);
        assert!(request.prompt().contains("no face visible"));
        assert!(request.validate().is_ok(), "still a legal request");
    }

    #[test]
    fn a_blank_reference_counts_as_no_reference() {
        let request = compose(&draft(true), "presenter", 3.0, Some("   "));
        assert_eq!(*request.identity(), IdentityMode::NoPresenter);
    }

    #[test]
    fn what_to_avoid_survives_into_the_prompt() {
        let request = compose(&draft(false), "desk", 3.0, None);
        assert!(request.prompt().contains("Keep out of frame: logos, text."));
    }

    #[test]
    fn the_model_is_never_shown_the_rule_it_must_not_paraphrase() {
        let sent = format!("{SYSTEM_PROMPT}{}", payload("a desk", 3.0, "the claim at 12s"));
        let lowered = sent.to_lowercase();

        assert!(!lowered.contains("facial identity"));
        assert!(!lowered.contains("landmark"));
        assert!(!lowered.contains("asymmetry"));
        assert!(!lowered.contains("reference"), "not even that one exists");
    }

    #[test]
    fn the_payload_says_what_the_shot_is_for() {
        let payload = payload("overhead desk", 3.55, "covers the claim");

        assert_eq!(payload["shot_topic"], "overhead desk");
        // f32 → JSON: compare with a tolerance rather than against a literal.
        let seconds = payload["duration_sec"].as_f64().expect("a number");
        assert!((seconds - 3.6).abs() < 0.01, "{seconds}");
        assert_eq!(payload["covers_this_beat"], "covers the claim");
    }
}
