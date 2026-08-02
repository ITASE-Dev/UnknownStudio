//! The edit plan: what to change, why, and what it will look like.

use crate::ai_tooling::revision::generation::GenerationRequest;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One concrete change to the user's timeline.
///
/// Deliberately specific: every variant carries everything needed to execute
/// it, so an approved task never has to go back to the engine for a detail.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RevisionAction {
    GenerateAndInsertBRoll {
        timestamp: f32,
        duration: f32,
        semantic_topic: String,
        generation_prompt: String,
        track_index: usize,
    },
    AddTransitionAudio {
        timestamp: f32,
        sfx_type: String,
    },
    FixRetentionDrop {
        timestamp: f32,
        suggestion: String,
    },
    ReviseEnding {
        start_time: f32,
        action: String,
    },
    /// Isolate a silent span so it can be dropped. `clip_id` is the clip the
    /// silence falls inside, as the timeline reports it.
    CutSilence {
        clip_id: String,
        start_time: f32,
        end_time: f32,
    },
}

impl RevisionAction {
    pub fn label(&self) -> &'static str {
        match self {
            Self::GenerateAndInsertBRoll { .. } => "Generate B-roll",
            Self::AddTransitionAudio { .. } => "Add transition SFX",
            Self::FixRetentionDrop { .. } => "Fix retention drop",
            Self::ReviseEnding { .. } => "Revise ending",
            Self::CutSilence { .. } => "Cut silence",
        }
    }

    /// Where on the timeline this lands.
    pub fn timestamp(&self) -> f32 {
        match self {
            Self::GenerateAndInsertBRoll { timestamp, .. }
            | Self::AddTransitionAudio { timestamp, .. }
            | Self::FixRetentionDrop { timestamp, .. } => *timestamp,
            Self::ReviseEnding { start_time, .. } | Self::CutSilence { start_time, .. } => {
                *start_time
            }
        }
    }

    /// Whether executing this needs an asset generated first.
    pub fn needs_generation(&self) -> bool {
        matches!(self, Self::GenerateAndInsertBRoll { .. })
    }

    /// The span this occupies, for the ghost preview.
    pub fn ghost(&self) -> GhostSpan {
        match self {
            Self::GenerateAndInsertBRoll {
                timestamp,
                duration,
                track_index,
                semantic_topic,
                ..
            } => GhostSpan {
                track_index: Some(*track_index),
                start_sec: *timestamp,
                duration_sec: *duration,
                kind: GhostKind::BRoll,
                label: semantic_topic.clone(),
            },
            Self::AddTransitionAudio { timestamp, sfx_type } => GhostSpan {
                track_index: None,
                start_sec: *timestamp,
                // An SFX has no meaningful length on the timeline; show a
                // marker-width sliver rather than a zero-width invisible one.
                duration_sec: 0.6,
                kind: GhostKind::Sfx,
                label: sfx_type.clone(),
            },
            Self::FixRetentionDrop { timestamp, .. } => GhostSpan {
                track_index: None,
                start_sec: *timestamp,
                duration_sec: 2.0,
                kind: GhostKind::Warning,
                label: "retention drop".into(),
            },
            Self::ReviseEnding { start_time, .. } => GhostSpan {
                track_index: None,
                start_sec: *start_time,
                duration_sec: 4.0,
                kind: GhostKind::Ending,
                label: "ending".into(),
            },
            // The only ghost with an exact span the user can check by eye: it
            // is precisely the footage that would be isolated.
            Self::CutSilence { start_time, end_time, .. } => GhostSpan {
                track_index: None,
                start_sec: *start_time,
                duration_sec: (end_time - start_time).max(0.05),
                kind: GhostKind::Warning,
                label: "silence".into(),
            },
        }
    }
}

/// A translucent preview drawn over the timeline while a task is hovered.
#[derive(Debug, Clone, PartialEq)]
pub struct GhostSpan {
    /// Track to draw on. `None` spans every lane — the change is not bound to
    /// one track (an advisory note, or an ending revision).
    pub track_index: Option<usize>,
    pub start_sec: f32,
    pub duration_sec: f32,
    pub kind: GhostKind,
    pub label: String,
}

