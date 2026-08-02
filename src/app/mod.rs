//! Global application shell: state, routing, menu bar and modal overlays.
#![allow(dead_code)]

pub mod ai;
pub mod credentials;
pub mod library;
pub mod menu_bar;
pub mod modals;
pub mod orchestrator;
pub mod router;

use crate::ui::components::status::ServiceState;
use crate::workspace::ProjectSummary;
use crate::ui::theme;
use crate::views;
use crate::views::studio::persistence;
use eframe::egui;
use modals::Modals;
use orchestrator::{AppCommand, AppEvent, OrchestratorHandle};
use std::sync::Arc;
use router::{AppRoute, ProjectId, RouteHistory};

/// A project the user can open — always backed by a folder on disk.
#[derive(Clone)]
pub struct Project {
    /// Stable hash of `disk_id`, so routes stay `Copy`.
    pub id: ProjectId,
    /// Folder name under the projects root.
    pub disk_id: String,
    pub name: String,
    pub platform: String,
    pub blueprint: String,
    pub duration: String,
    pub modified: String,
    pub clips: usize,
}

impl Project {
    pub fn from_summary(summary: &ProjectSummary) -> Self {
        let seconds = summary.duration_seconds.max(0.0) as i64;
        Self {
            id: library::route_id(&summary.id),
            disk_id: summary.id.clone(),
            name: summary.name.clone(),
            platform: platform_label(summary.target).to_string(),
            blueprint: summary.blueprint.clone().unwrap_or_else(|| "—".into()),
            duration: format!(
                "{:02}:{:02}:{:02}",
                seconds / 3600,
                (seconds % 3600) / 60,
                seconds % 60
            ),
            modified: relative_time(summary.modified_at),
            clips: summary.clip_count,
        }
    }
}

fn platform_label(target: crate::workspace::TargetPlatform) -> &'static str {
    use crate::workspace::TargetPlatform as T;
    match target {
        T::YouTubeLandscape => "YouTube 16:9",
        T::YouTubeShorts => "Shorts 9:16",
        T::TikTok => "TikTok 9:16",
        T::InstagramReels => "Reels 9:16",
        T::Custom { .. } => "Custom",
    }
}

fn relative_time(unix_seconds: u64) -> String {
    let now = crate::workspace::models::now_unix();
    let delta = now.saturating_sub(unix_seconds);
    match delta {
        0..=59 => "just now".to_string(),
        60..=3599 => format!("{} min ago", delta / 60),
        3600..=86_399 => format!("{} hours ago", delta / 3600),
        86_400..=172_799 => "yesterday".to_string(),
        _ => format!("{} days ago", delta / 86_400),
    }
}

pub struct AiDirectorApp {
    pub route: AppRoute,
    pub history: RouteHistory,
    pub modals: Modals,
    pub library: library::ProjectLibrary,
    pub services: Vec<(&'static str, ServiceState)>,
    pub time: f32,
    pub gallery_open: bool,
    pub gallery: views::gallery::GalleryState,
    pub dashboard: views::dashboard::DashboardState,
    pub onboarding: views::onboarding::OnboardingState,
    pub studio: views::studio::StudioState,
    pub growth: views::growth::GrowthState,
    /// Channel research. Owns its own runtime, so it is built lazily — the
    /// screen is optional and most sessions never open it.
    pub insights: Option<views::insights::InsightsState>,
    pub ai: ai::AiBridge,
    /// Credentials form — the only place API keys are entered.
    pub settings: views::settings::SettingsState,
    /// The one background hub. `None` only if a Tokio runtime could not be
    /// created, which leaves the app usable for everything synchronous.
    pub orchestrator: Option<OrchestratorHandle>,
    /// Last snapshot published to the worker, so an unchanged frame does
    /// not take the write lock.
    published_clips: Option<(usize, f32)>,
    /// When the open project first differed from what is on disk.
    dirty_since: Option<f32>,
}

impl AiDirectorApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        let ai = ai::AiBridge::new();

