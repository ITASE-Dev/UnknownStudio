//! Everything the UI can ask the background to do.
//!
//! One enum, one direction. A command is a *request*, never a mutation: the UI
//! thread hands one over and forgets about it, and whatever comes back arrives
//! later as an [`AppEvent`](super::event::AppEvent).

use crate::ai_tooling::revision::models::RevisionTask;
use crate::ai_tooling::youtube_insights::ViralScore;

/// Correlates a command with the events it produces.
///
/// Needed because several jobs can be in flight at once: two videos being
/// deconstructed while an export runs. Without it the UI could only guess which
/// progress event belongs to which spinner.
pub type JobId = u64;

#[derive(Debug, Clone)]
pub enum AppCommand {
    /// Sample a channel's uploads and score them against its own baseline.
    StartCompetitorAnalysis {
        job: JobId,
        channel_id: String,
    },

    /// Take one outlier apart into the warehouse.
    DeconstructVideo {
        job: JobId,
        score: Box<ViralScore>,
        channel_id: String,
        duration_sec: f32,
    },

    /// Measure a video's rhythm from its caption track.
    MeasurePacing {
        job: JobId,
        video_id: String,
    },

    /// Diff a deconstructed competitor against the live timeline.
    ///
    /// The timeline is not passed: the orchestrator reads the shared snapshot,
    /// so the plan is always built against what is on screen *now* rather than
    /// whatever was there when the button was pressed.
    GenerateRevisionPlan {
        job: JobId,
        video_id: String,
        /// Route through the three-agent LLM pipeline instead of the offline
        /// rule engine.
        use_llm: bool,
        presenter_reference: Option<String>,
    },

    /// Run one approved revision: generate its asset, then hand back the
    /// commands that apply it.
    ApproveRevisionTask {
        job: JobId,
        task: Box<RevisionTask>,
    },

    /// Render a cutaway for an existing clip.
    ///
    /// Raised by the dispatcher when the chat assistant asks for B-roll,
    /// which it defers rather than doing inline. Goes through the same
    /// prompt engineer — and therefore the same facial identity
    /// constraint — as a revision task.
    RenderBroll {
        job: JobId,
        clip_id: String,
        prompt: String,
    },

    /// Encode the programme.
    ExecuteTimelineExport {
        job: JobId,
        preset: String,
    },

    /// Check what the analysis features need: API keys, and yt-dlp on PATH.
    ///
    /// A background job because probing yt-dlp spawns a process, which is
    /// tens of milliseconds the render thread should not spend.
    CheckPrerequisites { job: JobId },

    /// Stop the worker. Sent on drop; also lets a test end the loop.
    Shutdown,
}

impl AppCommand {
    /// The job this belongs to, for routing progress back to the right place.
    pub fn job(&self) -> Option<JobId> {
        match self {
            Self::StartCompetitorAnalysis { job, .. }
            | Self::DeconstructVideo { job, .. }
            | Self::MeasurePacing { job, .. }
            | Self::GenerateRevisionPlan { job, .. }
            | Self::ApproveRevisionTask { job, .. }
            | Self::RenderBroll { job, .. }
            | Self::ExecuteTimelineExport { job, .. }
            | Self::CheckPrerequisites { job } => Some(*job),
            Self::Shutdown => None,
        }
    }

    /// Short name for progress messages and logs.
    pub fn label(&self) -> &'static str {
        match self {
            Self::StartCompetitorAnalysis { .. } => "channel analysis",
            Self::DeconstructVideo { .. } => "deconstruction",
            Self::MeasurePacing { .. } => "pacing",
            Self::GenerateRevisionPlan { .. } => "revision plan",
            Self::ApproveRevisionTask { .. } => "revision",
            Self::RenderBroll { .. } => "b-roll render",
            Self::ExecuteTimelineExport { .. } => "export",
            Self::CheckPrerequisites { .. } => "prerequisites",
            Self::Shutdown => "shutdown",
        }
    }

    /// Whether this may run alongside others.
    ///
    /// Everything is concurrent except the export, which saturates the machine
    /// and would make every other job look hung.
    pub fn is_concurrent(&self) -> bool {
        !matches!(self, Self::ExecuteTimelineExport { .. } | Self::Shutdown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_working_command_carries_a_job_id() {
        let commands = [
            AppCommand::StartCompetitorAnalysis { job: 1, channel_id: "UC".into() },
            AppCommand::MeasurePacing { job: 2, video_id: "v".into() },
            AppCommand::GenerateRevisionPlan {
                job: 3,
                video_id: "v".into(),
                use_llm: false,
                presenter_reference: None,
            },
            AppCommand::ExecuteTimelineExport { job: 4, preset: "p".into() },
            AppCommand::RenderBroll {
                job: 5,
                clip_id: "c1".into(),
                prompt: "a shot".into(),
            },
        ];

        for command in commands {
            assert!(
                command.job().is_some(),
                "{} has no job to report against",
                command.label()
            );
        }
        assert_eq!(AppCommand::Shutdown.job(), None);
    }

    #[test]
    fn the_export_is_the_only_thing_that_runs_alone() {
        assert!(!AppCommand::ExecuteTimelineExport { job: 1, preset: "p".into() }.is_concurrent());
        assert!(AppCommand::StartCompetitorAnalysis { job: 1, channel_id: "UC".into() }.is_concurrent());
        assert!(AppCommand::MeasurePacing { job: 1, video_id: "v".into() }.is_concurrent());
    }
}
