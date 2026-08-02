//! The diff engine: a viral profile on one side, a populated edit on the other.
//!
//! The comparison is *positional, not literal*. Two videos of different lengths
//! still share a shape, so every competitor observation is mapped onto the
//! user's timeline by narrative proportion before anything is proposed. A beat
//! 22% into a 12-minute reference is checked against 22% of a 4-minute edit.
//!
//! Every rule then has the same form: the competitor did X at this beat, the
//! user's timeline does *not* have X there, and the timeline has room for it.
//! All three must hold. A proposal that duplicates work the user already did is
//! worse than no proposal — it teaches them to stop reading the list.

use crate::ai_tooling::competitor::models::{
    BRollPlacement, CompetitorVideo, DropCause, EndingStyle,
};
use crate::ai_tooling::competitor::store::{SemanticIndex, SemanticKind};
use crate::ai_tooling::revision::generation::{GenerationRequest, IdentityMode};
use crate::ai_tooling::revision::models::{
    Evidence, RevisionAction, RevisionPlan, RevisionTask, TaskStatus,
};
use crate::ai_tooling::revision::timeline::CurrentTimelineState;
use std::sync::atomic::{AtomicU64, Ordering};

/// Thresholds the rules read. Exposed so the UI can loosen a noisy engine
/// rather than the engine guessing.
#[derive(Debug, Clone)]
pub struct DiffSettings {
    /// An unbroken A-roll shot longer than this reads as static.
    pub static_hold_sec: f32,
    /// An existing SFX this close to a proposed one counts as already there.
    pub sfx_tolerance_sec: f32,
    /// Default length of a generated cutaway.
    pub broll_duration_sec: f32,
    /// Below this share of the competitor's cutting rate, pacing is flagged.
    pub pacing_ratio_floor: f32,
    /// Hard cap, so a long reference cannot bury the user in tasks.
    pub max_tasks: usize,
    /// The user's own face reference. Without one, no generated shot may
    /// render the presenter — see [`identity_for`].
    pub presenter_reference: Option<String>,
}

impl Default for DiffSettings {
    fn default() -> Self {
        Self {
            static_hold_sec: 6.0,
            sfx_tolerance_sec: 0.75,
            broll_duration_sec: 3.5,
            pacing_ratio_floor: 0.6,
            max_tasks: 24,
            presenter_reference: None,
        }
    }
}

pub struct ComparisonEngine<'a, V: SemanticIndex> {
    index: &'a V,
    settings: DiffSettings,
    /// Atomic rather than a `Cell`: the engine is borrowed across `await`
    /// points inside a spawned task, which requires it to be `Sync`.
    next_id: AtomicU64,
}

impl<'a, V: SemanticIndex> ComparisonEngine<'a, V> {
    pub fn new(index: &'a V, settings: DiffSettings) -> Self {
        Self {
            index,
            settings,
            next_id: AtomicU64::new(1),
        }
    }

    fn id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Runs every rule and returns the plan, most valuable first.
    pub async fn compare(
        &self,
        competitor: &CompetitorVideo,
        current: &CurrentTimelineState,
    ) -> RevisionPlan {
        let mut plan = RevisionPlan {
            competitor_video_id: competitor.video_id.clone(),
            competitor_title: competitor.title.clone(),
            summary: self.summarize(competitor, current),
            tasks: Vec::new(),
        };

        if current.is_empty() {
            plan.summary.push(
                "The timeline is empty — place your A-roll first, then compare again.".into(),
            );
            return plan;
        }

        plan.tasks.extend(self.broll_gaps(competitor, current).await);
        plan.tasks.extend(self.missing_transitions(competitor, current));
        plan.tasks.extend(self.structural_weaknesses(competitor, current));
        if let Some(task) = self.ending(competitor, current) {
            plan.tasks.push(task);
        }

        plan.sort_by_impact();
        plan.tasks.truncate(self.settings.max_tasks);
        plan
    }

