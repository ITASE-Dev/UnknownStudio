//! The three-stage agent pipeline.
//!
//! ```text
//!   CompetitorVideo ─▶ [1 Deconstructor] ─▶ CompetitorDNA ─┐
//!                                                          ├─▶ [2 Director] ─▶ RevisionDraftList
//!   CurrentTimelineState ──────────────────────────────────┘                          │
//!                                                                                     ▼
//!                                            [3 Prompt Engineer] ◀── one per GenerateAndInsertBRoll
//!                                                     │
//!                                                     ▼
//!                                             GenerationRequest (constraint injected)
//!                                                     │
//!                                                     ▼
//!                                                RevisionPlan
//! ```
//!
//! Every hop is a strict Structured Output call: the schema is derived from the
//! Rust type the answer must deserialize into, so a response that parses is a
//! response that fits. There is no repair step, no markdown fence to strip and
//! no "if the model returns prose" branch, because those cases cannot arrive.
//!
//! The engine reuses the existing [`LlmClient`] rather than replacing it, so
//! the blueprint path and the chat assistant keep working unchanged.

pub mod agents;
pub mod models;
pub mod schema;

use crate::ai_tooling::competitor::models::CompetitorVideo;
use crate::ai_tooling::config::AiToolingConfig;
use crate::ai_tooling::providers::{Completion, LlmClient, RetryPolicy};
use crate::ai_tooling::revision::generation::GenerationRequest;
use crate::ai_tooling::revision::models::{
    RevisionAction, RevisionPlan, RevisionTask, TaskStatus,
};
#[cfg(test)]
use crate::ai_tooling::revision::models::Evidence;
use crate::ai_tooling::revision::timeline::CurrentTimelineState;
use crate::ai_tooling::AiToolingError;
pub use models::{AssetPromptDraft, CompetitorDNA, RevisionDraft, RevisionDraftList};
pub use schema::{strict_schema_for, SchemaSpec as StrictSchema};
use reqwest::Client;
use schema::SchemaSpec;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::time::Duration;
use thiserror::Error;

/// Long enough for a reasoning model on a large payload, short enough that a
/// hung request does not strand the UI's progress indicator.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Debug, Error)]
pub enum PipelineError {
    #[error("{stage}: {source}")]
    Provider {
        stage: &'static str,
        #[source]
        source: AiToolingError,
    },