        // The worker wakes the UI through this rather than holding an
        // `egui::Context` itself, which keeps the orchestrator testable
        // without a window.
        let ctx = cc.egui_ctx.clone();
        let orchestrator = OrchestratorHandle::spawn(Arc::new(move || ctx.request_repaint()));
        let director_state = if ai.is_online() {
            ServiceState::Online
        } else {
            ServiceState::Error
        };
        Self {
            route: AppRoute::Dashboard,
            history: RouteHistory::default(),
            modals: Modals::default(),
            library: library::ProjectLibrary::new(),
            services: vec![
                ("Audio Engine Online", ServiceState::Online),
                ("ComfyUI Rendering…", ServiceState::Working),
                ("Whisper ASR Online", ServiceState::Online),
                ("LLM Director", director_state),
            ],
            time: 0.0,
            gallery_open: false,
            gallery: Default::default(),
            dashboard: Default::default(),
            onboarding: Default::default(),
            studio: Default::default(),
            growth: Default::default(),
            insights: None,
            ai,
            settings: Default::default(),
            orchestrator,
            published_clips: None,
            dirty_since: None,
        }
    }

    fn project(&self, id: ProjectId) -> Option<&Project> {
        self.library.find(id)
    }

    /// Reconnects the assistant with freshly saved keys. Reading them straight
    /// from the form avoids mutating the process environment, which is unsound
    /// once worker threads are running.
    fn apply_credentials(&mut self) {
        let credentials = self.settings.draft.clone();
        self.studio.chat.reconnect(&credentials.to_config());

        let director = if credentials.assistant_ready() {
            ServiceState::Online
        } else {
            ServiceState::Error
        };
        if let Some(entry) = self.services.iter_mut().find(|(name, _)| *name == "LLM Director") {
            entry.1 = director;
        }
    }


    /// Publishes the timeline to the worker, when it has actually changed.
    ///
    /// Cheap guard on clip count and duration rather than a deep compare: the
    /// snapshot is rebuilt and the write lock taken only when the edit moved,
    /// so an idle frame costs nothing.
    fn publish_timeline(&mut self) {
        let Some(orchestrator) = &self.orchestrator else {
            return;
        };
        let fingerprint = (
            self.studio.timeline.clip_count(),
            self.studio.timeline.content_end(),
        );
        if self.published_clips == Some(fingerprint) {
            return;
        }
        self.published_clips = Some(fingerprint);
        orchestrator
            .timeline
            .publish(views::studio::revisions::snapshot(&self.studio.timeline));
    }

    /// Hands queued work to the orchestrator. Runs after the views have drawn,
    /// so a command raised this frame leaves this frame.
    fn pump_commands(&mut self) {
        let Some(orchestrator) = &mut self.orchestrator else {
            return;
        };

        // Heavy work the dispatcher deferred. Before this the receiver was
        // never read: the assistant was told its render was queued and the
        // job evaporated.
        for deferred in self.studio.take_async_jobs() {
            use crate::ai_tooling::orchestration::AsyncJob;
            match deferred {
                AsyncJob::RenderBroll { clip_id, prompt, .. } => {
                    orchestrator.dispatch(|job| AppCommand::RenderBroll { job, clip_id, prompt })
                }
                AsyncJob::Export { preset } => {
                    orchestrator.dispatch(|job| AppCommand::ExecuteTimelineExport { job, preset })
                }
            };
        }

        for task in self.studio.revisions.take_approved() {
            orchestrator.dispatch(|job| AppCommand::ApproveRevisionTask {
                job,
                task: Box::new(task.clone()),
            });
        }

        if let Some(insights) = &mut self.insights {
            for request in insights.take_requests() {
                use views::insights::InsightsRequest as R;
                orchestrator.dispatch(|job| match request {
                    R::CheckPrerequisites => AppCommand::CheckPrerequisites { job },
                    R::AnalyzeChannel(channel_id) => {
                        AppCommand::StartCompetitorAnalysis { job, channel_id }
                    }
                    R::Deconstruct { score, channel_id, duration_sec } => {
                        AppCommand::DeconstructVideo { job, score, channel_id, duration_sec }
                    }
                    R::MeasurePacing(video_id) => AppCommand::MeasurePacing { job, video_id },
                    R::ComparePlan { video_id, use_llm, presenter_reference } => {
                        AppCommand::GenerateRevisionPlan {
                            job,
                            video_id,
                            use_llm,
                            presenter_reference,
                        }
                    }
                });
            }
        }
    }

    /// Routes one frame's worth of events.
    ///
    /// Every mutation of editor state in the whole background architecture
    /// happens here, on the UI thread, one event at a time.
    fn drain_events(&mut self) {
        let Some(orchestrator) = &mut self.orchestrator else {
            return;
        };

        for event in orchestrator.drain() {
            // The research screen absorbs its own results and hands back a
            // finished plan, which is the only thing it cannot apply itself.
            if let Some(insights) = &mut self.insights {
                if let Some(plan) = insights.apply_event(&event) {
                    self.studio.revisions.adopt(plan);
                    self.studio.show_revisions = true;
                }
            }

            match event {
                AppEvent::AssetGenerated { asset, .. } => {
                    // Ahead of the commands that place it: the dispatcher
                    // refuses an asset it cannot resolve in the pool.
                    views::studio::register_asset(&mut self.studio, &asset);
                }
                AppEvent::ApplyActions { task_id, commands, note, .. } => {
                    let report = self.studio.apply_actions(commands);
                    if report.had_failures() {
                        self.studio.revisions.fail(task_id, report.feedback());
                    } else {
                        self.studio.revisions.settle(task_id);
                        self.studio.revisions.status = Some(note);
                        // The edit moved, so the worker's view of it is stale.
                        self.published_clips = None;
                    }
                }
                AppEvent::TaskFailed { task_id, reason, .. } => {
                    self.studio.revisions.fail(task_id, reason);
                }
                AppEvent::ExportFinished { path, .. } => {
                    self.growth.export_status = Some(format!("Wrote {}", path.display()));
                }
                AppEvent::Error { ref message, .. } => {
                    // Route it to the screen that owns the job. The research
                    // view absorbs its own errors in `apply_event`; an export
                    // failure belongs on the export form; anything with no
                    // home becomes a modal rather than vanishing.
                    let claimed_by_insights = self
                        .insights
                        .as_ref()
                        .is_some_and(|i| i.error.as_deref() == Some(message.as_str()));
                    if claimed_by_insights {
                        // already shown there
                    } else if matches!(self.route, AppRoute::Growth(_)) {
                        self.growth.export_status = Some(message.clone());
                    } else {
                        self.modals.info("Background job failed", message);
                    }
                }
                _ => {}
            }
        }
    }

    /// Writes the open project immediately, ignoring the autosave debounce.
    fn flush_open_project(&mut self) {
        let Some(open) = &self.library.open else {
            return;
        };
        if open.needs_apply {
            return;
        }
        let context = open.context.clone();
        let save = library::ProjectSave {
            timeline: persistence::to_snapshot(&self.studio.timeline, &context),
            chat: persistence::chat_to_disk(self.studio.chat.history()),
        };
        self.dirty_since = None;
        self.library.save_open(save);
    }

    /// Loads a freshly opened project into the studio, then autosaves changes.
    ///
    /// Saving is debounced: an edit is written once the timeline has been quiet
    /// for a moment, so dragging a clip does not queue a write per frame.
    fn sync_open_project(&mut self) {
        const AUTOSAVE_DEBOUNCE: f32 = 1.5;

        let Some(open) = &mut self.library.open else {
            return;
        };
        let context = open.context.clone();

        if open.needs_apply {
            open.needs_apply = false;
            let timeline = open.config.timeline.clone();
            let chat = open.saved_chat.clone();
            persistence::apply_snapshot(&mut self.studio.timeline, &timeline, &context);
            self.studio.chat.restore(persistence::chat_from_disk(&chat));
            self.dirty_since = None;
            return;
        }

        let timeline = persistence::to_snapshot(&self.studio.timeline, &context);
        let chat = persistence::chat_to_disk(self.studio.chat.history());
        let changed = timeline != open.saved_timeline || chat != open.saved_chat;

        match (changed, self.dirty_since) {
            (true, None) => self.dirty_since = Some(self.time),
            (true, Some(since)) if self.time - since >= AUTOSAVE_DEBOUNCE => {
                self.dirty_since = None;
                self.library
                    .save_open(library::ProjectSave { timeline, chat });
            }
            (false, Some(_)) => self.dirty_since = None,
            _ => {}
        }
    }
}

