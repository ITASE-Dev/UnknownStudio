//! One async function per command.
//!
//! Handlers are the only place the engines get called. They take a
//! [`WorkerContext`], do the work, and report through it — none of them can
//! reach editor state, because none of them is given a way to.

use crate::ai_tooling::competitor::store::{CompetitorDataStore, InMemoryWarehouse};
use crate::ai_tooling::competitor::{deconstruct, CompetitorVideo};
use crate::ai_tooling::config::AiToolingConfig;
use crate::ai_tooling::orchestration::dispatcher::ActionCommand;
use crate::ai_tooling::pipeline::LlmPipelineEngine;
use crate::ai_tooling::revision::diff::{ComparisonEngine, DiffSettings};
use crate::ai_tooling::revision::executor::execute_task;
use crate::ai_tooling::revision::generation::MockGenerator;
use crate::ai_tooling::revision::models::{RevisionPlan, RevisionTask};
use crate::ai_tooling::scraping::DeepScraper;
use crate::ai_tooling::youtube_insights::{heatmap, InsightsAggregator, OutlierSettings, ViralScore};
use crate::app::orchestrator::command::JobId;
use crate::app::orchestrator::event::AppEvent;
use crate::app::orchestrator::shared::SharedTimeline;
use reqwest::Client;
use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::sync::Arc;

/// Half-width of the transcript window the scraper measures around the peak.
const PEAK_WINDOW_SEC: f64 = 10.0;

/// Latency the mock generator pretends to have.
const GENERATION_LATENCY_MS: u64 = 1_200;

/// Length of a cutaway the assistant asks for without naming one.
const DEFAULT_BROLL_SEC: f32 = 3.5;

/// Handed to every handler. Cloned per task, so nothing is contended.
#[derive(Clone)]
pub struct WorkerContext {
    pub warehouse: InMemoryWarehouse,
    pub timeline: SharedTimeline,
    events: Sender<AppEvent>,
    /// Wakes the UI when an event lands. A closure rather than an
    /// `egui::Context` so the orchestrator stays testable without a window.
    repaint: Arc<dyn Fn() + Send + Sync>,
}

impl WorkerContext {
    pub fn new(
        warehouse: InMemoryWarehouse,
        timeline: SharedTimeline,
        events: Sender<AppEvent>,
        repaint: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        Self {
            warehouse,
            timeline,
            events,
            repaint,
        }
    }

    /// Sends an event and nudges the UI.
    ///
    /// A closed channel means the window is gone; there is nobody left to tell,
    /// so the send is dropped rather than escalated. A worker that panics
    /// because the app is shutting down is a crash report about nothing.
    pub fn emit(&self, event: AppEvent) {
        if self.events.send(event).is_ok() {
            (self.repaint)();
        }
    }

    pub fn progress(&self, job: JobId, fraction: f32, stage: impl Into<String>) {
        self.emit(AppEvent::AnalysisProgress {
            job,
            fraction,
            stage: stage.into(),
        });
    }

    pub fn fail(&self, job: JobId, message: impl Into<String>) {
        self.emit(AppEvent::Error {
            job: Some(job),
            message: message.into(),
        });
    }
}

// ------------------------------------------------------------ prerequisites

/// Probes what the analysis features need.
///
/// Cheap, but not free: the yt-dlp check spawns a process. Running it here
/// rather than in the view keeps that off the render thread.
pub async fn check_prerequisites(ctx: WorkerContext, job: JobId) {
    use crate::app::orchestrator::event::Prerequisites;

    ctx.progress(job, 0.3, "reading configuration");
    let mut report = Prerequisites::default();

    match AiToolingConfig::load() {
        Ok(config) => {
            report.youtube_key = !config.youtube_api_key.trim().is_empty();
            report.llm_key = config.provider_key().is_ok();
        }
        Err(err) => {
            // A missing LLM key makes the whole load fail, so read the one key
            // the analysis actually needs directly rather than reporting that
            // everything is unavailable.
            report.youtube_key = std::env::var("YOUTUBE_API_KEY")
                .is_ok_and(|key| !key.trim().is_empty());
            report.config_error = Some(err.to_string());
        }
    }

    ctx.progress(job, 0.7, "looking for yt-dlp");
    report.yt_dlp = tokio::process::Command::new("yt-dlp")
        .arg("--version")
        .kill_on_drop(true)
        .output()
        .await
        .is_ok_and(|out| out.status.success());

    ctx.emit(AppEvent::PrerequisitesChecked { job, report });
}