    /// Headline numbers, so the user can see the gap before reading the list.
    fn summarize(
        &self,
        competitor: &CompetitorVideo,
        current: &CurrentTimelineState,
    ) -> Vec<String> {
        let their_cuts = competitor.structure.cuts_per_minute(competitor.duration_sec);
        let our_cuts = current.cuts_per_minute();
        let their_broll = competitor.structure.broll_coverage(competitor.duration_sec) * 100.0;
        let our_broll = current.broll_coverage() * 100.0;
        let their_sfx = competitor.audio.sfx_per_minute(competitor.duration_sec);

        vec![
            format!("Cutting: {our_cuts:.1}/min here against {their_cuts:.1}/min in the reference."),
            format!("B-roll coverage: {our_broll:.0}% here against {their_broll:.0}% there."),
            format!("Transition SFX: {their_sfx:.1}/min in the reference."),
            format!(
                "Hook: {} opening, {:.0} wpm, {} cuts in the first seconds.",
                competitor.transcript.hook.hook_type.label(),
                competitor.transcript.hook.words_per_minute,
                competitor.transcript.hook.cuts_in_hook
            ),
        ]
    }

    /// Rule 1 — a replay peak carried by a cutaway there, a static shot here.
    ///
    /// This is the highest-value rule: the audience *chose* to rewatch those
    /// seconds, and the thing they were watching was not a talking head.
    async fn broll_gaps(
        &self,
        competitor: &CompetitorVideo,
        current: &CurrentTimelineState,
    ) -> Vec<RevisionTask> {
        let mut tasks = Vec::new();

        for peak in &competitor.retention.peaks {
            let here = current.map_narrative_time(peak.start_sec, competitor.duration_sec);

            if current.has_broll_at(here) {
                continue;
            }
            let hold = current.static_hold_sec(here);
            if hold < self.settings.static_hold_sec {
                continue;
            }

            // What did they cut to? Prefer the shot actually covering the peak;
            // fall back to the nearest topic by meaning.
            let topic = match competitor.structure.broll_at(peak.start_sec) {
                Some(shot) => shot.clone(),
                None => match self.nearest_topic(&peak.description).await {
                    Some(shot) => shot,
                    None => continue,
                },
            };

            let duration = self
                .settings
                .broll_duration_sec
                .min(peak.duration_sec().max(2.0));

            let Some(track_index) = current.free_video_track_at(here, duration) else {
                // Nowhere to put it without covering something the user placed.
                continue;
            };

            let (identity, prompt_note) = self.identity_for(&topic);
            let base_prompt = format!(
                "{}{prompt_note} Cinematic, matches a fast-cut talking-head video. \
                 {duration:.1} seconds, no on-screen text.",
                topic.semantic_topic
            );

            let generation = GenerationRequest::new(
                topic.semantic_topic.clone(),
                base_prompt,
                identity,
                duration,
            );

            tasks.push(RevisionTask {
                id: self.id(),
                action: RevisionAction::GenerateAndInsertBRoll {
                    timestamp: here,
                    duration,
                    semantic_topic: topic.semantic_topic.clone(),
                    generation_prompt: generation.prompt().to_string(),
                    track_index,
                },
                rationale: format!(
                    "Their audience replays {:.0}s–{:.0}s at {:.1}x, and they are looking at a \
                     cutaway. Here the same beat is {hold:.0}s of unbroken talking head.",
                    peak.start_sec,
                    peak.end_sec,
                    peak.intensity
                ),
                evidence: Evidence {
                    competitor_video_id: competitor.video_id.clone(),
                    competitor_time_sec: peak.start_sec,
                    observation: peak.description.clone(),
                },
                // A replay peak is the strongest signal available; scale by how
                // hard the audience rewatched it.
                impact: (0.55 + peak.intensity * 0.12).min(1.0),
                status: TaskStatus::Proposed,
                generation: Some(generation),
            });
        }

        tasks
    }

