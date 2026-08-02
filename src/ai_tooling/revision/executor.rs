//! Turning an approved task into edits.
//!
//! Execution is split in two on purpose. The slow half — generating a cutaway,
//! resolving an effect — is async and runs on a worker. The fast half is a list
//! of [`ActionCommand`]s the UI thread applies through the dispatcher that
//! already exists, so an AI-driven edit and a user-driven one go through the
//! same validation and land in the same undo history.
//!
//! Nothing here touches editor state. It cannot: it has no handle to it.

use crate::ai_tooling::orchestration::dispatcher::ActionCommand;
use crate::ai_tooling::revision::generation::{
    generate, AssetGenerator, GeneratedAsset, GeneratedKind, GenerationError,
};
use crate::ai_tooling::revision::models::{RevisionAction, RevisionTask};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ExecutionError {
    #[error("generation: {0}")]
    Generation(#[from] GenerationError),

    /// A B-roll task arrived without its prepared payload — the engine builds
    /// one for every such task, so this means the plan was tampered with.
    #[error("task {0} inserts b-roll but carries no generation request")]
    MissingPayload(u64),

    #[error("no sound effect named {0}")]
    UnknownSfx(String),

    /// A silence that ends before it starts would split the clip twice at the
    /// same place and isolate nothing.
    #[error("silent span {start:.2}s–{end:.2}s is empty or inverted")]
    EmptySpan { start: f32, end: f32 },
}

pub type Result<T> = std::result::Result<T, ExecutionError>;

/// Everything the UI thread needs to apply one task.
#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionOutcome {
    pub task_id: u64,
    /// Register these in the media pool *before* applying the commands: the
    /// dispatcher refuses to place an asset it cannot find.
    pub assets: Vec<GeneratedAsset>,
    pub commands: Vec<ActionCommand>,
    /// One line for the status area.
    pub note: String,
}

/// Runs the slow half of a task.
///
/// Async, and free of editor state, so it is safe to spawn. The returned
/// commands are applied later, synchronously, by the caller.
pub async fn execute_task<G: AssetGenerator>(
    task: &RevisionTask,
    generator: &G,
) -> Result<ExecutionOutcome> {
    match &task.action {
        RevisionAction::GenerateAndInsertBRoll {
            timestamp,
            track_index,
            semantic_topic,
            ..
        } => {
            let request = task
                .generation
                .as_ref()
                .ok_or(ExecutionError::MissingPayload(task.id))?;

            // `generate` validates the identity constraint before the backend
            // is reached; a face shot without it fails here, not silently.
            let asset = generate(generator, request).await?;

            Ok(ExecutionOutcome {
                task_id: task.id,
                note: format!("Generated “{semantic_topic}” and placed it at {timestamp:.1}s."),
                commands: vec![ActionCommand::PlaceAsset {
                    asset: asset.name.clone(),
                    target_track_idx: *track_index,
                    target_time_sec: *timestamp,
                }],
                assets: vec![asset],
            })
        }

        RevisionAction::AddTransitionAudio { timestamp, sfx_type } => {
            let asset = resolve_sfx(sfx_type).await?;

            Ok(ExecutionOutcome {
                task_id: task.id,
                note: format!("Placed a {sfx_type} on the cut at {timestamp:.1}s."),
                commands: vec![ActionCommand::AddAudio {
                    start_sec: *timestamp,
                    file_id: asset.name.clone(),
                    volume_db: -9.0,
                }],
                assets: vec![asset],
            })
        }

        // Advisory. The engine can see that a span is too slow, but *which*
        // frames to lose is an editorial call — so this lands as a marker at
        // the exact second, with the instruction attached, rather than
        // silently cutting the user's footage.
        RevisionAction::FixRetentionDrop { timestamp, suggestion } => Ok(ExecutionOutcome {
            task_id: task.id,
            note: format!("Marked {timestamp:.1}s for a pacing fix."),
            assets: Vec::new(),
            commands: vec![
                ActionCommand::AddMarker {
                    time_sec: *timestamp,
                    color: "amber".into(),
                    label: suggestion.clone(),
                },
                ActionCommand::SetPlayhead { time_sec: *timestamp },
            ],
        }),

        // Isolates the silence with two splits, then marks it. The order is
        // load-bearing: a split leaves the *head* under the original id and
        // gives the tail a new one, so cutting at `end_time` first keeps
        // `clip_id` valid for the second cut. Reversed, the second split would
        // aim past the end of what `clip_id` now covers and be rejected.
        //
        // The deletion itself is left to the user: the engine is confident
        // about where the silence is, not about whether losing that breath is
        // what the edit wants.
        RevisionAction::CutSilence {
            clip_id,
            start_time,
            end_time,
        } => {
            if end_time <= start_time {
                return Err(ExecutionError::EmptySpan {
                    start: *start_time,
                    end: *end_time,
                });
            }

            Ok(ExecutionOutcome {
                task_id: task.id,
                note: format!(
                    "Isolated {:.2}s of silence at {start_time:.1}s — delete the marked segment.",
                    end_time - start_time
                ),
                assets: Vec::new(),
                commands: vec![
                    ActionCommand::SplitClip {
                        clip_id: clip_id.clone(),
                        time_sec: *end_time,
                    },
                    ActionCommand::SplitClip {
                        clip_id: clip_id.clone(),
                        time_sec: *start_time,
                    },
                    ActionCommand::AddMarker {
                        time_sec: *start_time,
                        color: "amber".into(),
                        label: format!("silence — {:.2}s to delete", end_time - start_time),
                    },
                    ActionCommand::SetPlayhead {
                        time_sec: *start_time,
                    },
                ],
            })
        }

        RevisionAction::ReviseEnding { start_time, action } => Ok(ExecutionOutcome {
            task_id: task.id,
            note: format!("Marked the outro at {start_time:.1}s."),
            assets: Vec::new(),
            commands: vec![
                ActionCommand::AddMarker {
                    time_sec: *start_time,
                    color: "violet".into(),
                    label: action.clone(),
                },
                ActionCommand::SetPlayhead { time_sec: *start_time },
            ],
        }),
    }
}