impl eframe::App for AiDirectorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let now = ctx.input(|i| i.time) as f32;
        let dt = (now - self.time).clamp(0.0, 0.1);
        self.time = now;
        ctx.request_repaint_after(std::time::Duration::from_millis(33));

        // 0. Drain background services before any view reads their state.
        //    Order matters: events land first so the frame draws the
        //    newest state, then the fresh timeline is published for the
        //    jobs queued later in this same frame.
        self.ai.poll();
        self.library.poll(&mut self.route);
        self.drain_events();
        self.sync_open_project();
        self.publish_timeline();

        // 1. Chrome first, so every route sits under the same menu bar.
        let project_name = self
            .route
            .project()
            .and_then(|id| self.project(id))
            .map(|p| p.name.clone());
        // The status pill used to show a hardcoded "ComfyUI Rendering…"
        // whatever was actually happening. It now reports the orchestrator.
        let busy = self
            .orchestrator
            .as_ref()
            .map(|o| o.jobs.active_count())
            .unwrap_or(0);
        let service = match busy {
            0 => ("Idle", ServiceState::Online),
            1 => ("1 job running", ServiceState::Working),
            _ => ("Jobs running", ServiceState::Working),
        };

        menu_bar::show(
            ctx,
            &mut menu_bar::MenuBarCtx {
                route: &mut self.route,
                history: &mut self.history,
                modals: &mut self.modals,
                gallery_open: &mut self.gallery_open,
                project_name: project_name.as_deref(),
                service,
                time: self.time,
            },
        );

        // 2. Route to the active view. Views mutate `route` to navigate.
        match self.route {
            AppRoute::Dashboard => {
                if views::dashboard::show(
                    ctx,
                    &mut self.route,
                    &mut self.dashboard,
                    &self.library.projects,
                    self.library.error.as_deref(),
                    &mut self.modals,
                ) {
                    self.settings.open();
                }
            }
            AppRoute::Onboarding => views::onboarding::show(
                ctx,
                &mut self.route,
                &mut self.onboarding,
                &mut self.library,
            ),
            AppRoute::Studio(id) => {
                // Switching projects: flush the outgoing one first, or its
                // pending edits would be overwritten by the incoming load.
                let switching = self.library.find(id).map(|p| p.disk_id.clone())
                    != self.library.open.as_ref().map(|o| o.config.id.clone());
                if switching {
                    self.flush_open_project();
                }
                self.library.request_open(id);
                views::studio::show(
                    ctx,
                    &mut self.route,
                    &mut self.studio,
                    &mut self.modals,
                    self.library.find(id),
                    self.time,
                )
            }
            AppRoute::Insights => {
                let project_open = self.library.open.is_some();
                let state = self.insights.get_or_insert_with(Default::default);
                let outcome = views::insights::show(ctx, &mut self.route, state, project_open);

                // The view cannot reach the editor or the worker. It
                // queues a request; `pump_commands` dispatches it, and the
                // plan comes back through `drain_events`.
                if let Some(video_id) = outcome.compare_with_studio {
                    state.compare(&video_id);
                }
            }
            AppRoute::Growth(id) => {
                if let Some(preset) = views::growth::show(
                    ctx,
                    &mut self.route,
                    &mut self.growth,
                    &mut self.modals,
                    self.library.find(id),
                ) {
                    self.growth.export_status = Some("Queued...".into());
                    if let Some(orchestrator) = &mut self.orchestrator {
                        orchestrator
                            .dispatch(|job| AppCommand::ExecuteTimelineExport { job, preset });
                    }
                }
            }
        }

        // 3. Optional dev surface, then overlays on top of everything.
        if views::settings::show(ctx, &mut self.settings)
            == Some(views::settings::SettingsOutcome::Saved)
        {
            self.apply_credentials();
        }
        views::gallery::window(ctx, &mut self.gallery_open, &mut self.gallery, self.time);
        modals::show(ctx, &mut self.modals, &mut self.route, dt);

        // 4. Anything the views queued this frame leaves this frame.
        self.pump_commands();
        self.history.track(self.route);
    }
}