    /// Rule 2 — they put a sound on this seam, the user's cut is silent.
    fn missing_transitions(
        &self,
        competitor: &CompetitorVideo,
        current: &CurrentTimelineState,
    ) -> Vec<RevisionTask> {
        let mut tasks = Vec::new();

        for cut in current.cut_times() {
            if current.has_audio_event_near(cut, self.settings.sfx_tolerance_sec) {
                continue;
            }

            // Where does this cut sit in their video, and did they sweeten the
            // equivalent seam?
            let ratio = cut / current.duration_sec.max(0.001);
            let their_time = ratio * competitor.duration_sec;
            let Some(sfx) = competitor.audio.transition_near(their_time, 2.5) else {
                continue;
            };

            tasks.push(RevisionTask {
                id: self.id(),
                action: RevisionAction::AddTransitionAudio {
                    timestamp: cut,
                    sfx_type: sfx.sfx_type.clone(),
                },
                rationale: format!(
                    "Your cut at {cut:.1}s is silent. They ride the equivalent seam with a \
                     {} at {:.0} dB below the bed.",
                    sfx.sfx_type,
                    -sfx.gain_db
                ),
                evidence: Evidence {
                    competitor_video_id: competitor.video_id.clone(),
                    competitor_time_sec: sfx.timestamp_sec,
                    observation: format!("{} on the transition", sfx.sfx_type),
                },
                impact: 0.35,
                status: TaskStatus::Proposed,
                generation: None,
            });
        }

        tasks
    }

    /// Rule 3 — they lost the audience here, and this edit has the same flaw.
    ///
    /// A competitor's drop is not by itself a problem for the user; it becomes
    /// one only when the structural cause is reproduced. So each cause is
    /// checked against the timeline before it is raised.
    fn structural_weaknesses(
        &self,
        competitor: &CompetitorVideo,
        current: &CurrentTimelineState,
    ) -> Vec<RevisionTask> {
        let mut tasks = Vec::new();

        for drop in &competitor.retention.drops {
            let here = current.map_narrative_time(drop.start_sec, competitor.duration_sec);
            let hold = current.static_hold_sec(here);

            let reproduced = match drop.cause {
                // Both show up as an over-long unbroken shot.
                DropCause::PacingStall | DropCause::DeadAir => {
                    hold >= self.settings.static_hold_sec
                }
                // Only a problem if nothing covers the shot.
                DropCause::VisualMonotony => {
                    hold >= self.settings.static_hold_sec && !current.has_broll_at(here)
                }
                // The outro rule owns this one.
                DropCause::WeakOutro => false,
                // Not structural; the engine cannot see it in a timeline.
                DropCause::UnmetExpectation | DropCause::Unclassified => false,
            };
            if !reproduced {
                continue;
            }

            tasks.push(RevisionTask {
                id: self.id(),
                action: RevisionAction::FixRetentionDrop {
                    timestamp: here,
                    suggestion: drop.cause.remedy().to_string(),
                },
                rationale: format!(
                    "They lose {:.0}% of the remaining audience across this beat to {}. \
                     Your edit holds one shot for {hold:.0}s at the same point.",
                    drop.severity * 100.0,
                    drop.cause.label()
                ),
                evidence: Evidence {
                    competitor_video_id: competitor.video_id.clone(),
                    competitor_time_sec: drop.start_sec,
                    observation: format!("retention drop — {}", drop.cause.label()),
                },
                // Severity is a measured audience loss; weight it heavily.
                impact: (0.4 + drop.severity).min(1.0),
                status: TaskStatus::Proposed,
                generation: None,
            });
        }

        // Global pacing: a whole edit cut slower than the reference.
        let their_rate = competitor.structure.cuts_per_minute(competitor.duration_sec);
        let our_rate = current.cuts_per_minute();
        if their_rate > 0.0 && our_rate < their_rate * self.settings.pacing_ratio_floor {
            if let Some(worst) = longest_hold(current) {
                tasks.push(RevisionTask {
                    id: self.id(),
                    action: RevisionAction::FixRetentionDrop {
                        timestamp: worst.0,
                        suggestion: format!(
                            "Pacing too slow overall — cut silences and tighten to about \
                             {their_rate:.0} cuts per minute."
                        ),
                    },
                    rationale: format!(
                        "The whole edit runs at {our_rate:.1} cuts per minute against their \
                         {their_rate:.1}. The worst offender is a {:.0}s shot at {:.0}s.",
                        worst.1, worst.0
                    ),
                    evidence: Evidence {
                        competitor_video_id: competitor.video_id.clone(),
                        competitor_time_sec: 0.0,
                        observation: format!("{their_rate:.1} cuts per minute"),
                    },
                    impact: 0.6,
                    status: TaskStatus::Proposed,
                    generation: None,
                });
            }
        }

        tasks
    }

