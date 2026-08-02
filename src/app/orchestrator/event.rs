//! Everything the background can tell the UI.
//!
//! Events are facts, not requests. The UI decides what to do with them; the
//! worker never assumes a particular screen is open, or that anyone is looking.

use crate::ai_tooling::competitor::models::CompetitorVideo;
use crate::ai_tooling::orchestration::dispatcher::ActionCommand;
use crate::ai_tooling::pipeline::CompetitorDNA;
use crate::ai_tooling::revision::generation::GeneratedAsset;
use crate::ai_tooling::revision::models::RevisionPlan;
use crate::ai_tooling::youtube_insights::{OutlierAnalysis, PacingHeatmap};
use crate::app::orchestrator::command::JobId;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum AppEvent {
    /// A job started. The UI shows a spinner from here.
    Started { job: JobId, what: &'static str },

    /// Fraction complete, 0.0..=1.0, and what is happening.
    ///
    /// Emitted at stage boundaries rather than continuously: a progress bar
    /// that advances in six honest steps beats one that animates a guess.
    AnalysisProgress {
        job: JobId,
        fraction: f32,
        stage: String,
    },

    /// A channel's uploads, scored. Durations come along because the outlier
    /// scores do not carry them and the next stage needs them.
    OutliersReady {
        job: JobId,
        analysis: Box<OutlierAnalysis>,
        durations: HashMap<String, f32>,
    },

    /// One competitor video, fully taken apart and in the warehouse.
    VideoDeconstructed {
        job: JobId,
        video: Box<CompetitorVideo>,
    },

    /// The LLM's reading of a competitor video. Only the AI path produces one.
    DnaReady {
        job: JobId,
        dna: Box<CompetitorDNA>,
    },

    PacingReady {
        job: JobId,
        heatmap: Box<PacingHeatmap>,
    },

    /// A finished edit plan, ready for the to-do list.
    RevisionsReady {
        job: JobId,
        plan: Box<RevisionPlan>,
    },

    /// An asset finished rendering. Register it in the media pool.
    AssetGenerated {
        job: JobId,
        task_id: u64,
        asset: Box<GeneratedAsset>,
    },

    /// The mutation itself.
    ///
    /// The worker cannot touch the timeline, so it sends the edits instead.
    /// The UI thread feeds them to the dispatcher — the same path a chat
    /// command takes — which is where a ghost becomes a real clip.
    ApplyActions {
        job: JobId,
        task_id: u64,
        commands: Vec<ActionCommand>,
        /// One line for the status area.
        note: String,
    },

    /// One task failed. The rest of the plan is unaffected.
    TaskFailed {
        job: JobId,
        task_id: u64,
        reason: String,
    },

    /// The programme was encoded.
    ExportFinished {
        job: JobId,
        path: PathBuf,
    },

    /// What the analysis features can and cannot do right now.
    PrerequisitesChecked { job: JobId, report: Prerequisites },

    /// A job ended, successfully or not. Always sent, so a spinner cannot be
    /// orphaned by an early return somewhere in a handler.
    Finished { job: JobId },

    /// Something went wrong that is not tied to one task.
    Error { job: Option<JobId>, message: String },
}

impl AppEvent {
    pub fn job(&self) -> Option<JobId> {
        match self {
            Self::Started { job, .. }
            | Self::AnalysisProgress { job, .. }
            | Self::OutliersReady { job, .. }
            | Self::VideoDeconstructed { job, .. }
            | Self::DnaReady { job, .. }
            | Self::PacingReady { job, .. }
            | Self::RevisionsReady { job, .. }
            | Self::AssetGenerated { job, .. }
            | Self::ApplyActions { job, .. }
            | Self::TaskFailed { job, .. }
            | Self::ExportFinished { job, .. }
            | Self::PrerequisitesChecked { job, .. }
            | Self::Finished { job } => Some(*job),
            Self::Error { job, .. } => *job,
        }
    }

    /// Whether this ends a job, so the UI can clear its spinner.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Finished { .. })
    }
}