impl GhostSpan {
    pub fn end_sec(&self) -> f32 {
        self.start_sec + self.duration_sec
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GhostKind {
    BRoll,
    Sfx,
    Warning,
    Ending,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// Waiting for the user.
    Proposed,
    /// Approved and queued; the pipeline has not started it yet.
    Approved,
    /// Generating or applying.
    Running,
    /// Applied to the timeline.
    Done,
    Failed(String),
    /// Dismissed by the user; kept so the engine does not re-propose it.
    Rejected,
}

impl TaskStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Approved => "approved",
            Self::Running => "working",
            Self::Done => "applied",
            Self::Failed(_) => "failed",
            Self::Rejected => "dismissed",
        }
    }

    pub fn is_settled(&self) -> bool {
        matches!(self, Self::Done | Self::Rejected)
    }

    pub fn is_actionable(&self) -> bool {
        matches!(self, Self::Proposed | Self::Failed(_))
    }
}

/// What in the competitor's video justifies this change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Evidence {
    pub competitor_video_id: String,
    /// The moment in *their* video this was read from.
    pub competitor_time_sec: f32,
    /// One line naming the observation.
    pub observation: String,
}

/// A proposed change, with everything the UI needs to explain it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RevisionTask {
    pub id: u64,
    pub action: RevisionAction,
    /// Why this helps, in the user's terms.
    pub rationale: String,
    pub evidence: Evidence,
    /// Estimated retention benefit, 0.0..=1.0. Orders the list.
    pub impact: f32,
    pub status: TaskStatus,
    /// Prepared payload for the generator. Present only for B-roll tasks, and
    /// the only way one can be built — see [`GenerationRequest`].
    pub generation: Option<GenerationRequest>,
}

impl RevisionTask {
    pub fn is_pending(&self) -> bool {
        self.status.is_actionable()
    }

    /// Impact as a short grade, for the list.
    pub fn impact_label(&self) -> &'static str {
        match self.impact {
            i if i >= 0.66 => "high",
            i if i >= 0.33 => "medium",
            _ => "low",
        }
    }
}

/// The full plan for one comparison.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RevisionPlan {
    /// Reference the plan was derived from.
    pub competitor_video_id: String,
    pub competitor_title: String,
    pub tasks: Vec<RevisionTask>,
    /// Headline differences, for the panel header.
    pub summary: Vec<String>,
}

impl RevisionPlan {
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    pub fn pending(&self) -> impl Iterator<Item = &RevisionTask> {
        self.tasks.iter().filter(|task| task.is_pending())
    }

    pub fn pending_count(&self) -> usize {
        self.pending().count()
    }

    pub fn applied_count(&self) -> usize {
        self.tasks
            .iter()
            .filter(|task| task.status == TaskStatus::Done)
            .count()
    }

    pub fn task(&self, id: u64) -> Option<&RevisionTask> {
        self.tasks.iter().find(|task| task.id == id)
    }

    pub fn task_mut(&mut self, id: u64) -> Option<&mut RevisionTask> {
        self.tasks.iter_mut().find(|task| task.id == id)
    }