    /// Rule 4 — how it lands.
    fn ending(
        &self,
        competitor: &CompetitorVideo,
        current: &CurrentTimelineState,
    ) -> Option<RevisionTask> {
        let their = &competitor.structure.ending;
        if their.style == EndingStyle::Unknown || current.duration_sec <= 0.0 {
            return None;
        }

        let here = current.map_narrative_time(their.start_sec, competitor.duration_sec);
        let tail = (current.duration_sec - here).max(0.0);

        // Only worth raising when the user's tail is materially longer than the
        // reference's — a short ending needs no advice.
        if tail <= their.tail_sec * 1.25 {
            return None;
        }

        let action = match their.style {
            EndingStyle::NextVideoTease => {
                "Cut to a tease for the next video before the energy drops."
            }
            EndingStyle::LoopBack => "Return to the opening image so the video loops.",
            EndingStyle::HardCut => "Hard cut on the last beat; drop the wind-down.",
            EndingStyle::Outro | EndingStyle::Unknown => "Tighten the sign-off.",
        };

        Some(RevisionTask {
            id: self.id(),
            action: RevisionAction::ReviseEnding {
                start_time: here,
                action: action.to_string(),
            },
            rationale: format!(
                "Their outro is {:.0}s and ends on a {}. Yours runs {tail:.0}s from the same \
                 narrative point.",
                their.tail_sec,
                their.style.label()
            ),
            evidence: Evidence {
                competitor_video_id: competitor.video_id.clone(),
                competitor_time_sec: their.start_sec,
                observation: format!("ending: {}", their.style.label()),
            },
            impact: 0.5,
            status: TaskStatus::Proposed,
            generation: None,
        })
    }

    /// Nearest indexed cutaway topic, by meaning.
    async fn nearest_topic(&self, description: &str) -> Option<BRollPlacement> {
        let hits = self
            .index
            .search(description, SemanticKind::BRollTopic, 1)
            .await
            .ok()?;
        let hit = hits.into_iter().next()?;

        Some(BRollPlacement {
            start_sec: hit.entry.timestamp_sec,
            end_sec: hit.entry.timestamp_sec + self.settings.broll_duration_sec,
            semantic_topic: hit.entry.text,
            features_presenter: false,
        })
    }

    /// Decides the identity mode for a generated shot.
    ///
    /// The constraint can only be honoured with something to match against. If
    /// the competitor's shot has the presenter in it but the user has supplied
    /// no reference, the shot is re-framed to exclude the face rather than
    /// generated as an invented likeness.
    fn identity_for(&self, topic: &BRollPlacement) -> (IdentityMode, &'static str) {
        if !topic.features_presenter {
            return (IdentityMode::NoPresenter, "");
        }

        match &self.settings.presenter_reference {
            Some(reference) if !reference.trim().is_empty() => (
                IdentityMode::PresenterFace {
                    reference_asset: reference.clone(),
                },
                " Presenter in frame, matching the reference exactly.",
            ),
            _ => (
                IdentityMode::NoPresenter,
                " Frame from behind or over the shoulder — no face visible, as no presenter \
                 reference is configured.",
            ),
        }
    }
}