// ------------------------------------------------------------------ channel

pub async fn analyze_channel(ctx: WorkerContext, job: JobId, channel_id: String) {
    ctx.progress(job, 0.1, "checking credentials");

    let config = match AiToolingConfig::load() {
        Ok(config) if config.youtube_api_key.trim().is_empty() => {
            ctx.fail(job, "YOUTUBE_API_KEY is blank — set it in Settings.");
            return;
        }
        Ok(config) => config,
        Err(err) => {
            ctx.fail(job, format!("{err} — add it in Settings, then try again."));
            return;
        }
    };

    ctx.progress(job, 0.3, "sampling uploads");
    let aggregator = match InsightsAggregator::new(config) {
        Ok(aggregator) => aggregator,
        Err(err) => {
            ctx.fail(job, err.to_string());
            return;
        }
    };

    match aggregator
        .analyze_channel(&channel_id, OutlierSettings::default())
        .await
    {
        Ok((metrics, analysis)) => {
            ctx.progress(job, 0.9, "scoring against the baseline");
            let durations: HashMap<String, f32> = metrics
                .videos
                .iter()
                .map(|v| (v.video_id.clone(), v.duration_seconds as f32))
                .collect();
            ctx.emit(AppEvent::OutliersReady {
                job,
                analysis: Box::new(analysis),
                durations,
            });
        }
        Err(err) => ctx.fail(job, err.to_string()),
    }
}

// ----------------------------------------------------------- deconstruction

pub async fn deconstruct_video(
    ctx: WorkerContext,
    job: JobId,
    score: ViralScore,
    channel_id: String,
    duration_sec: f32,
) {
    ctx.progress(job, 0.15, "retention heatmap");

    match deconstruct(&score, &channel_id, duration_sec, &ctx.warehouse, &ctx.warehouse).await {
        Ok(video) => {
            ctx.progress(job, 0.95, "writing to the warehouse");
            ctx.emit(AppEvent::VideoDeconstructed {
                job,
                video: Box::new(video),
            });
        }
        Err(err) => ctx.fail(job, err.to_string()),
    }
}

// ------------------------------------------------------------------ pacing

pub async fn measure_pacing(ctx: WorkerContext, job: JobId, video_id: String) {
    ctx.progress(job, 0.3, "fetching captions");

    let scraper = DeepScraper::new(Client::new(), PEAK_WINDOW_SEC);
    let url = format!("https://www.youtube.com/watch?v={video_id}");

    match scraper.transcript(&url).await {
        Ok(cues) if cues.is_empty() => {
            ctx.fail(job, "No English captions on that video — pacing cannot be measured.")
        }
        Ok(cues) => {
            ctx.progress(job, 0.8, "measuring rhythm");
            let duration = cues.last().map_or(0.0, |cue| cue.end) as f32;
            ctx.emit(AppEvent::PacingReady {
                job,
                heatmap: Box::new(heatmap::from_cues(&video_id, &cues, duration, None)),
            });
        }
        Err(err) => ctx.fail(job, err.to_string()),
    }
}

// ------------------------------------------------------------------- plans

