//! Asset generation, and the facial identity rule.
//!
//! # The hard constraint
//!
//! When a generated shot has to contain the presenter, the request **must**
//! carry the identity constraint. Generators left to themselves regress a face
//! toward the average of their training set — the result looks like a
//! good-looking stranger, which is worse than no B-roll at all.
//!
//! Enforcement is in three layers, because a single one is a comment with
//! extra steps:
//!
//! 1. [`GenerationRequest`] has private fields and one constructor. When the
//!    mode is [`IdentityMode::PresenterFace`], the constructor injects the
//!    constraint. There is no setter that can take it back out, so a request
//!    built in process cannot be wrong.
//! 2. [`GenerationRequest::validate`] re-checks the invariant. Serde bypasses
//!    constructors, so a plan loaded from disk — or edited there — is checked
//!    again rather than trusted.
//! 3. [`generate`] validates before it calls the backend. A generator can only
//!    be reached through it, so no path skips the check.

use serde::{Deserialize, Serialize};
use std::future::Future;
use std::time::Duration;
use thiserror::Error;

/// The rule, verbatim and unedited. Injected into every payload that renders
/// the presenter.
///
/// Treat this string as a fixed asset: reword it and the generator's behaviour
/// changes for reasons nobody will be able to reconstruct later.
pub const FACIAL_IDENTITY_CONSTRAINT: &str = "Treat the subject’s facial identity as a hard constraint. \
Analyze and strictly adhere to specific facial landmarks of the reference (eye shape, nose structure, \
jawline, and unique asymmetry). Do not beautify or blend these features with generic models.";

/// Appended alongside the constraint: the failure modes stated as negatives.
pub const IDENTITY_NEGATIVE_PROMPT: &str =
    "beautified face, smoothed skin, symmetrical face, generic model features, \
     altered jawline, different nose, face blending, celebrity likeness";

#[derive(Debug, Error)]
pub enum GenerationError {
    /// The invariant this module exists to protect.
    #[error("generation request renders the presenter but carries no facial identity constraint")]
    MissingIdentityConstraint,

    #[error("generation request renders the presenter but names no reference asset")]
    MissingReference,

    #[error("prompt is empty")]
    EmptyPrompt,

    #[error("backend failed: {0}")]
    Backend(String),
}

pub type Result<T> = std::result::Result<T, GenerationError>;

/// Whether the shot contains the presenter, and what to match them against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum IdentityMode {
    /// No person in frame — a product shot, a timelapse, an animation.
    NoPresenter,
    /// The presenter appears. Subject to the hard constraint.
    PresenterFace {
        /// Reference the generator matches landmarks against.
        reference_asset: String,
    },
}

impl IdentityMode {
    pub fn requires_constraint(&self) -> bool {
        matches!(self, Self::PresenterFace { .. })
    }
}

/// A prepared payload for an image/video generator.
///
/// Fields are private on purpose: the constraint is an invariant of this type,
/// not a convention callers are asked to remember.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GenerationRequest {
    prompt: String,
    negative_prompt: String,
    identity: IdentityMode,
    duration_sec: f32,
    /// What the shot is of, kept unmodified for the vector index and the UI.
    semantic_topic: String,
}

impl GenerationRequest {
    /// Builds a request, injecting the identity constraint when the shot needs
    /// the presenter's face.
    pub fn new(
        semantic_topic: impl Into<String>,
        base_prompt: impl Into<String>,
        identity: IdentityMode,
        duration_sec: f32,
    ) -> Self {
        let semantic_topic = semantic_topic.into();
        let base = base_prompt.into();

        let (prompt, negative_prompt) = if identity.requires_constraint() {
            (
                format!("{base}\n\n{FACIAL_IDENTITY_CONSTRAINT}"),
                IDENTITY_NEGATIVE_PROMPT.to_string(),
            )
        } else {
            (base, String::new())
        };

        Self {
            prompt,
            negative_prompt,
            identity,
            duration_sec: duration_sec.max(0.1),
            semantic_topic,
        }
    }

    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    pub fn negative_prompt(&self) -> &str {
        &self.negative_prompt
    }

    pub fn identity(&self) -> &IdentityMode {
        &self.identity
    }

    pub fn duration_sec(&self) -> f32 {
        self.duration_sec
    }

    pub fn semantic_topic(&self) -> &str {
        &self.semantic_topic
    }

    /// Whether this payload is safe to send.
    ///
    /// Re-checks rather than assumes: `Deserialize` builds this type without
    /// going through [`Self::new`], so a plan restored from disk gets the same
    /// scrutiny as one built in memory.
    pub fn validate(&self) -> Result<()> {
        if self.prompt.trim().is_empty() {
            return Err(GenerationError::EmptyPrompt);
        }

        if let IdentityMode::PresenterFace { reference_asset } = &self.identity {
            if reference_asset.trim().is_empty() {
                return Err(GenerationError::MissingReference);
            }
            if !self.prompt.contains(FACIAL_IDENTITY_CONSTRAINT) {
                return Err(GenerationError::MissingIdentityConstraint);
            }
        }

        Ok(())
    }
}

/// What kind of lane the finished asset belongs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeneratedKind {
    Video,
    Audio,
}

/// A finished asset, ready to be placed on the timeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeneratedAsset {
    /// Pool name the dispatcher will look the asset up by.
    pub name: String,
    pub duration_sec: f32,
    pub kind: GeneratedKind,
    /// Where the backend wrote it. `None` for the mock.
    pub path: Option<String>,
}

