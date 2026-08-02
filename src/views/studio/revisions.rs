//! The edit plan panel: to-do list, ghost previews, and the approval pipeline.
//!
//! The panel owns no editor state. It reports what the user approved; the
//! studio runs it and applies the result. Execution is asynchronous — a
//! generated cutaway takes seconds — so approving queues work and the panel
//! polls for outcomes, exactly like every other background service here.

use crate::ai_tooling::revision::models::{
    GhostKind, GhostSpan, RevisionAction, RevisionPlan, RevisionTask, TaskStatus,
};
use crate::ai_tooling::revision::timeline::{
    ClipRole, ClipView, CurrentTimelineState, TrackRole, TrackView,
};
use crate::ui::components::timeline::clip::ClipKind;
use crate::ui::components::timeline::headers::TrackKind;
use crate::ui::core::buttons::{ghost_button, pro_button};
use crate::ui::core::typography::{hairline_rule, panel, section_header};
use crate::ui::theme::tokens::*;
use crate::views::studio::timeline_panel::TimelineState;
use eframe::egui::{self, Align, Layout, RichText, ScrollArea, Ui};

/// Projects the live timeline into the read-only view the engine diffs against.
///
/// This is the whole coupling between the editor and the revision engine: one
/// function, one direction, no shared types.
pub fn snapshot(timeline: &TimelineState) -> CurrentTimelineState {
    let tracks = timeline
        .tracks
        .iter()
        .enumerate()
        .map(|(index, track)| TrackView {
            index,
            name: track.name.clone(),
            role: match track.kind {
                TrackKind::Video => TrackRole::Video,
                TrackKind::Audio => TrackRole::Audio,
            },
            locked: track.locked,
            clips: track
                .clips
                .iter()
                .map(|clip| ClipView {
                    id: clip.id,
                    label: clip.label.clone(),
                    start_sec: clip.start,
                    end_sec: clip.start + clip.len,
                    role: match clip.kind {
                        ClipKind::ARoll => ClipRole::ARoll,
                        ClipKind::BRoll => ClipRole::BRoll,
                        ClipKind::Audio => ClipRole::Audio,
                    },
                })
                .collect(),
        })
        .collect();

    CurrentTimelineState {
        tracks,
        caption_spans: timeline
            .texts
            .iter()
            .map(|text| (text.start, text.end))
            .collect(),
        // The content end, not the visible span: the engine measures the edit,
        // not the canvas it is drawn on.
        duration_sec: timeline.content_end(),
    }
}

pub struct RevisionState {
    pub plan: RevisionPlan,
    /// Task under the pointer, whose ghost is drawn on the timeline.
    pub hovered: Option<u64>,
    pub status: Option<String>,
    /// Tasks the user approved, waiting to be handed to the orchestrator.
    ///
    /// The panel cannot reach the worker and does not try to: it queues here,
    /// the app drains it once per frame and dispatches. That keeps this type
    /// free of channels, runtimes, and any knowledge that a worker exists.
    pub outbox: Vec<RevisionTask>,
}

impl Default for RevisionState {
    fn default() -> Self {
        Self {
            plan: RevisionPlan::default(),
            hovered: None,
            status: None,
            outbox: Vec::new(),
        }
    }
}

impl RevisionState {
    pub fn has_plan(&self) -> bool {
        !self.plan.is_empty()
    }

    /// Replaces the plan, discarding whatever was there.
    pub fn adopt(&mut self, plan: RevisionPlan) {
        self.status = Some(format!(
            "{} revisions against the reference \"{}\".",
            plan.tasks.len(),
            plan.competitor_title
        ));
        self.plan = plan;
        self.hovered = None;
    }

    /// Ghost spans to draw this frame. Empty unless a task is hovered.
    pub fn ghosts(&self) -> Vec<GhostSpan> {
        self.hovered
            .and_then(|id| self.plan.task(id))
            .map(|task| vec![task.action.ghost()])
            .unwrap_or_default()
    }