    /// Most valuable first — the order the user should work through.
    pub fn sort_by_impact(&mut self) {
        self.tasks.sort_by(|a, b| b.impact.total_cmp(&a.impact));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the wire tags.
    ///
    /// These strings are the contract with the model — they appear in the
    /// generated schema, in the director's system prompt, and in every plan
    /// ever written to disk. `rename_all = "snake_case"` splits `BRoll` into
    /// `b_roll`, which is easy to mistype as `broll` when writing a prompt by
    /// hand. Renaming a variant silently changes all three.
    #[test]
    fn the_serialized_tags_are_the_ones_the_schema_and_the_prompts_use() {
        let tag = |action: RevisionAction| {
            serde_json::to_value(&action).expect("serialize")["kind"]
                .as_str()
                .expect("a tag")
                .to_string()
        };

        assert_eq!(
            tag(RevisionAction::GenerateAndInsertBRoll {
                timestamp: 0.0,
                duration: 1.0,
                semantic_topic: String::new(),
                generation_prompt: String::new(),
                track_index: 0,
            }),
            "generate_and_insert_b_roll"
        );
        assert_eq!(
            tag(RevisionAction::AddTransitionAudio {
                timestamp: 0.0,
                sfx_type: String::new(),
            }),
            "add_transition_audio"
        );
        assert_eq!(
            tag(RevisionAction::FixRetentionDrop {
                timestamp: 0.0,
                suggestion: String::new(),
            }),
            "fix_retention_drop"
        );
        assert_eq!(
            tag(RevisionAction::ReviseEnding {
                start_time: 0.0,
                action: String::new(),
            }),
            "revise_ending"
        );
        assert_eq!(
            tag(RevisionAction::CutSilence {
                clip_id: String::new(),
                start_time: 0.0,
                end_time: 1.0,
            }),
            "cut_silence"
        );
    }

    #[test]
    fn a_broll_ghost_lands_on_its_own_track_with_its_own_length() {
        let action = RevisionAction::GenerateAndInsertBRoll {
            timestamp: 12.0,
            duration: 4.0,
            semantic_topic: "desk overhead".into(),
            generation_prompt: "…".into(),
            track_index: 1,
        };

        let ghost = action.ghost();
        assert_eq!(ghost.track_index, Some(1));
        assert_eq!(ghost.start_sec, 12.0);
        assert_eq!(ghost.end_sec(), 16.0);
        assert_eq!(ghost.kind, GhostKind::BRoll);
    }

    #[test]
    fn a_point_action_still_gets_a_visible_ghost() {
        let action = RevisionAction::AddTransitionAudio {
            timestamp: 30.0,
            sfx_type: "whoosh".into(),
        };
        let ghost = action.ghost();

        assert!(ghost.duration_sec > 0.0, "a zero-width ghost cannot be seen");
        assert_eq!(ghost.track_index, None, "not bound to one lane");
    }

    #[test]
    fn only_broll_needs_an_asset_generated() {
        let broll = RevisionAction::GenerateAndInsertBRoll {
            timestamp: 0.0,
            duration: 1.0,
            semantic_topic: String::new(),
            generation_prompt: String::new(),
            track_index: 0,
        };
        let sfx = RevisionAction::AddTransitionAudio {
            timestamp: 0.0,
            sfx_type: "whoosh".into(),
        };

        assert!(broll.needs_generation());
        assert!(!sfx.needs_generation());
    }

    #[test]
    fn the_plan_orders_by_impact_and_counts_what_is_left() {
        let make = |id: u64, impact: f32, status: TaskStatus| RevisionTask {
            id,
            action: RevisionAction::FixRetentionDrop {
                timestamp: 0.0,
                suggestion: String::new(),
            },
            rationale: String::new(),
            evidence: Evidence {
                competitor_video_id: "v".into(),
                competitor_time_sec: 0.0,
                observation: String::new(),
            },
            impact,
            status,
            generation: None,
        };

        let mut plan = RevisionPlan {
            competitor_video_id: "v".into(),
            competitor_title: "t".into(),
            tasks: vec![
                make(1, 0.2, TaskStatus::Proposed),
                make(2, 0.9, TaskStatus::Proposed),
                make(3, 0.5, TaskStatus::Done),
            ],
            summary: Vec::new(),
        };
        plan.sort_by_impact();

        assert_eq!(plan.tasks[0].id, 2, "highest impact first");
        assert_eq!(plan.pending_count(), 2, "the applied one is not pending");
        assert_eq!(plan.applied_count(), 1);
    }

    #[test]
    fn a_failed_task_can_be_retried_but_a_dismissed_one_cannot() {
        assert!(TaskStatus::Failed("boom".into()).is_actionable());
        assert!(TaskStatus::Proposed.is_actionable());
        assert!(!TaskStatus::Rejected.is_actionable());
        assert!(!TaskStatus::Done.is_actionable());
        assert!(!TaskStatus::Running.is_actionable());
    }

    #[test]
    fn impact_grades_cover_the_whole_range() {
        let grade = |impact: f32| {
            RevisionTask {
                id: 0,
                action: RevisionAction::FixRetentionDrop {
                    timestamp: 0.0,
                    suggestion: String::new(),
                },
                rationale: String::new(),
                evidence: Evidence {
                    competitor_video_id: String::new(),
                    competitor_time_sec: 0.0,
                    observation: String::new(),
                },
                impact,
                status: TaskStatus::Proposed,
                generation: None,
            }
            .impact_label()
        };

        assert_eq!(grade(0.9), "high");
        assert_eq!(grade(0.4), "medium");
        assert_eq!(grade(0.0), "low");
    }
}