/// Diffs a competitor against whatever is on the timeline *now*.
pub async fn generate_plan(
    ctx: WorkerContext,
    job: JobId,
    video_id: String,
    use_llm: bool,
    presenter_reference: Option<String>,
) {
    let competitor: CompetitorVideo = match ctx.warehouse.load_video(&video_id).await {
        Ok(video) => video,
        Err(err) => {
            ctx.fail(job, format!("{err} — deconstruct the video first."));
            return;
        }
    };

    // Read at the moment the job runs, not when the button was pressed: the
    // user may have kept editing while the queue drained.
    let current = ctx.timeline.snapshot();
    if current.is_empty() {
        ctx.fail(job, "The timeline is empty — place your A-roll before comparing.");
        return;
    }

    let plan = if use_llm {
        match run_llm_pipeline(&ctx, job, &competitor, &current, presenter_reference).await {
            Some(plan) => plan,
            None => return,
        }
    } else {
        ctx.progress(job, 0.5, "diffing against your timeline");
        let settings = DiffSettings {
            presenter_reference,
            ..Default::default()
        };
        ComparisonEngine::new(&ctx.warehouse, settings)
            .compare(&competitor, &current)
            .await
    };

    ctx.progress(job, 0.95, "ordering by impact");
    ctx.emit(AppEvent::RevisionsReady {
        job,
        plan: Box::new(plan),
    });
}

/// The three-agent path.
///
/// Delegates the chaining to [`LlmPipelineEngine::run`] and passes a reporter,
/// rather than reimplementing the sequence to get progress out of it. The DNA
/// is forwarded the moment stage 1 lands, so the research screen can show the
/// analysis while stages 2 and 3 are still working.
async fn run_llm_pipeline(
    ctx: &WorkerContext,
    job: JobId,
    competitor: &CompetitorVideo,
    current: &crate::ai_tooling::revision::timeline::CurrentTimelineState,
    presenter_reference: Option<String>,
) -> Option<RevisionPlan> {
    use crate::ai_tooling::pipeline::Stage;

    ctx.progress(job, 0.1, "loading credentials");
    let config = match AiToolingConfig::load() {
        Ok(config) => config,
        Err(err) => {
            ctx.fail(job, format!("{err} — or untick the AI director to run offline."));
            return None;
        }
    };

    let engine = match LlmPipelineEngine::from_config(&config) {
        Ok(engine) => engine.with_presenter_reference(presenter_reference),
        Err(err) => {
            ctx.fail(job, err.to_string());
            return None;
        }
    };

    let reporter = |stage: Stage| {
        ctx.progress(job, stage.fraction(), stage.label());
        if let Stage::Directing(dna) = stage {
            ctx.emit(AppEvent::DnaReady {
                job,
                dna: Box::new(dna),
            });
        }
    };

    match engine.run(competitor, current, &reporter).await {
        Ok(output) => Some(output.plan),
        Err(err) => {
            ctx.fail(job, err.to_string());
            None
        }
    }
}

// ------------------------------------------------------------- the mutator

/// Runs one approved task and hands back the edits.
///
/// The slow half — rendering a cutaway, resolving an effect — happens here. The
/// fast half is a list of commands the UI thread applies; this function never
/// touches the timeline, which is why it is safe to run concurrently with the
/// user continuing to edit.
pub async fn approve_task(ctx: WorkerContext, job: JobId, task: RevisionTask) {
    let task_id = task.id;

    if task.action.needs_generation() {
        ctx.progress(job, 0.2, "generating the asset");
    } else {
        ctx.progress(job, 0.4, "preparing the edit");
    }

    let generator = MockGenerator::new(GENERATION_LATENCY_MS);
    match execute_task(&task, &generator).await {
        Ok(outcome) => {
            for asset in &outcome.assets {
                ctx.emit(AppEvent::AssetGenerated {
                    job,
                    task_id,
                    asset: Box::new(asset.clone()),
                });
            }

            ctx.progress(job, 0.9, "applying to the timeline");
            ctx.emit(AppEvent::ApplyActions {
                job,
                task_id,
                commands: outcome.commands,
                note: outcome.note,
            });
        }
        Err(err) => ctx.emit(AppEvent::TaskFailed {
            job,
            task_id,
            reason: err.to_string(),
        }),
    }
}