    /// Marks a task in flight and queues it for the orchestrator.
    pub fn approve(&mut self, task_id: u64) {
        let Some(task) = self.plan.task_mut(task_id) else {
            return;
        };
        if !task.is_pending() {
            return;
        }

        task.status = TaskStatus::Running;
        self.outbox.push(task.clone());
    }

    pub fn reject(&mut self, task_id: u64) {
        if let Some(task) = self.plan.task_mut(task_id) {
            task.status = TaskStatus::Rejected;
        }
    }

    /// Everything approved since the last call.
    pub fn take_approved(&mut self) -> Vec<RevisionTask> {
        std::mem::take(&mut self.outbox)
    }

    /// Marks a task applied, once its commands have actually landed.
    pub fn settle(&mut self, task_id: u64) {
        if let Some(task) = self.plan.task_mut(task_id) {
            task.status = TaskStatus::Done;
        }
    }

    pub fn fail(&mut self, task_id: u64, reason: String) {
        if let Some(task) = self.plan.task_mut(task_id) {
            task.status = TaskStatus::Failed(reason.clone());
        }
        self.status = Some(format!("Failed: {reason}"));
    }
}

/// Draws the panel. Hovering a row arms its ghost for the timeline pass.
pub fn show(ui: &mut Ui, state: &mut RevisionState) {
    let mut approve: Option<u64> = None;
    let mut reject: Option<u64> = None;
    let mut hovered: Option<u64> = None;

    ui.horizontal(|ui| {
        ui.label(RichText::new("Edit Plan").strong().color(TEXT_PRIMARY));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(
                RichText::new(format!(
                    "{} left · {} applied",
                    state.plan.pending_count(),
                    state.plan.applied_count()
                ))
                .small()
                .color(TEXT_SECONDARY),
            );
        });
    });

    if state.plan.is_empty() {
        ui.add_space(8.0);
        ui.label(
            RichText::new(
                "No plan yet. Examine a channel, deconstruct an outlier, then compare it \
                 against this timeline.",
            )
            .small()
            .color(TEXT_DISABLED),
        );
        return;
    }

    ui.add_space(6.0);
    ui.label(
        RichText::new(format!("Reference: {}", state.plan.competitor_title))
            .small()
            .color(TEXT_SECONDARY),
    );
    ui.add_space(6.0);
    hairline_rule(ui);
    ui.add_space(8.0);

    if !state.plan.summary.is_empty() {
        panel(ui, |ui| {
            for line in &state.plan.summary {
                ui.label(RichText::new(line).small().color(TEXT_SECONDARY));
            }
        });
        ui.add_space(8.0);
    }

    section_header(ui, "Revisions");
    ScrollArea::vertical()
        .auto_shrink([false, false])
        .id_source("revision_tasks")
        .show(ui, |ui| {
            for task in &state.plan.tasks {
                let response = row(ui, task);
                if response.approve {
                    approve = Some(task.id);
                }
                if response.reject {
                    reject = Some(task.id);
                }
                if response.hovered {
                    hovered = Some(task.id);
                }
            }
        });

    // Hover is recomputed every frame: leaving the panel clears the ghost.
    state.hovered = hovered;
    if let Some(id) = approve {
        state.approve(id);
    }
    if let Some(id) = reject {
        state.reject(id);
    }

    if let Some(status) = &state.status {
        ui.add_space(6.0);
        ui.label(RichText::new(status).small().color(TEXT_DISABLED));
    }
}

#[derive(Default)]
struct RowResponse {
    approve: bool,
    reject: bool,
    hovered: bool,
}