/// What each analysis feature needs, and whether it has it.
///
/// The point is a screen that explains why a button will not work *before* it
/// is pressed, instead of a failure message after.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Prerequisites {
    /// `YOUTUBE_API_KEY` — channel sampling and outlier scoring.
    pub youtube_key: bool,
    /// The selected provider's key — the AI director only.
    pub llm_key: bool,
    /// `yt-dlp` on PATH — caption fetching, so pacing measurement.
    pub yt_dlp: bool,
    /// Why the config failed to load, when it did.
    pub config_error: Option<String>,
}

impl Prerequisites {
    /// Whether a channel can be examined at all.
    pub fn can_analyze(&self) -> bool {
        self.youtube_key
    }

    /// Everything blocking, as one line each.
    pub fn blockers(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.youtube_key {
            out.push("YOUTUBE_API_KEY is not set — channel analysis cannot run.".into());
        }
        if !self.yt_dlp {
            out.push("yt-dlp is not on PATH — pacing measurement cannot fetch captions.".into());
        }
        if !self.llm_key {
            out.push("No LLM key — the AI director is unavailable; the offline engine still works.".into());
        }
        out
    }
}

/// Live jobs and how far along they are.
///
/// Kept UI-side. The worker is stateless about progress — it reports, it does
/// not remember — so a dropped event costs a stale bar, never a wedged one:
/// `Finished` clears the entry whatever happened in between.
#[derive(Debug, Default)]
pub struct JobTracker {
    active: HashMap<JobId, Progress>,
    next: JobId,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    pub what: String,
    pub fraction: f32,
    pub stage: String,
}

impl JobTracker {
    /// Next id. Ids are never reused, so a late event from a finished job
    /// cannot resurrect a spinner belonging to a new one.
    pub fn next_job(&mut self) -> JobId {
        self.next += 1;
        self.next
    }

    pub fn is_busy(&self) -> bool {
        !self.active.is_empty()
    }

    pub fn active_count(&self) -> usize {
        self.active.len()
    }

    pub fn get(&self, job: JobId) -> Option<&Progress> {
        self.active.get(&job)
    }

    /// Every live job, for a status strip.
    pub fn iter(&self) -> impl Iterator<Item = (&JobId, &Progress)> {
        self.active.iter()
    }