/// The longest unbroken A-roll shot: `(start, duration)`.
fn longest_hold(current: &CurrentTimelineState) -> Option<(f32, f32)> {
    current
        .video_tracks()
        .flat_map(|track| track.clips.iter())
        .filter(|clip| clip.role == crate::ai_tooling::revision::timeline::ClipRole::ARoll)
        .max_by(|a, b| a.duration_sec().total_cmp(&b.duration_sec()))
        .map(|clip| (clip.start_sec, clip.duration_sec()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_tooling::competitor::models::*;
    use crate::ai_tooling::competitor::store::InMemoryWarehouse;
    use crate::ai_tooling::revision::timeline::{ClipRole, ClipView, TrackRole, TrackView};

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("runtime")
    }

    /// A 120s reference: one replay peak at 24s covered by a cutaway, an SFX
    /// on the seam at 24s, a pacing stall at 12s, a next-video tease outro.
    fn competitor() -> CompetitorVideo {
        CompetitorVideo {
            video_id: "ref1".into(),
            channel_id: "UC1".into(),
            title: "the reference".into(),
            view_count: 1_000_000,
            outlier_multiplier: 8.0,
            duration_sec: 120.0,
            published_at: None,
            retention: RetentionAnalysis {
                peaks: vec![HeatmapPeak {
                    start_sec: 24.0,
                    end_sec: 30.0,
                    intensity: 3.0,
                    description: "overhead desk shot while explaining".into(),
                }],
                drops: vec![RetentionDrop {
                    start_sec: 12.0,
                    end_sec: 18.0,
                    severity: 0.2,
                    cause: DropCause::PacingStall,
                }],
                average_view_ratio: 0.5,
            },
            audio: AudioDynamics {
                transitions: vec![TransitionSound {
                    timestamp_sec: 24.0,
                    sfx_type: "whoosh".into(),
                    gain_db: -9.0,
                }],
                ..Default::default()
            },
            structure: VisualAndPacingStructure {
                scene_cuts: (0..40)
                    .map(|i| SceneCut { timestamp_sec: i as f32 * 3.0, score: 0.8 })
                    .collect(),
                broll: vec![BRollPlacement {
                    start_sec: 23.0,
                    end_sec: 29.0,
                    semantic_topic: "overhead desk shot, shallow depth of field".into(),
                    features_presenter: false,
                }],
                ending: VideoEndingAnalysis {
                    start_sec: 110.0,
                    style: EndingStyle::NextVideoTease,
                    tail_sec: 4.0,
                    loops_to_hook: false,
                    call_to_action: None,
                },
                average_shot_sec: 3.0,
            },
            transcript: TranscriptAndHooks::default(),
            analyzed_at: 0,
        }
    }

    /// 60s edit: one 60s static A-roll on V1, an empty V2, an empty A1.
    fn static_edit() -> CurrentTimelineState {
        CurrentTimelineState {
            tracks: vec![
                TrackView {
                    index: 0,
                    name: "V1".into(),
                    role: TrackRole::Video,
                    locked: false,
                    clips: vec![ClipView {
                        id: 1,
                        label: "interview.mp4".into(),
                        start_sec: 0.0,
                        end_sec: 60.0,
                        role: ClipRole::ARoll,
                    }],
                },
                TrackView {
                    index: 1,
                    name: "V2".into(),
                    role: TrackRole::Video,
                    locked: false,
                    clips: Vec::new(),
                },
                TrackView {
                    index: 2,
                    name: "A1".into(),
                    role: TrackRole::Audio,
                    locked: false,
                    clips: Vec::new(),
                },
            ],
            caption_spans: Vec::new(),
            duration_sec: 60.0,
        }
    }

    fn plan_for(current: CurrentTimelineState) -> RevisionPlan {
        runtime().block_on(async {
            let warehouse = InMemoryWarehouse::new();
            let engine = ComparisonEngine::new(&warehouse, DiffSettings::default());
            engine.compare(&competitor(), &current).await
        })
    }

    #[test]
    fn a_replay_peak_over_a_static_shot_becomes_a_broll_task() {
        let plan = plan_for(static_edit());

        let broll = plan
            .tasks
            .iter()
            .find(|t| matches!(t.action, RevisionAction::GenerateAndInsertBRoll { .. }))
            .expect("a b-roll task");

        match &broll.action {
            RevisionAction::GenerateAndInsertBRoll { timestamp, track_index, .. } => {
                // 24s of 120s is 20%, which is 12s of a 60s edit.
                assert!((timestamp - 12.0).abs() < 0.01, "mapped by proportion");
                assert_eq!(*track_index, 1, "the free overlay track");
            }
            other => panic!("wrong action: {other:?}"),
        }
        assert!(broll.generation.is_some(), "the payload is prepared up front");
    }

    #[test]
    fn a_beat_the_user_already_covered_is_not_proposed_again() {
        let mut current = static_edit();
        // Cover 10–16s, which is where the peak maps to.
        current.tracks[1].clips.push(ClipView {
            id: 9,
            label: "cutaway.mp4".into(),
            start_sec: 10.0,
            end_sec: 16.0,
            role: ClipRole::BRoll,
        });

        let plan = plan_for(current);
        assert!(
            !plan
                .tasks
                .iter()
                .any(|t| matches!(t.action, RevisionAction::GenerateAndInsertBRoll { .. })),
            "the engine must not duplicate the user's own work"
        );
    }

    #[test]
    fn a_silent_cut_gets_the_sound_they_used_on_the_same_seam() {
        let mut current = static_edit();
        // Split V1 so there is a real cut at 12s, the peak position.
        current.tracks[0].clips = vec![
            ClipView { id: 1, label: "a".into(), start_sec: 0.0, end_sec: 12.0, role: ClipRole::ARoll },
            ClipView { id: 2, label: "b".into(), start_sec: 12.0, end_sec: 60.0, role: ClipRole::ARoll },
        ];

        let plan = plan_for(current);
        let sfx = plan
            .tasks
            .iter()
            .find(|t| matches!(t.action, RevisionAction::AddTransitionAudio { .. }))
            .expect("an sfx task");

        match &sfx.action {
            RevisionAction::AddTransitionAudio { timestamp, sfx_type } => {
                assert!((timestamp - 12.0).abs() < 0.01);
                assert_eq!(sfx_type, "whoosh");
            }
            other => panic!("wrong action: {other:?}"),
        }
    }

    #[test]
    fn a_cut_that_already_has_a_sound_is_left_alone() {
        let mut current = static_edit();
        current.tracks[0].clips = vec![
            ClipView { id: 1, label: "a".into(), start_sec: 0.0, end_sec: 12.0, role: ClipRole::ARoll },
            ClipView { id: 2, label: "b".into(), start_sec: 12.0, end_sec: 60.0, role: ClipRole::ARoll },
        ];
        current.tracks[2].clips.push(ClipView {
            id: 8,
            label: "whoosh.wav".into(),
            start_sec: 12.1,
            end_sec: 12.7,
            role: ClipRole::Audio,
        });

        let plan = plan_for(current);
        assert!(
            !plan
                .tasks
                .iter()
                .any(|t| matches!(t.action, RevisionAction::AddTransitionAudio { .. })),
            "an existing sfx within tolerance counts as done"
        );
    }

    #[test]
    fn a_competitor_drop_is_only_raised_when_this_edit_reproduces_it() {
        // The static edit holds one shot for 60s, so the stall is reproduced.
        let plan = plan_for(static_edit());
        assert!(
            plan.tasks
                .iter()
                .any(|t| matches!(t.action, RevisionAction::FixRetentionDrop { .. })),
            "the stall is present here too"
        );

        // A fast-cut edit at the same beat has no stall to fix.
        let mut fast = static_edit();
        fast.tracks[0].clips = (0..30)
            .map(|i| ClipView {
                id: i as u64 + 1,
                label: format!("shot{i}"),
                start_sec: i as f32 * 2.0,
                end_sec: i as f32 * 2.0 + 2.0,
                role: ClipRole::ARoll,
            })
            .collect();

        let plan = plan_for(fast);
        let stalls: Vec<_> = plan
            .tasks
            .iter()
            .filter(|t| match &t.action {
                RevisionAction::FixRetentionDrop { suggestion, .. } => {
                    suggestion.contains("Pacing too slow here")
                }
                _ => false,
            })
            .collect();
        assert!(stalls.is_empty(), "nothing to fix in a fast-cut edit");
    }

    #[test]
    fn a_long_tail_gets_an_ending_revision_and_a_short_one_does_not() {
        let plan = plan_for(static_edit());
        // Their outro starts at 110/120 = 91.7%, which is 55s of 60s: a 5s tail
        // against their 4s. 5 <= 4 * 1.25, so nothing is raised.
        assert!(
            !plan
                .tasks
                .iter()
                .any(|t| matches!(t.action, RevisionAction::ReviseEnding { .. })),
            "a tail already close to the reference needs no advice"
        );

        // Stretch the edit so the same narrative point leaves a long tail.
        let mut long = static_edit();
        long.duration_sec = 300.0;
        long.tracks[0].clips[0].end_sec = 300.0;

        let plan = plan_for(long);
        assert!(
            plan.tasks
                .iter()
                .any(|t| matches!(t.action, RevisionAction::ReviseEnding { .. })),
            "a 25s tail against their 4s is worth flagging"
        );
    }

    #[test]
    fn an_empty_timeline_yields_advice_rather_than_a_list_of_impossible_tasks() {
        let plan = plan_for(CurrentTimelineState::default());

        assert!(plan.tasks.is_empty());
        assert!(
            plan.summary.iter().any(|line| line.contains("empty")),
            "the user is told what to do first"
        );
    }

    #[test]
    fn tasks_come_back_most_valuable_first_and_within_the_cap() {
        let plan = plan_for(static_edit());

        assert!(!plan.tasks.is_empty());
        for pair in plan.tasks.windows(2) {
            assert!(pair[0].impact >= pair[1].impact, "sorted by impact");
        }
        assert!(plan.tasks.len() <= DiffSettings::default().max_tasks);
    }

    #[test]
    fn a_generated_shot_of_the_presenter_carries_the_constraint() {
        let mut settings = DiffSettings::default();
        settings.presenter_reference = Some("me.png".into());

        let plan = runtime().block_on(async {
            let warehouse = InMemoryWarehouse::new();
            let mut reference = competitor();
            reference.structure.broll[0].features_presenter = true;

            let engine = ComparisonEngine::new(&warehouse, settings);
            engine.compare(&reference, &static_edit()).await
        });

        let task = plan
            .tasks
            .iter()
            .find(|t| t.generation.is_some())
            .expect("a generation task");
        let request = task.generation.as_ref().expect("payload");

        assert!(matches!(
            request.identity(),
            IdentityMode::PresenterFace { .. }
        ));
        assert!(request.validate().is_ok(), "the constraint is present");
    }

    #[test]
    fn without_a_reference_the_shot_is_reframed_rather_than_inventing_a_face() {
        let plan = runtime().block_on(async {
            let warehouse = InMemoryWarehouse::new();
            let mut reference = competitor();
            reference.structure.broll[0].features_presenter = true;

            // Default settings carry no presenter reference.
            let engine = ComparisonEngine::new(&warehouse, DiffSettings::default());
            engine.compare(&reference, &static_edit()).await
        });

        let request = plan
            .tasks
            .iter()
            .find_map(|t| t.generation.as_ref())
            .expect("a generation task");

        assert_eq!(*request.identity(), IdentityMode::NoPresenter);
        assert!(
            request.prompt().contains("no face visible"),
            "the prompt says why: {}",
            request.prompt()
        );
        assert!(request.validate().is_ok());
    }
}