fn row(ui: &mut Ui, task: &RevisionTask) -> RowResponse {
    let mut out = RowResponse::default();

    let scope = ui.scope(|ui| {
        panel(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(task.action.label())
                    .strong()
                    .color(accent_for(&task.action)),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(status_text(&task.status))
                        .small()
                        .color(status_color(&task.status)),
                );
                ui.label(
                    RichText::new(format!("{} impact", task.impact_label()))
                        .small()
                        .color(TEXT_DISABLED),
                );
            });
        });

        ui.label(
            RichText::new(format!("at {:.1}s", task.action.timestamp()))
                .small()
                .color(TEXT_SECONDARY),
        );
        ui.add_space(2.0);
        ui.label(RichText::new(&task.rationale).small().color(TEXT_SECONDARY));

        if let RevisionAction::GenerateAndInsertBRoll { semantic_topic, .. } = &task.action {
            ui.add_space(2.0);
            ui.label(
                RichText::new(format!("→ {semantic_topic}"))
                    .small()
                    .italics()
                    .color(TEXT_DISABLED),
            );
        }

        if let TaskStatus::Failed(reason) = &task.status {
            ui.label(RichText::new(reason).small().color(ERR));
        }

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_enabled_ui(task.is_pending(), |ui| {
                if pro_button(ui, approve_label(&task.status), true).clicked() {
                    out.approve = true;
                }
                if ghost_button(ui, "Dismiss").clicked() {
                    out.reject = true;
                }
            });
        });
        });
    });

    // Hover is taken from the region the row actually occupies, so moving down
    // the list moves the ghost with it.
    out.hovered = ui.rect_contains_pointer(scope.response.rect);

    ui.add_space(4.0);
    out
}

fn approve_label(status: &TaskStatus) -> &'static str {
    match status {
        TaskStatus::Failed(_) => "Retry",
        _ => "Approve",
    }
}

fn status_text(status: &TaskStatus) -> String {
    status.label().to_string()
}

fn status_color(status: &TaskStatus) -> egui::Color32 {
    match status {
        TaskStatus::Proposed => TEXT_SECONDARY,
        TaskStatus::Approved | TaskStatus::Running => ACCENT,
        TaskStatus::Done => OK,
        TaskStatus::Failed(_) => ERR,
        TaskStatus::Rejected => TEXT_DISABLED,
    }
}