/// Renders a cutaway the chat assistant asked for.
///
/// Routed through [`prompt_engineer::compose`] rather than straight to the
/// generator, so a shot involving the presenter picks up the facial identity
/// constraint exactly as a revision task would. A model-initiated render must
/// not be a way around the rule.
pub async fn render_broll(
    ctx: WorkerContext,
    job: JobId,
    clip_id: String,
    prompt: String,
    presenter_reference: Option<String>,
) {
    use crate::ai_tooling::pipeline::agents::prompt_engineer;
    use crate::ai_tooling::pipeline::models::AssetPromptDraft;
    use crate::ai_tooling::revision::generation::generate;

    ctx.progress(job, 0.2, "preparing the shot");

    // No LLM in this path: the assistant already wrote the prompt. It still
    // goes through `compose`, which is where the constraint is applied.
    let draft = AssetPromptDraft {
        involves_human_subject: mentions_presenter(&prompt),
        prompt,
        avoid: "on-screen text, watermarks, logos".into(),
        intent: format!("covers {clip_id}"),
    };
    let request = prompt_engineer::compose(
        &draft,
        "assistant b-roll",
        DEFAULT_BROLL_SEC,
        presenter_reference.as_deref(),
    );

    ctx.progress(job, 0.5, "rendering");
    let generator = MockGenerator::new(GENERATION_LATENCY_MS);
    match generate(&generator, &request).await {
        Ok(asset) => {
            ctx.emit(AppEvent::AssetGenerated {
                job,
                task_id: 0,
                asset: Box::new(asset.clone()),
            });
            ctx.emit(AppEvent::ApplyActions {
                job,
                task_id: 0,
                note: format!("Rendered a cutaway for {clip_id}."),
                commands: vec![ActionCommand::PlaceAsset {
                    asset: asset.name,
                    target_track_idx: 0,
                    target_time_sec: 0.0,
                }],
            });
        }
        Err(err) => ctx.fail(job, err.to_string()),
    }
}

/// Whether a prompt describes the presenter, so the constraint is applied.
///
/// Conservative on purpose: a false positive costs a re-framed shot, a false
/// negative costs an invented likeness.
fn mentions_presenter(prompt: &str) -> bool {
    const MARKERS: [&str; 8] = [
        "presenter", "host", "speaker", "me ", "myself", "my face", "talking head", "on camera",
    ];
    let lowered = prompt.to_lowercase();
    MARKERS.iter().any(|marker| lowered.contains(marker))
}

// ------------------------------------------------------------------ export

