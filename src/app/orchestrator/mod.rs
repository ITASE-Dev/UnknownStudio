//! The Grand Orchestrator — one background hub for every long-running job.
//!
//! # Topology
//!
//! ```text
//!   ┌─ UI thread (egui, synchronous) ──────────────────────────────┐
//!   │  owns TimelineState, MediaState, every view's local state    │
//!   │                                                              │
//!   │   send(AppCommand) ──── tokio unbounded ────▶                │
//!   │   ◀─── std::sync::mpsc ──── AppEvent                         │
//!   │   publish(snapshot) ──▶ Arc<RwLock<CurrentTimelineState>>    │
//!   └──────────────────────────────────────────────────────────────┘
//!                                    │
//!   ┌─ Tokio multi-thread runtime ───▼─────────────────────────────┐
//!   │  run loop: recv command → spawn handler → handler emits      │
//!   │  handlers read the snapshot; none of them can write it       │
//!   └──────────────────────────────────────────────────────────────┘
//! ```
//!
//! Two channel kinds, on purpose:
//!
//! - **UI → worker** is `tokio::sync::mpsc::unbounded`. Its `send` is not
//!   `async`, so the render thread can call it mid-frame, and the worker can
//!   `await` on `recv`. A bounded channel would let a full queue block the
//!   render thread, which is the one thing this design must never do.
//! - **worker → UI** is `std::sync::mpsc`. The UI polls `try_recv` in a drain
//!   loop once per frame — the natural shape for an immediate-mode loop, and it
//!   needs no runtime context to read.
//!
//! # Why no lock on the live timeline
//!
//! egui reads the timeline dozens of times per frame. Putting it behind a lock
//! shared with workers would mean a handler holding the write guard across an
//! LLM or FFmpeg call stalls the renderer — the freeze this whole architecture
//! exists to prevent. So the live state stays on the UI thread, the *read-only
//! projection* is shared, and mutations come back as
//! [`AppEvent::ApplyActions`] to be applied by the dispatcher on the UI thread.
//!
//! # Deadlock
//!
//! There is no lock ordering to get wrong: one lock, held only across a move
//! or a clone, never across an `.await` (enforced by [`SharedTimeline`]
//! returning owned values rather than guards). The worker never blocks on the
//! UI, and the UI never blocks on the worker.

pub mod command;
pub mod event;
pub mod handlers;
pub mod shared;

pub use command::{AppCommand, JobId};
pub use event::{AppEvent, JobTracker, Prerequisites, Progress};
pub use shared::SharedTimeline;