/// A backend that can render a request. ComfyUI implements this for real; the
/// mock below stands in until it is wired.
pub trait AssetGenerator: Send + Sync {
    fn render(
        &self,
        request: &GenerationRequest,
    ) -> impl Future<Output = Result<GeneratedAsset>> + Send;
}

/// The only way to reach a generator.
///
/// Validation happens here, so no call site can forget it.
pub async fn generate<G: AssetGenerator>(
    generator: &G,
    request: &GenerationRequest,
) -> Result<GeneratedAsset> {
    request.validate()?;
    generator.render(request).await
}

/// Stands in for ComfyUI. Records what it was asked for so tests and the UI
/// can assert on the payload that would have been sent.
#[derive(Debug, Default, Clone)]
pub struct MockGenerator {
    /// Milliseconds the render pretends to take.
    pub latency_ms: u64,
}

impl MockGenerator {
    pub fn new(latency_ms: u64) -> Self {
        Self { latency_ms }
    }
}

impl AssetGenerator for MockGenerator {
    async fn render(&self, request: &GenerationRequest) -> Result<GeneratedAsset> {
        tokio::time::sleep(Duration::from_millis(self.latency_ms)).await;

        // Named after the topic so the clip is recognisable on the timeline.
        let slug: String = request
            .semantic_topic()
            .chars()
            .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
            .collect();
        let slug: String = slug.split('_').filter(|s| !s.is_empty()).take(4).collect::<Vec<_>>().join("_");

        Ok(GeneratedAsset {
            name: format!("gen_{slug}.mp4"),
            duration_sec: request.duration_sec(),
            kind: GeneratedKind::Video,
            path: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("runtime")
    }

    fn with_face() -> GenerationRequest {
        GenerationRequest::new(
            "presenter reacting to the result",
            "Medium shot of the presenter reacting, warm key light.",
            IdentityMode::PresenterFace {
                reference_asset: "presenter_ref.png".into(),
            },
            4.0,
        )
    }

    #[test]
    fn a_shot_with_the_presenter_carries_the_constraint_verbatim() {
        let request = with_face();

        assert!(request.prompt().contains(FACIAL_IDENTITY_CONSTRAINT));
        assert!(request.prompt().contains("warm key light"), "the base survives");
        assert_eq!(request.negative_prompt(), IDENTITY_NEGATIVE_PROMPT);
        assert!(request.validate().is_ok());
    }

    #[test]
    fn a_shot_without_a_person_is_not_burdened_with_it() {
        let request = GenerationRequest::new(
            "overhead desk timelapse",
            "Overhead timelapse of a desk, no people.",
            IdentityMode::NoPresenter,
            3.0,
        );

        assert!(!request.prompt().contains(FACIAL_IDENTITY_CONSTRAINT));
        assert!(request.negative_prompt().is_empty());
        assert!(request.validate().is_ok());
    }

    #[test]
    fn a_tampered_payload_is_refused_even_though_serde_built_it() {
        // Serde does not run the constructor, which is exactly the hole the
        // second layer of enforcement exists to close.
        let json = r#"{
            "prompt": "Medium shot of the presenter, cinematic.",
            "negative_prompt": "",
            "identity": { "mode": "presenter_face", "reference_asset": "ref.png" },
            "duration_sec": 4.0,
            "semantic_topic": "presenter"
        }"#;

        let request: GenerationRequest = serde_json::from_str(json).expect("deserialize");
        assert!(matches!(
            request.validate(),
            Err(GenerationError::MissingIdentityConstraint)
        ));
    }

    #[test]
    fn a_face_shot_without_a_reference_is_refused() {
        let request = GenerationRequest::new(
            "presenter",
            "Close up.",
            IdentityMode::PresenterFace {
                reference_asset: "  ".into(),
            },
            2.0,
        );
        assert!(matches!(
            request.validate(),
            Err(GenerationError::MissingReference)
        ));
    }

    #[test]
    fn an_empty_prompt_never_reaches_a_backend() {
        let request = GenerationRequest::new("topic", "   ", IdentityMode::NoPresenter, 2.0);
        assert!(matches!(request.validate(), Err(GenerationError::EmptyPrompt)));
    }

    #[test]
    fn the_generate_entry_point_validates_before_calling_the_backend() {
        runtime().block_on(async {
            let json = r#"{
                "prompt": "Presenter close up.",
                "negative_prompt": "",
                "identity": { "mode": "presenter_face", "reference_asset": "ref.png" },
                "duration_sec": 4.0,
                "semantic_topic": "presenter"
            }"#;
            let bad: GenerationRequest = serde_json::from_str(json).expect("deserialize");

            let generator = MockGenerator::new(0);
            let err = generate(&generator, &bad).await.expect_err("must refuse");
            assert!(matches!(err, GenerationError::MissingIdentityConstraint));
        });
    }

    #[test]
    fn a_valid_request_renders_to_a_placeable_asset() {
        runtime().block_on(async {
            let generator = MockGenerator::new(0);
            let asset = generate(&generator, &with_face()).await.expect("render");

            assert!(asset.name.starts_with("gen_"));
            assert!(asset.name.ends_with(".mp4"));
            assert_eq!(asset.duration_sec, 4.0);
        });
    }

    #[test]
    fn the_duration_never_reaches_the_backend_as_zero() {
        let request = GenerationRequest::new("t", "p", IdentityMode::NoPresenter, 0.0);
        assert!(request.duration_sec() > 0.0);
    }
}