    /// The model declined. Distinct from a transport failure: retrying an
    /// unchanged prompt will be declined again.
    #[error("{stage}: the model refused to answer")]
    Refused { stage: &'static str },

    /// Should be impossible under a strict schema. Kept because "impossible"
    /// depends on the provider honouring the contract, and a silent `unwrap`
    /// here would be a panic in a background task.
    #[error("{stage}: response did not match its schema: {source}")]
    Malformed {
        stage: &'static str,
        #[source]
        source: serde_json::Error,
    },

    #[error("configuration: {0}")]
    Config(#[source] AiToolingError),
}

pub type Result<T> = std::result::Result<T, PipelineError>;

/// Everything the pipeline produced, including the intermediate stage.
///
/// The DNA is returned rather than discarded: it is what the user reads to
/// understand *why* the plan says what it says, and re-deriving it costs
/// another call.
#[derive(Debug, Clone)]
pub struct PipelineOutput {
    pub dna: CompetitorDNA,
    pub plan: RevisionPlan,
    /// Stage 3 ran this many times — one per B-roll task.
    pub prompts_written: usize,
}

pub struct LlmPipelineEngine {
    client: LlmClient,
    retry: RetryPolicy,
    /// The user's face reference, if they configured one.
    presenter_reference: Option<String>,
}

impl LlmPipelineEngine {
    /// Builds from the same config the rest of the app uses.
    pub fn from_config(config: &AiToolingConfig) -> Result<Self> {
        let http = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|err| PipelineError::Config(AiToolingError::Http(err)))?;

        Ok(Self {
            client: LlmClient::from_config(config, http).map_err(PipelineError::Config)?,
            retry: RetryPolicy::default(),
            presenter_reference: None,
        })
    }

    /// Wraps an already-built client — the existing connection, adapted rather
    /// than replaced.
    pub fn with_client(client: LlmClient) -> Self {
        Self {
            client,
            retry: RetryPolicy::default(),
            presenter_reference: None,
        }
    }

    pub fn with_retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    pub fn with_presenter_reference(mut self, reference: Option<String>) -> Self {
        self.presenter_reference = reference.filter(|r| !r.trim().is_empty());
        self
    }

    pub fn model_id(&self) -> &str {
        self.client.model_id()
    }

    // ------------------------------------------------------------- stage 1

    /// Agent 1. Measured data in, judgement out.
    pub async fn deconstruct(&self, video: &CompetitorVideo) -> Result<CompetitorDNA> {
        self.call(
            "deconstructor",
            agents::deconstructor::SYSTEM_PROMPT,
            &agents::deconstructor::payload(video),
            &agents::deconstructor::schema(),
        )
        .await
    }

    // ------------------------------------------------------------- stage 2

    /// Agent 2. The DNA and the live timeline in, drafts out.
    pub async fn direct(
        &self,
        dna: &CompetitorDNA,
        current: &CurrentTimelineState,
    ) -> Result<RevisionDraftList> {
        self.call(
            "director",
            agents::director::SYSTEM_PROMPT,
            &agents::director::payload(dna, current),
            &agents::director::schema(),
        )
        .await
    }

    // ------------------------------------------------------------- stage 3

    /// Agent 3. One shot topic in, a validated generation request out.
    ///
    /// The facial constraint is applied here, after the model has answered —
    /// see [`agents::prompt_engineer::compose`].
    pub async fn write_asset_prompt(
        &self,
        semantic_topic: &str,
        duration_sec: f32,
        beat: &str,
    ) -> Result<GenerationRequest> {
        let draft: AssetPromptDraft = self
            .call(
                "prompt_engineer",
                agents::prompt_engineer::SYSTEM_PROMPT,
                &agents::prompt_engineer::payload(semantic_topic, duration_sec, beat),
                &agents::prompt_engineer::schema(),
            )
            .await?;

        Ok(agents::prompt_engineer::compose(
            &draft,
            semantic_topic,
            duration_sec,
            self.presenter_reference.as_deref(),
        ))
    }

    // -------------------------------------------------------- the full run

    /// Runs all three stages.
    ///
    /// Stage 2 cannot start before stage 1 finishes — it consumes its output —
    /// so those are sequential by necessity.
    ///
    /// `report` is called at each stage boundary. It exists because the
    /// orchestrator previously reimplemented this chain purely to emit
    /// progress between the stages, which left two copies of the same
    /// sequence free to drift apart. Callers that do not care pass
    /// [`no_progress`].
    pub async fn run(
        &self,
        video: &CompetitorVideo,
        current: &CurrentTimelineState,
        report: &(dyn Fn(Stage) + Sync),
    ) -> Result<PipelineOutput> {
        report(Stage::Deconstructing);
        let dna = self.deconstruct(video).await?;

        report(Stage::Directing(dna.clone()));
        let drafts = self.direct(&dna, current).await?;

        let mut tasks = Vec::with_capacity(drafts.tasks.len());
        let mut prompts_written = 0;

        for (index, draft) in drafts.tasks.iter().enumerate() {
            // Ids are ours: a model that can choose them can collide them.
            let id = index as u64 + 1;
            let generation = match &draft.action {
                RevisionAction::GenerateAndInsertBRoll {
                    semantic_topic,
                    duration,
                    ..
                } => {
                    prompts_written += 1;
                    report(Stage::WritingPrompt {
                        index,
                        total: drafts.tasks.len(),
                        topic: semantic_topic.clone(),
                    });
                    Some(
                        self.write_asset_prompt(semantic_topic, *duration, &draft.rationale)
                            .await?,
                    )
                }
                _ => None,
            };

            tasks.push(draft_into_task(id, draft, generation));
        }

        let mut plan = RevisionPlan {
            competitor_video_id: video.video_id.clone(),
            competitor_title: video.title.clone(),
            summary: drafts.overall_assessment,
            tasks,
        };
        plan.sort_by_impact();

        Ok(PipelineOutput {
            dna,
            plan,
            prompts_written,
        })
    }

    /// One structured call, decoded into `T`.
    async fn call<T: DeserializeOwned>(
        &self,
        stage: &'static str,
        system_prompt: &str,
        payload: &Value,
        spec: &SchemaSpec,
    ) -> Result<T> {
        let completion = self
            .client
            .complete_spec(system_prompt, payload, spec, self.retry)
            .await
            .map_err(|source| PipelineError::Provider { stage, source })?;

        let Completion::Json(text) = completion else {
            return Err(PipelineError::Refused { stage });
        };

        serde_json::from_str(&text).map_err(|source| PipelineError::Malformed { stage, source })
    }
}

/// Where the pipeline has got to, for a caller that wants to show progress.
#[derive(Debug, Clone)]
pub enum Stage {
    Deconstructing,
    /// Stage 1 is done; its output comes along so the caller can show it
    /// without waiting for the whole run.
    Directing(CompetitorDNA),
    WritingPrompt {
        index: usize,
        total: usize,
        topic: String,
    },
}

impl Stage {
    /// Rough completion, for a progress bar.
    pub fn fraction(&self) -> f32 {
        match self {
            Self::Deconstructing => 0.25,
            Self::Directing(_) => 0.55,
            Self::WritingPrompt { index, total, .. } => {
                0.6 + 0.3 * (*index as f32 / (*total).max(1) as f32)
            }
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Deconstructing => "agent 1 — deconstructing the reference".into(),
            Self::Directing(_) => "agent 2 — finding the gaps".into(),
            Self::WritingPrompt { index, total, topic } => {
                format!("agent 3 — prompt {} of {total}: {topic}", index + 1)
            }
        }
    }
}

/// A reporter for callers with nothing to report to.
pub fn no_progress(_: Stage) {}

/// Draft → task. The model supplies judgement; everything else is assigned.
///
/// Public so an orchestrator that drives the three stages itself — to report
/// progress between them — can still assemble the plan the same way `run` does.
pub fn draft_into_task(
    id: u64,
    draft: &RevisionDraft,
    generation: Option<GenerationRequest>,
) -> RevisionTask {
    // A model asked for a number in 0..=1 will occasionally return 1.4.
    let impact = draft.impact.clamp(0.0, 1.0);

    let mut action = draft.action.clone();
    // Keep the stored prompt and the payload in step, so the panel shows what
    // will actually be rendered rather than the model's first sketch.
    if let (
        RevisionAction::GenerateAndInsertBRoll {
            generation_prompt, ..
        },
        Some(request),
    ) = (&mut action, generation.as_ref())
    {
        *generation_prompt = request.prompt().to_string();
    }

    RevisionTask {
        id,
        action,
        rationale: draft.rationale.clone(),
        evidence: draft.evidence.clone(),
        impact,
        status: TaskStatus::Proposed,
        generation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_tooling::revision::generation::{IdentityMode, FACIAL_IDENTITY_CONSTRAINT};

    fn draft(action: RevisionAction, impact: f32) -> RevisionDraft {
        RevisionDraft {
            action,
            rationale: "because".into(),
            evidence: Evidence {
                competitor_video_id: "ref1".into(),
                competitor_time_sec: 24.0,
                observation: "cutaway at the peak".into(),
            },
            impact,
        }
    }

    #[test]
    fn ids_are_assigned_by_us_not_taken_from_the_model() {
        let drafts = [
            draft(
                RevisionAction::FixRetentionDrop {
                    timestamp: 1.0,
                    suggestion: "tighten".into(),
                },
                0.5,
            ),
            draft(
                RevisionAction::FixRetentionDrop {
                    timestamp: 2.0,
                    suggestion: "tighten".into(),
                },
                0.5,
            ),
        ];

        let tasks: Vec<RevisionTask> = drafts
            .iter()
            .enumerate()
            .map(|(i, d)| draft_into_task(i as u64 + 1, d, None))
            .collect();

        assert_eq!(tasks[0].id, 1);
        assert_eq!(tasks[1].id, 2);
        assert_ne!(tasks[0].id, tasks[1].id, "ids cannot collide");
    }

    #[test]
    fn an_out_of_range_impact_is_clamped_rather_than_trusted() {
        let task = draft_into_task(
            1,
            &draft(
                RevisionAction::FixRetentionDrop {
                    timestamp: 0.0,
                    suggestion: String::new(),
                },
                1.4,
            ),
            None,
        );
        assert_eq!(task.impact, 1.0);

        let task = draft_into_task(
            1,
            &draft(
                RevisionAction::FixRetentionDrop {
                    timestamp: 0.0,
                    suggestion: String::new(),
                },
                -3.0,
            ),
            None,
        );
        assert_eq!(task.impact, 0.0);
    }

    #[test]
    fn every_new_task_starts_proposed_so_nothing_self_approves() {
        let task = draft_into_task(
            1,
            &draft(
                RevisionAction::CutSilence {
                    clip_id: "c1".into(),
                    start_time: 1.0,
                    end_time: 2.0,
                },
                0.5,
            ),
            None,
        );
        assert_eq!(task.status, TaskStatus::Proposed);
    }

    #[test]
    fn the_stored_prompt_is_replaced_by_the_one_that_will_be_rendered() {
        let request = GenerationRequest::new(
            "presenter reacting",
            "Medium shot.",
            IdentityMode::PresenterFace {
                reference_asset: "me.png".into(),
            },
            3.0,
        );

        let task = draft_into_task(
            1,
            &draft(
                RevisionAction::GenerateAndInsertBRoll {
                    timestamp: 12.0,
                    duration: 3.0,
                    semantic_topic: "presenter reacting".into(),
                    // What Agent 2 sketched, before Agent 3 ran.
                    generation_prompt: "a rough idea".into(),
                    track_index: 1,
                },
                0.8,
            ),
            Some(request),
        );

        match &task.action {
            RevisionAction::GenerateAndInsertBRoll { generation_prompt, .. } => {
                assert!(
                    generation_prompt.contains(FACIAL_IDENTITY_CONSTRAINT),
                    "the panel must show the prompt that will actually be sent"
                );
                assert_ne!(generation_prompt, "a rough idea");
            }
            other => panic!("wrong action: {other:?}"),
        }
    }

    #[test]
    fn a_non_broll_task_carries_no_generation_payload() {
        let task = draft_into_task(
            1,
            &draft(
                RevisionAction::AddTransitionAudio {
                    timestamp: 5.0,
                    sfx_type: "whoosh".into(),
                },
                0.4,
            ),
            None,
        );
        assert!(task.generation.is_none());
    }

    #[test]
    fn each_stage_names_itself_in_its_errors() {
        let refused = PipelineError::Refused { stage: "director" };
        assert!(refused.to_string().contains("director"));

        let malformed = PipelineError::Malformed {
            stage: "deconstructor",
            source: serde_json::from_str::<CompetitorDNA>("{}").expect_err("invalid"),
        };
        assert!(malformed.to_string().contains("deconstructor"));
    }

    #[test]
    fn stage_progress_advances_monotonically_through_the_run() {
        let dna = |video_id: &str| CompetitorDNA {
            video_id: video_id.into(),
            verdict: String::new(),
            hook: models::HookDna {
                hook_type: String::new(),
                promise: String::new(),
                time_to_value_sec: 0.0,
                assessment: String::new(),
            },
            pacing: models::PacingDna {
                cuts_per_minute: 0.0,
                max_silence_sec: 0.0,
                words_per_minute: 0.0,
                front_loaded: false,
            },
            audio: models::AudioDna {
                sfx_per_minute: 0.0,
                signature_effects: Vec::new(),
                dynamics_note: String::new(),
            },
            visual: models::VisualDna {
                broll_coverage: 0.0,
                longest_static_hold_sec: 0.0,
                recurring_topics: Vec::new(),
                ending_style: String::new(),
            },
            retention_notes: Vec::new(),
            transferable_rules: Vec::new(),
        };

        let stages = [
            Stage::Deconstructing,
            Stage::Directing(dna("v")),
            Stage::WritingPrompt { index: 0, total: 2, topic: "a".into() },
            Stage::WritingPrompt { index: 1, total: 2, topic: "b".into() },
        ];

        for pair in stages.windows(2) {
            assert!(
                pair[0].fraction() <= pair[1].fraction(),
                "a bar that goes backwards reads as a bug"
            );
        }
        assert!(stages.last().expect("last").fraction() < 1.0, "run() finishes it");
        assert!(stages.iter().all(|s| !s.label().is_empty()));
    }

    #[test]
    fn a_prompt_stage_with_no_tasks_does_not_divide_by_zero() {
        let stage = Stage::WritingPrompt { index: 0, total: 0, topic: String::new() };
        assert!(stage.fraction().is_finite());
    }

    #[test]
    fn the_three_schemas_have_distinct_names() {
        let names = [
            agents::deconstructor::schema().name,
            agents::director::schema().name,
            agents::prompt_engineer::schema().name,
        ];

        let mut unique = names.to_vec();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), 3, "a shared name makes three contracts one: {names:?}");
    }
}