    /// Folds an event into the tracker. Returns the event so callers can chain.
    pub fn observe(&mut self, event: &AppEvent) {
        match event {
            AppEvent::Started { job, what } => {
                self.active.insert(
                    *job,
                    Progress {
                        what: (*what).to_string(),
                        fraction: 0.0,
                        stage: "starting".into(),
                    },
                );
            }
            AppEvent::AnalysisProgress { job, fraction, stage } => {
                // An update for an unknown job means `Started` was lost; take
                // the progress as the start rather than dropping it.
                let entry = self.active.entry(*job).or_insert_with(|| Progress {
                    what: "working".into(),
                    fraction: 0.0,
                    stage: String::new(),
                });
                entry.fraction = fraction.clamp(0.0, 1.0);
                entry.stage = stage.clone();
            }
            AppEvent::Finished { job } => {
                self.active.remove(job);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_setup_blocks_nothing() {
        let ready = Prerequisites {
            youtube_key: true,
            llm_key: true,
            yt_dlp: true,
            config_error: None,
        };
        assert!(ready.can_analyze());
        assert!(ready.blockers().is_empty());
    }

    #[test]
    fn each_missing_piece_names_itself_and_what_it_costs() {
        let bare = Prerequisites::default();
        assert!(!bare.can_analyze());

        let blockers = bare.blockers().join(" ");
        assert!(blockers.contains("YOUTUBE_API_KEY"));
        assert!(blockers.contains("yt-dlp"));
        assert!(blockers.contains("LLM key"));
    }

    #[test]
    fn a_missing_llm_key_does_not_block_the_analysis_itself() {
        // The offline diff engine needs no model; saying otherwise would send
        // the user hunting for a key they do not need.
        let report = Prerequisites {
            youtube_key: true,
            llm_key: false,
            yt_dlp: true,
            config_error: None,
        };
        assert!(report.can_analyze());
        assert_eq!(report.blockers().len(), 1);
        assert!(report.blockers()[0].contains("offline engine still works"));
    }

    #[test]
    fn a_job_appears_on_start_and_is_gone_on_finish() {
        let mut tracker = JobTracker::default();
        let job = tracker.next_job();

        tracker.observe(&AppEvent::Started { job, what: "export" });
        assert!(tracker.is_busy());
        assert_eq!(tracker.get(job).map(|p| p.what.as_str()), Some("export"));

        tracker.observe(&AppEvent::Finished { job });
        assert!(!tracker.is_busy(), "the spinner must not outlive the job");
    }

    #[test]
    fn a_failure_still_clears_the_spinner_because_finished_always_follows() {
        let mut tracker = JobTracker::default();
        let job = tracker.next_job();

        tracker.observe(&AppEvent::Started { job, what: "revision" });
        tracker.observe(&AppEvent::TaskFailed {
            job,
            task_id: 1,
            reason: "boom".into(),
        });
        assert!(tracker.is_busy(), "a failed task is not a finished job");

        tracker.observe(&AppEvent::Finished { job });
        assert!(!tracker.is_busy());
    }

    #[test]
    fn progress_is_clamped_so_a_bad_fraction_cannot_overflow_a_bar() {
        let mut tracker = JobTracker::default();
        let job = tracker.next_job();
        tracker.observe(&AppEvent::Started { job, what: "x" });

        tracker.observe(&AppEvent::AnalysisProgress {
            job,
            fraction: 4.5,
            stage: "wat".into(),
        });
        assert_eq!(tracker.get(job).expect("progress").fraction, 1.0);
    }

    #[test]
    fn progress_for_an_unseen_job_starts_it_rather_than_being_dropped() {
        let mut tracker = JobTracker::default();

        tracker.observe(&AppEvent::AnalysisProgress {
            job: 99,
            fraction: 0.5,
            stage: "halfway".into(),
        });
        assert_eq!(tracker.get(99).map(|p| p.fraction), Some(0.5));
    }

    #[test]
    fn ids_are_never_reused_so_a_late_event_cannot_hit_a_new_job() {
        let mut tracker = JobTracker::default();
        let first = tracker.next_job();
        tracker.observe(&AppEvent::Started { job: first, what: "a" });
        tracker.observe(&AppEvent::Finished { job: first });

        let second = tracker.next_job();
        assert_ne!(first, second);
    }

    #[test]
    fn several_jobs_are_tracked_independently() {
        let mut tracker = JobTracker::default();
        let (a, b) = (tracker.next_job(), tracker.next_job());

        tracker.observe(&AppEvent::Started { job: a, what: "a" });
        tracker.observe(&AppEvent::Started { job: b, what: "b" });
        assert_eq!(tracker.active_count(), 2);

        tracker.observe(&AppEvent::Finished { job: a });
        assert_eq!(tracker.active_count(), 1);
        assert!(tracker.get(b).is_some());
    }

    #[test]
    fn every_event_reports_the_job_it_belongs_to() {
        let events = [
            AppEvent::Started { job: 1, what: "x" },
            AppEvent::Finished { job: 1 },
            AppEvent::TaskFailed { job: 1, task_id: 2, reason: String::new() },
            AppEvent::ApplyActions {
                job: 1,
                task_id: 2,
                commands: Vec::new(),
                note: String::new(),
            },
        ];
        for event in events {
            assert_eq!(event.job(), Some(1));
        }

        // Only a free-floating error may have none.
        assert_eq!(
            AppEvent::Error { job: None, message: String::new() }.job(),
            None
        );
    }
}