use crate::ai_tooling::competitor::store::InMemoryWarehouse;
use handlers::WorkerContext;
use std::sync::mpsc::{self as std_mpsc, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::sync::mpsc::{self as tokio_mpsc, UnboundedSender};

/// Events drained per frame.
///
/// A burst — a plan landing with a dozen tasks — should not cost a dropped
/// frame, and anything left over is picked up next frame a few milliseconds
/// later. Unbounded draining would let a runaway producer stall the renderer.
const MAX_EVENTS_PER_FRAME: usize = 64;

/// The UI's end of the orchestrator.
///
/// Holds the runtime: dropping the handle shuts the worker down. Not `Clone` —
/// there is one hub, and a second `Receiver` would silently split the event
/// stream so half the results went nowhere.
pub struct OrchestratorHandle {
    commands: UnboundedSender<AppCommand>,
    events: Receiver<AppEvent>,
    pub jobs: JobTracker,
    /// Shared read-only projection of the timeline.
    pub timeline: SharedTimeline,
    /// The competitor warehouse, shared with the worker.
    pub warehouse: InMemoryWarehouse,
    /// Dropped last; shutting the runtime down aborts anything still running.
    runtime: Option<Runtime>,
}

impl OrchestratorHandle {
    /// Starts the worker. `repaint` is called whenever an event is emitted.
    ///
    /// Returns `None` if a Tokio runtime cannot be created, which the caller
    /// should surface rather than treat as fatal: the app is still usable for
    /// everything that does not need a background job.
    pub fn spawn(repaint: Arc<dyn Fn() + Send + Sync>) -> Option<Self> {
        let runtime = Runtime::new().ok()?;

        let (commands_tx, commands_rx) = tokio_mpsc::unbounded_channel();
        let (events_tx, events_rx) = std_mpsc::channel();

        let timeline = SharedTimeline::default();
        let warehouse = InMemoryWarehouse::new();
        let ctx = WorkerContext::new(warehouse.clone(), timeline.clone(), events_tx, repaint);

        runtime.spawn(run(commands_rx, ctx));

        Some(Self {
            commands: commands_tx,
            events: events_rx,
            jobs: JobTracker::default(),
            timeline,
            warehouse,
            runtime: Some(runtime),
        })
    }

    /// Queues a command and returns its job id.
    ///
    /// Never blocks. A failed send means the worker is gone — during shutdown,
    /// or after a runtime failure — and the job simply never starts, so the
    /// tracker is not told about it.
    pub fn dispatch(&mut self, make: impl FnOnce(JobId) -> AppCommand) -> Option<JobId> {
        let job = self.jobs.next_job();
        let command = make(job);
        let what = command.label();

        if self.commands.send(command).is_err() {
            return None;
        }
        self.jobs.observe(&AppEvent::Started { job, what });
        Some(job)
    }

    /// Drains up to [`MAX_EVENTS_PER_FRAME`] events, folding progress into the
    /// tracker as it goes. Call once per frame; never blocks.
    pub fn drain(&mut self) -> Vec<AppEvent> {
        let mut batch = Vec::new();

        for _ in 0..MAX_EVENTS_PER_FRAME {
            match self.events.try_recv() {
                Ok(event) => {
                    self.jobs.observe(&event);
                    batch.push(event);
                }
                // Disconnected means the worker stopped; nothing more will
                // arrive, and that is not an error worth reporting per frame.
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }

        batch
    }

    pub fn is_busy(&self) -> bool {
        self.jobs.is_busy()
    }
}

impl Drop for OrchestratorHandle {
    fn drop(&mut self) {
        // Ask politely, then take the runtime down. `shutdown_background`
        // rather than a plain drop: dropping a runtime blocks until every task
        // finishes, which on a hung HTTP call means the window will not close.
        let _ = self.commands.send(AppCommand::Shutdown);
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

/// The run loop.
///
/// Each command is spawned rather than awaited, so a slow channel analysis does
/// not hold up an approval behind it. The exception is the export, which
/// saturates the machine — running it concurrently would make everything else
/// look hung, so it is awaited in place.
async fn run(mut commands: tokio_mpsc::UnboundedReceiver<AppCommand>, ctx: WorkerContext) {
    while let Some(command) = commands.recv().await {
        if matches!(command, AppCommand::Shutdown) {
            break;
        }

        let concurrent = command.is_concurrent();
        let ctx = ctx.clone();
        let work = handle(ctx, command);

        if concurrent {
            tokio::spawn(work);
        } else {
            work.await;
        }
    }
}

/// Routes one command, and guarantees its terminal event.
///
/// Every path emits `Finished`, including the ones that bail early, so a
/// spinner can never be orphaned by a `return` inside a handler.
async fn handle(ctx: WorkerContext, command: AppCommand) {
    let Some(job) = command.job() else {
        return;
    };

    match command {
        AppCommand::StartCompetitorAnalysis { channel_id, .. } => {
            handlers::analyze_channel(ctx.clone(), job, channel_id).await
        }
        AppCommand::DeconstructVideo {
            score,
            channel_id,
            duration_sec,
            ..
        } => handlers::deconstruct_video(ctx.clone(), job, *score, channel_id, duration_sec).await,
        AppCommand::MeasurePacing { video_id, .. } => {
            handlers::measure_pacing(ctx.clone(), job, video_id).await
        }
        AppCommand::GenerateRevisionPlan {
            video_id,
            use_llm,
            presenter_reference,
            ..
        } => {
            handlers::generate_plan(ctx.clone(), job, video_id, use_llm, presenter_reference).await
        }
        AppCommand::ApproveRevisionTask { task, .. } => {
            handlers::approve_task(ctx.clone(), job, *task).await
        }
        AppCommand::CheckPrerequisites { .. } => {
            handlers::check_prerequisites(ctx.clone(), job).await
        }
        AppCommand::RenderBroll { clip_id, prompt, .. } => {
            handlers::render_broll(ctx.clone(), job, clip_id, prompt, None).await
        }
        AppCommand::ExecuteTimelineExport { preset, .. } => {
            handlers::export(ctx.clone(), job, preset).await
        }
        AppCommand::Shutdown => return,
    }

    ctx.emit(AppEvent::Finished { job });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_tooling::revision::models::{Evidence, RevisionAction, RevisionTask, TaskStatus};
    use crate::ai_tooling::revision::timeline::{
        ClipRole, ClipView, CurrentTimelineState, TrackRole, TrackView,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    fn handle_with_counter() -> (OrchestratorHandle, Arc<AtomicUsize>) {
        let repaints = Arc::new(AtomicUsize::new(0));
        let counter = repaints.clone();
        let handle = OrchestratorHandle::spawn(Arc::new(move || {
            counter.fetch_add(1, Ordering::Relaxed);
        }))
        .expect("runtime");
        (handle, repaints)
    }

    /// Polls like the UI does, until `predicate` holds or the deadline passes.
    fn pump(
        handle: &mut OrchestratorHandle,
        timeout: Duration,
        mut predicate: impl FnMut(&[AppEvent]) -> bool,
    ) -> Vec<AppEvent> {
        let deadline = Instant::now() + timeout;
        let mut seen = Vec::new();

        while Instant::now() < deadline {
            seen.extend(handle.drain());
            if predicate(&seen) {
                return seen;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        seen
    }

    fn populated() -> CurrentTimelineState {
        CurrentTimelineState {
            tracks: vec![TrackView {
                index: 0,
                name: "V1".into(),
                role: TrackRole::Video,
                locked: false,
                clips: vec![ClipView {
                    id: 1,
                    label: "a.mp4".into(),
                    start_sec: 0.0,
                    end_sec: 60.0,
                    role: ClipRole::ARoll,
                }],
            }],
            caption_spans: Vec::new(),
            duration_sec: 60.0,
        }
    }

    fn advisory_task(id: u64) -> RevisionTask {
        RevisionTask {
            id,
            action: RevisionAction::FixRetentionDrop {
                timestamp: 12.0,
                suggestion: "tighten".into(),
            },
            rationale: String::new(),
            evidence: Evidence {
                competitor_video_id: "v".into(),
                competitor_time_sec: 0.0,
                observation: String::new(),
            },
            impact: 0.5,
            status: TaskStatus::Approved,
            generation: None,
        }
    }

    #[test]
    fn dispatching_never_blocks_the_calling_thread() {
        let (mut handle, _) = handle_with_counter();

        // Queue more work than the worker can possibly have started.
        let start = Instant::now();
        for _ in 0..200 {
            handle.dispatch(|job| AppCommand::ApproveRevisionTask {
                job,
                task: Box::new(advisory_task(1)),
            });
        }
        let elapsed = start.elapsed();

        assert!(
            elapsed < Duration::from_millis(250),
            "200 dispatches took {elapsed:?} — the render thread would have dropped frames"
        );
    }

    #[test]
    fn an_approved_task_comes_back_as_commands_for_the_ui_to_apply() {
        let (mut handle, _) = handle_with_counter();
        let job = handle
            .dispatch(|job| AppCommand::ApproveRevisionTask {
                job,
                task: Box::new(advisory_task(7)),
            })
            .expect("queued");

        let events = pump(&mut handle, Duration::from_secs(5), |seen| {
            seen.iter().any(|e| matches!(e, AppEvent::Finished { .. }))
        });

        let applied = events
            .iter()
            .find_map(|e| match e {
                AppEvent::ApplyActions { task_id, commands, .. } => Some((*task_id, commands.len())),
                _ => None,
            })
            .expect("the edits came back");

        assert_eq!(applied.0, 7);
        assert!(applied.1 > 0);
        assert!(events.iter().any(|e| e.job() == Some(job)));
    }

    #[test]
    fn every_job_ends_with_finished_so_no_spinner_is_orphaned() {
        let (mut handle, _) = handle_with_counter();

        // This one fails: the effect is not in the library.
        handle.dispatch(|job| AppCommand::ApproveRevisionTask {
            job,
            task: Box::new(RevisionTask {
                action: RevisionAction::AddTransitionAudio {
                    timestamp: 0.0,
                    sfx_type: "airhorn".into(),
                },
                ..advisory_task(3)
            }),
        });

        assert!(handle.is_busy(), "tracked from the moment it was queued");

        pump(&mut handle, Duration::from_secs(5), |seen| {
            seen.iter().any(|e| matches!(e, AppEvent::Finished { .. }))
        });

        assert!(!handle.is_busy(), "a failed job still clears its spinner");
    }

    #[test]
    fn independent_jobs_run_concurrently_rather_than_queueing() {
        let (mut handle, _) = handle_with_counter();

        // Each B-roll generation sleeps ~1.2s in the mock. Three in series
        // would be 3.6s; concurrently they overlap.
        let start = Instant::now();
        for id in 1..=3 {
            handle.dispatch(|job| AppCommand::ApproveRevisionTask {
                job,
                task: Box::new(RevisionTask {
                    action: RevisionAction::GenerateAndInsertBRoll {
                        timestamp: 0.0,
                        duration: 2.0,
                        semantic_topic: format!("shot {id}"),
                        generation_prompt: String::new(),
                        track_index: 0,
                    },
                    generation: Some(
                        crate::ai_tooling::revision::generation::GenerationRequest::new(
                            format!("shot {id}"),
                            "a prompt",
                            crate::ai_tooling::revision::generation::IdentityMode::NoPresenter,
                            2.0,
                        ),
                    ),
                    ..advisory_task(id)
                }),
            });
        }

        pump(&mut handle, Duration::from_secs(10), |seen| {
            seen.iter()
                .filter(|e| matches!(e, AppEvent::Finished { .. }))
                .count()
                == 3
        });
        let elapsed = start.elapsed();

        assert!(
            elapsed < Duration::from_millis(3_000),
            "three 1.2s jobs took {elapsed:?} — they ran in series"
        );
        assert!(!handle.is_busy());
    }

    #[test]
    fn the_worker_sees_the_snapshot_the_ui_published() {
        let (handle, _) = handle_with_counter();

        handle.timeline.publish(populated());
        assert_eq!(handle.timeline.snapshot().duration_sec, 60.0);
        assert!(!handle.timeline.is_empty());
    }

    #[test]
    fn an_event_wakes_the_ui_so_a_result_is_never_left_sitting() {
        let (mut handle, repaints) = handle_with_counter();
        handle.dispatch(|job| AppCommand::ApproveRevisionTask {
            job,
            task: Box::new(advisory_task(1)),
        });

        pump(&mut handle, Duration::from_secs(5), |seen| {
            seen.iter().any(|e| matches!(e, AppEvent::Finished { .. }))
        });

        assert!(
            repaints.load(Ordering::Relaxed) > 0,
            "without a repaint the UI would sit idle holding a finished job"
        );
    }

    #[test]
    fn a_burst_is_drained_across_frames_rather_than_in_one() {
        let (mut handle, _) = handle_with_counter();
        for _ in 0..(MAX_EVENTS_PER_FRAME * 2) {
            handle.dispatch(|job| AppCommand::ApproveRevisionTask {
                job,
                task: Box::new(advisory_task(1)),
            });
        }

        // Let the worker get ahead, then check one frame's drain is capped.
        std::thread::sleep(Duration::from_millis(300));
        assert!(handle.drain().len() <= MAX_EVENTS_PER_FRAME);
    }

    #[test]
    fn dropping_the_handle_returns_promptly_even_with_work_in_flight() {
        let (mut handle, _) = handle_with_counter();
        for id in 1..=4 {
            handle.dispatch(|job| AppCommand::ApproveRevisionTask {
                job,
                task: Box::new(RevisionTask {
                    generation: Some(
                        crate::ai_tooling::revision::generation::GenerationRequest::new(
                            "shot",
                            "a prompt",
                            crate::ai_tooling::revision::generation::IdentityMode::NoPresenter,
                            2.0,
                        ),
                    ),
                    action: RevisionAction::GenerateAndInsertBRoll {
                        timestamp: 0.0,
                        duration: 2.0,
                        semantic_topic: "shot".into(),
                        generation_prompt: String::new(),
                        track_index: 0,
                    },
                    ..advisory_task(id)
                }),
            });
        }

        // Closing the window must not wait for a 1.2s render, let alone a
        // hung HTTP call.
        let start = Instant::now();
        drop(handle);
        assert!(
            start.elapsed() < Duration::from_millis(500),
            "shutdown blocked for {:?}",
            start.elapsed()
        );
    }
}