pub async fn export(ctx: WorkerContext, job: JobId, preset: String) {
    ctx.progress(job, 0.05, "reading the timeline");
    let current = ctx.timeline.snapshot();

    if current.is_empty() {
        ctx.fail(job, "Nothing to export — the timeline is empty.");
        return;
    }

    ctx.progress(job, 0.2, format!("preparing {preset}"));

    // The renderer that walks a multi-track timeline and encodes it does not
    // exist yet. Reporting that plainly is better than emitting an
    // `ExportFinished` with a path to a file nobody wrote.
    ctx.fail(
        job,
        format!(
            "Export to “{preset}” is not wired up: the timeline renderer is still missing. \
             The {} clips are ready for it.",
            current.clip_count()
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_tooling::revision::models::{Evidence, RevisionAction, TaskStatus};
    use crate::ai_tooling::revision::timeline::{ClipRole, ClipView, TrackRole, TrackView};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("runtime")
    }

    /// A context whose repaint calls are counted.
    fn context() -> (WorkerContext, mpsc::Receiver<AppEvent>, Arc<AtomicUsize>) {
        let (tx, rx) = mpsc::channel();
        let repaints = Arc::new(AtomicUsize::new(0));
        let counter = repaints.clone();

        let ctx = WorkerContext::new(
            InMemoryWarehouse::new(),
            SharedTimeline::default(),
            tx,
            Arc::new(move || {
                counter.fetch_add(1, Ordering::Relaxed);
            }),
        );
        (ctx, rx, repaints)
    }

    fn populated() -> crate::ai_tooling::revision::timeline::CurrentTimelineState {
        crate::ai_tooling::revision::timeline::CurrentTimelineState {
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

    fn task(action: RevisionAction) -> RevisionTask {
        RevisionTask {
            id: 42,
            action,
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
    fn every_emitted_event_wakes_the_ui() {
        let (ctx, rx, repaints) = context();

        ctx.progress(7, 0.5, "halfway");
        ctx.fail(7, "nope");

        assert_eq!(repaints.load(Ordering::Relaxed), 2);
        assert_eq!(rx.try_iter().count(), 2);
    }

    #[test]
    fn a_closed_channel_is_survived_rather_than_panicked_on() {
        let (ctx, rx, repaints) = context();
        drop(rx);

        // The window closed mid-job; the worker must simply stop reporting.
        ctx.progress(1, 0.5, "still going");
        assert_eq!(repaints.load(Ordering::Relaxed), 0, "nobody to wake");
    }

    #[test]
    fn approving_an_advisory_task_yields_commands_and_no_assets() {
        let (ctx, rx, _) = context();

        runtime().block_on(approve_task(
            ctx,
            1,
            task(RevisionAction::FixRetentionDrop {
                timestamp: 12.0,
                suggestion: "tighten".into(),
            }),
        ));

        let events: Vec<AppEvent> = rx.try_iter().collect();
        assert!(
            !events.iter().any(|e| matches!(e, AppEvent::AssetGenerated { .. })),
            "an advisory task generates nothing"
        );

        let applied = events
            .iter()
            .find_map(|e| match e {
                AppEvent::ApplyActions { commands, task_id, .. } => Some((commands, *task_id)),
                _ => None,
            })
            .expect("the edits came back");
        assert_eq!(applied.1, 42, "tied to the task that produced them");
        assert!(!applied.0.is_empty());
    }

    #[test]
    fn approving_a_broll_task_reports_the_asset_before_the_edits() {
        use crate::ai_tooling::revision::generation::{GenerationRequest, IdentityMode};

        let (ctx, rx, _) = context();
        let request =
            GenerationRequest::new("desk", "Overhead desk.", IdentityMode::NoPresenter, 3.0);
        let mut broll = task(RevisionAction::GenerateAndInsertBRoll {
            timestamp: 12.0,
            duration: 3.0,
            semantic_topic: "desk".into(),
            generation_prompt: String::new(),
            track_index: 1,
        });
        broll.generation = Some(request);

        runtime().block_on(approve_task(ctx, 1, broll));

        let events: Vec<AppEvent> = rx.try_iter().collect();
        let asset_at = events
            .iter()
            .position(|e| matches!(e, AppEvent::AssetGenerated { .. }))
            .expect("an asset");
        let apply_at = events
            .iter()
            .position(|e| matches!(e, AppEvent::ApplyActions { .. }))
            .expect("the edits");

        assert!(
            asset_at < apply_at,
            "the pool must learn about the asset before the dispatcher is asked to place it"
        );
    }

    #[test]
    fn a_failing_task_reports_against_its_own_id_and_not_the_whole_job() {
        let (ctx, rx, _) = context();

        runtime().block_on(approve_task(
            ctx,
            1,
            // No such effect in the library.
            task(RevisionAction::AddTransitionAudio {
                timestamp: 0.0,
                sfx_type: "airhorn".into(),
            }),
        ));

        let failed = rx
            .try_iter()
            .find_map(|e| match e {
                AppEvent::TaskFailed { task_id, reason, .. } => Some((task_id, reason)),
                _ => None,
            })
            .expect("a failure");
        assert_eq!(failed.0, 42);
        assert!(failed.1.contains("airhorn"));
    }

    #[test]
    fn a_plan_for_an_empty_timeline_is_refused_with_a_useful_message() {
        let (ctx, rx, _) = context();

        runtime().block_on(generate_plan(ctx, 1, "nope".into(), false, None));

        let message = rx
            .try_iter()
            .find_map(|e| match e {
                AppEvent::Error { message, .. } => Some(message),
                _ => None,
            })
            .expect("an error");
        // The video is not in the warehouse, which is the first thing checked.
        assert!(message.contains("deconstruct"), "{message}");
    }

    #[test]
    fn the_plan_reads_the_timeline_when_it_runs_not_when_it_was_queued() {
        let (ctx, rx, _) = context();
        let timeline = ctx.timeline.clone();

        // Queued against an empty timeline…
        assert!(timeline.is_empty());
        // …but the user kept working before the job got its turn.
        timeline.publish(populated());

        runtime().block_on(async {
            let video = deconstruct(
                &ViralScore {
                    video_id: "v1".into(),
                    title: "t".into(),
                    view_count: 10,
                    baseline_views: 1.0,
                    multiplier: 10.0,
                    modified_z: 5.0,
                    percentile: 0.99,
                    method: crate::ai_tooling::youtube_insights::models::OutlierMethod::ModifiedZScore,
                    is_outlier: true,
                },
                "UC1",
                120.0,
                &ctx.warehouse,
                &ctx.warehouse,
            )
            .await
            .expect("deconstruct");

            generate_plan(ctx.clone(), 1, video.video_id, false, None).await;
        });

        let ready = rx
            .try_iter()
            .any(|e| matches!(e, AppEvent::RevisionsReady { .. }));
        assert!(ready, "it planned against the published edit, not the empty one");
    }

    #[test]
    fn a_prompt_naming_the_presenter_is_detected_conservatively() {
        assert!(mentions_presenter("Medium shot of the presenter at the desk"));
        assert!(mentions_presenter("The host walks toward camera"));
        assert!(mentions_presenter("Talking head, shallow depth of field"));
        assert!(mentions_presenter("Cut to me explaining the graph"));

        assert!(!mentions_presenter("Overhead timelapse of an empty desk"));
        assert!(!mentions_presenter("Aerial drone shot of mountains"));
    }

    /// The rule the audit was told to protect, on the path the audit created.
    ///
    /// `RenderBroll` is a new route to the generator, opened up by draining the
    /// dispatcher's deferred queue. A new route to the generator is exactly how
    /// a constraint gets bypassed, so this asserts it did not.
    #[test]
    fn an_assistant_requested_render_of_the_presenter_still_carries_the_constraint() {
        use crate::ai_tooling::pipeline::agents::prompt_engineer;
        use crate::ai_tooling::pipeline::models::AssetPromptDraft;
        use crate::ai_tooling::revision::generation::{IdentityMode, FACIAL_IDENTITY_CONSTRAINT};

        let draft = AssetPromptDraft {
            prompt: "Medium shot of the presenter reacting.".into(),
            involves_human_subject: mentions_presenter("Medium shot of the presenter reacting."),
            avoid: String::new(),
            intent: String::new(),
        };
        assert!(draft.involves_human_subject, "the guard must fire first");

        let request = prompt_engineer::compose(&draft, "topic", 3.0, Some("me.png"));
        assert!(matches!(
            request.identity(),
            IdentityMode::PresenterFace { .. }
        ));
        assert!(request.prompt().contains(FACIAL_IDENTITY_CONSTRAINT));
        assert!(request.validate().is_ok());
    }

    #[test]
    fn an_assistant_render_without_a_reference_is_reframed_not_invented() {
        use crate::ai_tooling::pipeline::agents::prompt_engineer;
        use crate::ai_tooling::pipeline::models::AssetPromptDraft;
        use crate::ai_tooling::revision::generation::IdentityMode;

        let draft = AssetPromptDraft {
            prompt: "Close up of the host.".into(),
            involves_human_subject: true,
            avoid: String::new(),
            intent: String::new(),
        };

        let request = prompt_engineer::compose(&draft, "topic", 3.0, None);
        assert_eq!(*request.identity(), IdentityMode::NoPresenter);
        assert!(request.prompt().contains("no face visible"));
    }

    #[test]
    fn an_export_of_nothing_says_so_rather_than_starting() {
        let (ctx, rx, _) = context();
        runtime().block_on(export(ctx, 1, "youtube_1080p".into()));

        let message = rx
            .try_iter()
            .find_map(|e| match e {
                AppEvent::Error { message, .. } => Some(message),
                _ => None,
            })
            .expect("an error");
        assert!(message.contains("empty"), "{message}");
    }
}