/// Sound-effect library. Stands in for a bundled pack on disk.
const SFX_LIBRARY: [(&str, f32); 4] = [
    ("whoosh", 0.6),
    ("riser", 1.4),
    ("sub_drop", 1.1),
    ("impact", 0.5),
];

async fn resolve_sfx(sfx_type: &str) -> Result<GeneratedAsset> {
    tokio::time::sleep(std::time::Duration::from_millis(60)).await;

    let (name, duration) = SFX_LIBRARY
        .iter()
        .find(|(name, _)| *name == sfx_type)
        .ok_or_else(|| ExecutionError::UnknownSfx(sfx_type.to_string()))?;

    Ok(GeneratedAsset {
        name: format!("sfx_{name}.wav"),
        duration_sec: *duration,
        kind: GeneratedKind::Audio,
        path: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_tooling::revision::generation::{
        GenerationRequest, IdentityMode, MockGenerator, FACIAL_IDENTITY_CONSTRAINT,
    };
    use crate::ai_tooling::revision::models::{Evidence, TaskStatus};

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("runtime")
    }

    fn task(action: RevisionAction, generation: Option<GenerationRequest>) -> RevisionTask {
        RevisionTask {
            id: 7,
            action,
            rationale: String::new(),
            evidence: Evidence {
                competitor_video_id: "v".into(),
                competitor_time_sec: 0.0,
                observation: String::new(),
            },
            impact: 0.5,
            status: TaskStatus::Approved,
            generation,
        }
    }

    #[test]
    fn an_approved_broll_task_generates_an_asset_and_a_place_command() {
        runtime().block_on(async {
            let request = GenerationRequest::new(
                "overhead desk shot",
                "Overhead desk shot.",
                IdentityMode::NoPresenter,
                3.5,
            );
            let task = task(
                RevisionAction::GenerateAndInsertBRoll {
                    timestamp: 12.0,
                    duration: 3.5,
                    semantic_topic: "overhead desk shot".into(),
                    generation_prompt: request.prompt().into(),
                    track_index: 1,
                },
                Some(request),
            );

            let outcome = execute_task(&task, &MockGenerator::new(0)).await.expect("execute");

            assert_eq!(outcome.assets.len(), 1, "the pool gets the new asset");
            assert_eq!(outcome.assets[0].kind, GeneratedKind::Video);
            assert_eq!(
                outcome.commands,
                vec![ActionCommand::PlaceAsset {
                    asset: outcome.assets[0].name.clone(),
                    target_track_idx: 1,
                    target_time_sec: 12.0,
                }]
            );
        });
    }

    #[test]
    fn the_placed_asset_name_matches_what_the_pool_will_be_told_about() {
        runtime().block_on(async {
            let request = GenerationRequest::new("a topic", "prompt", IdentityMode::NoPresenter, 2.0);
            let task = task(
                RevisionAction::GenerateAndInsertBRoll {
                    timestamp: 0.0,
                    duration: 2.0,
                    semantic_topic: "a topic".into(),
                    generation_prompt: String::new(),
                    track_index: 0,
                },
                Some(request),
            );

            let outcome = execute_task(&task, &MockGenerator::new(0)).await.expect("execute");
            match &outcome.commands[0] {
                ActionCommand::PlaceAsset { asset, .. } => {
                    assert_eq!(*asset, outcome.assets[0].name, "or the dispatcher rejects it");
                }
                other => panic!("wrong command: {other:?}"),
            }
        });
    }

    #[test]
    fn a_face_shot_missing_its_constraint_fails_instead_of_rendering() {
        runtime().block_on(async {
            // Only reachable by tampering; serde is the realistic route in.
            let json = format!(
                r#"{{"prompt":"Close up of the presenter.","negative_prompt":"",
                     "identity":{{"mode":"presenter_face","reference_asset":"ref.png"}},
                     "duration_sec":3.0,"semantic_topic":"presenter"}}"#
            );
            let tampered: GenerationRequest = serde_json::from_str(&json).expect("deserialize");
            assert!(!tampered.prompt().contains(FACIAL_IDENTITY_CONSTRAINT));

            let task = task(
                RevisionAction::GenerateAndInsertBRoll {
                    timestamp: 0.0,
                    duration: 3.0,
                    semantic_topic: "presenter".into(),
                    generation_prompt: String::new(),
                    track_index: 0,
                },
                Some(tampered),
            );

            let err = execute_task(&task, &MockGenerator::new(0))
                .await
                .expect_err("must refuse");
            assert!(matches!(
                err,
                ExecutionError::Generation(GenerationError::MissingIdentityConstraint)
            ));
        });
    }

    #[test]
    fn a_broll_task_without_its_payload_is_refused_rather_than_improvised() {
        runtime().block_on(async {
            let task = task(
                RevisionAction::GenerateAndInsertBRoll {
                    timestamp: 0.0,
                    duration: 1.0,
                    semantic_topic: "x".into(),
                    generation_prompt: String::new(),
                    track_index: 0,
                },
                None,
            );

            let err = execute_task(&task, &MockGenerator::new(0))
                .await
                .expect_err("must refuse");
            assert!(matches!(err, ExecutionError::MissingPayload(7)));
        });
    }

    #[test]
    fn a_transition_resolves_to_a_library_effect_on_an_audio_lane() {
        runtime().block_on(async {
            let task = task(
                RevisionAction::AddTransitionAudio {
                    timestamp: 30.0,
                    sfx_type: "riser".into(),
                },
                None,
            );

            let outcome = execute_task(&task, &MockGenerator::new(0)).await.expect("execute");
            assert_eq!(outcome.assets[0].name, "sfx_riser.wav");
            assert_eq!(outcome.assets[0].kind, GeneratedKind::Audio);
            assert!(matches!(
                outcome.commands[0],
                ActionCommand::AddAudio { start_sec, .. } if start_sec == 30.0
            ));
        });
    }

    #[test]
    fn an_effect_that_is_not_in_the_library_fails_loudly() {
        runtime().block_on(async {
            let task = task(
                RevisionAction::AddTransitionAudio {
                    timestamp: 0.0,
                    sfx_type: "airhorn".into(),
                },
                None,
            );

            let err = execute_task(&task, &MockGenerator::new(0))
                .await
                .expect_err("must refuse");
            assert!(matches!(err, ExecutionError::UnknownSfx(name) if name == "airhorn"));
        });
    }

    #[test]
    fn a_pacing_fix_marks_the_spot_rather_than_cutting_the_users_footage() {
        runtime().block_on(async {
            let task = task(
                RevisionAction::FixRetentionDrop {
                    timestamp: 18.0,
                    suggestion: "Cut the silences.".into(),
                },
                None,
            );

            let outcome = execute_task(&task, &MockGenerator::new(0)).await.expect("execute");
            assert!(outcome.assets.is_empty());
            assert!(
                outcome
                    .commands
                    .iter()
                    .all(|c| !matches!(c, ActionCommand::DeleteClip { .. } | ActionCommand::TrimClip { .. })),
                "an advisory task never destroys footage"
            );
            assert!(matches!(
                &outcome.commands[0],
                ActionCommand::AddMarker { label, .. } if label == "Cut the silences."
            ));
        });
    }

    #[test]
    fn every_outcome_reports_the_task_it_came_from() {
        runtime().block_on(async {
            let task = task(
                RevisionAction::ReviseEnding {
                    start_time: 55.0,
                    action: "Tease the next video.".into(),
                },
                None,
            );

            let outcome = execute_task(&task, &MockGenerator::new(0)).await.expect("execute");
            assert_eq!(outcome.task_id, 7, "so the UI can settle the right row");
            assert!(!outcome.note.is_empty());
        });
    }
}