fn accent_for(action: &RevisionAction) -> egui::Color32 {
    match action.ghost().kind {
        GhostKind::BRoll => CLIP_BROLL,
        GhostKind::Sfx => CLIP_AUDIO,
        GhostKind::Warning => WARN,
        GhostKind::Ending => AI,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_tooling::revision::models::{Evidence, RevisionAction};
    use crate::views::studio::dnd::DragAsset;

    fn asset(name: &str, seconds: f32, kind: ClipKind) -> DragAsset {
        DragAsset {
            name: name.into(),
            path: None,
            seconds,
            kind,
            has_audio: false,
        }
    }

    fn task(id: u64, action: RevisionAction) -> RevisionTask {
        RevisionTask {
            id,
            action,
            rationale: String::new(),
            evidence: Evidence {
                competitor_video_id: "v".into(),
                competitor_time_sec: 0.0,
                observation: String::new(),
            },
            impact: 0.5,
            status: TaskStatus::Proposed,
            generation: None,
        }
    }

    #[test]
    fn the_snapshot_carries_roles_across_so_cutaways_are_recognised() {
        let mut timeline = TimelineState::default();
        timeline.place(0, &asset("interview.mp4", 30.0, ClipKind::ARoll), 0.0);
        timeline.place(0, &asset("cutaway.mp4", 4.0, ClipKind::BRoll), 30.0);

        let view = snapshot(&timeline);

        assert_eq!(view.clip_count(), 2);
        assert!(view.has_broll_at(31.0), "the generated block reads as b-roll");
        assert!(!view.has_broll_at(5.0));
        assert_eq!(view.tracks[1].role, TrackRole::Audio);
    }

    #[test]
    fn the_snapshot_measures_the_edit_not_the_visible_canvas() {
        let mut timeline = TimelineState::default();
        timeline.place(0, &asset("a.mp4", 10.0, ClipKind::ARoll), 0.0);

        // `seconds` is padded past the content so there is room to drop.
        assert!(timeline.seconds > 10.0);
        assert_eq!(snapshot(&timeline).duration_sec, 10.0);
    }

    #[test]
    fn a_locked_track_survives_the_projection() {
        let mut timeline = TimelineState::default();
        timeline.tracks[0].locked = true;

        let view = snapshot(&timeline);
        assert!(view.tracks[0].locked);
        assert_eq!(view.free_video_track_at(0.0, 1.0), None);
    }

    #[test]
    fn only_the_hovered_task_produces_a_ghost() {
        let mut state = RevisionState::default();
        state.plan.tasks = vec![
            task(
                1,
                RevisionAction::AddTransitionAudio {
                    timestamp: 5.0,
                    sfx_type: "whoosh".into(),
                },
            ),
            task(
                2,
                RevisionAction::FixRetentionDrop {
                    timestamp: 20.0,
                    suggestion: String::new(),
                },
            ),
        ];

        assert!(state.ghosts().is_empty(), "nothing hovered, nothing drawn");

        state.hovered = Some(2);
        let ghosts = state.ghosts();
        assert_eq!(ghosts.len(), 1);
        assert_eq!(ghosts[0].start_sec, 20.0);
        assert_eq!(ghosts[0].kind, GhostKind::Warning);
    }

    #[test]
    fn a_stale_hover_id_does_not_draw_a_ghost() {
        let mut state = RevisionState::default();
        state.hovered = Some(99);
        assert!(state.ghosts().is_empty());
    }

    #[test]
    fn approving_moves_a_task_to_running_and_settling_finishes_it() {
        let mut state = RevisionState::default();
        state.plan.tasks = vec![task(
            1,
            RevisionAction::FixRetentionDrop {
                timestamp: 3.0,
                suggestion: "tighten".into(),
            },
        )];

        state.approve(1);
        assert_eq!(state.plan.task(1).map(|t| t.status.clone()), Some(TaskStatus::Running));
        // A running task cannot be approved twice.
        state.approve(1);
        assert_eq!(state.plan.pending_count(), 0);

        state.settle(1);
        assert_eq!(state.plan.applied_count(), 1);
    }

    #[test]
    fn approving_queues_the_task_exactly_once_for_the_orchestrator() {
        let mut state = RevisionState::default();
        state.plan.tasks = vec![task(
            1,
            RevisionAction::FixRetentionDrop { timestamp: 3.0, suggestion: "x".into() },
        )];

        state.approve(1);
        state.approve(1); // a second click while it is running

        let queued = state.take_approved();
        assert_eq!(queued.len(), 1, "a double click must not run it twice");
        assert_eq!(queued[0].id, 1);
        assert!(state.take_approved().is_empty(), "draining is destructive");
    }

    #[test]
    fn a_failure_from_the_worker_reopens_the_task_for_a_retry() {
        let mut state = RevisionState::default();
        state.plan.tasks = vec![task(
            1,
            RevisionAction::FixRetentionDrop { timestamp: 0.0, suggestion: String::new() },
        )];
        state.approve(1);
        let _ = state.take_approved();

        state.fail(1, "generator exploded".into());

        assert!(state.plan.task(1).expect("task").is_pending(), "retryable");
        assert!(state.status.as_deref().is_some_and(|s| s.contains("exploded")));
    }

    #[test]
    fn dismissing_takes_a_task_out_of_the_queue_for_good() {
        let mut state = RevisionState::default();
        state.plan.tasks = vec![task(
            1,
            RevisionAction::FixRetentionDrop {
                timestamp: 0.0,
                suggestion: String::new(),
            },
        )];

        state.reject(1);
        assert_eq!(state.plan.pending_count(), 0);
        state.approve(1);
        assert_eq!(
            state.plan.task(1).map(|t| t.status.clone()),
            Some(TaskStatus::Rejected),
            "a dismissed task stays dismissed"
        );
    }

    /// The whole chain, with no UI in it: a timeline goes in, a deconstructed
    /// competitor is diffed against it, an approved task is executed, and the
    /// result lands on the same timeline through the ordinary dispatcher.
    #[test]
    fn an_approved_broll_task_travels_from_the_warehouse_onto_the_timeline() {
        use crate::ai_tooling::competitor::store::InMemoryWarehouse;
        use crate::ai_tooling::competitor::{deconstruct, models::CompetitorVideo};
        use crate::ai_tooling::revision::diff::{ComparisonEngine, DiffSettings};
        use crate::ai_tooling::revision::executor::execute_task;
        use crate::ai_tooling::revision::generation::{GeneratedKind, MockGenerator};
        use crate::ai_tooling::revision::models::RevisionAction;
        use crate::ai_tooling::youtube_insights::models::OutlierMethod;
        use crate::ai_tooling::youtube_insights::ViralScore;
        use crate::media::MediaKind;
        use crate::views::studio::dispatch;
        use crate::views::studio::media_panel::{Asset, MediaState};

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("runtime");

        // A 90s edit that is one unbroken talking head, plus an empty overlay
        // lane and an empty audio lane.
        let mut timeline = TimelineState::default();
        timeline.add_track(TrackKind::Video);
        timeline.place(0, &asset("interview.mp4", 90.0, ClipKind::ARoll), 0.0);
        let overlay = timeline
            .tracks
            .iter()
            .position(|t| t.kind == TrackKind::Video && t.clips.is_empty())
            .expect("an empty video lane");

        let score = ViralScore {
            video_id: "viral1".into(),
            title: "the reference".into(),
            view_count: 2_000_000,
            baseline_views: 200_000.0,
            multiplier: 10.0,
            modified_z: 8.0,
            percentile: 0.99,
            method: OutlierMethod::ModifiedZScore,
            is_outlier: true,
        };

        let (plan, competitor): (RevisionPlan, CompetitorVideo) = runtime.block_on(async {
            let warehouse = InMemoryWarehouse::new();
            let competitor = deconstruct(&score, "UC1", 360.0, &warehouse, &warehouse)
                .await
                .expect("deconstruct");

            let engine = ComparisonEngine::new(&warehouse, DiffSettings::default());
            let plan = engine.compare(&competitor, &snapshot(&timeline)).await;
            (plan, competitor)
        });

        assert_eq!(plan.competitor_video_id, competitor.video_id);
        let task = plan
            .tasks
            .iter()
            .find(|t| matches!(t.action, RevisionAction::GenerateAndInsertBRoll { .. }))
            .expect("a static 90s shot against replay peaks must yield b-roll work");

        // Execute the slow half exactly as the worker does.
        let outcome = runtime
            .block_on(execute_task(task, &MockGenerator::new(0)))
            .expect("execute");

        // Then the fast half, exactly as `apply_revisions` does.
        let mut pool = MediaState::default();
        for generated in &outcome.assets {
            pool.assets.push(Asset::demo(
                &generated.name,
                "AI generated",
                generated.duration_sec,
                match generated.kind {
                    GeneratedKind::Audio => MediaKind::Audio,
                    GeneratedKind::Video => MediaKind::Video,
                },
                true,
            ));
        }

        let before = timeline.clip_count();
        let (tx, _rx) = std::sync::mpsc::channel();
        let report = dispatch::dispatch(outcome.commands, &mut timeline, &pool, &tx);

        assert!(!report.had_failures(), "{}", report.feedback());
        assert_eq!(timeline.clip_count(), before + 1, "the cutaway is on the timeline");

        let placed = timeline.tracks[overlay]
            .clips
            .first()
            .expect("placed on the free overlay lane");
        assert!(
            placed.kind == ClipKind::BRoll,
            "generated assets land as b-roll"
        );

        // And the loop closes: a fresh snapshot now sees the coverage, so the
        // same comparison would no longer propose it.
        assert!(snapshot(&timeline).has_broll_at(placed.start + 0.1));
    }

    #[test]
    fn adopting_a_plan_replaces_the_previous_one() {
        let mut state = RevisionState::default();
        state.plan.tasks = vec![task(
            1,
            RevisionAction::FixRetentionDrop { timestamp: 0.0, suggestion: String::new() },
        )];
        state.hovered = Some(1);

        state.adopt(RevisionPlan {
            competitor_video_id: "new".into(),
            competitor_title: "new reference".into(),
            tasks: Vec::new(),
            summary: Vec::new(),
        });

        assert!(!state.has_plan());
        assert_eq!(state.hovered, None, "the old hover cannot outlive its plan");
    }
}
