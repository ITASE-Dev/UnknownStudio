//! Global application shell: state, routing, menu bar and modal overlays.
#![allow(dead_code)]

pub mod ai;
pub mod credentials;
pub mod library;
pub mod menu_bar;
pub mod modals;
pub mod router;

use crate::ui::components::status::ServiceState;
use crate::workspace::ProjectSummary;
use crate::ui::theme;
use crate::views;
use crate::views::studio::persistence;
use eframe::egui;
use modals::Modals;
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
    pub ai: ai::AiBridge,
    /// Credentials form — the only place API keys are entered.
    pub settings: views::settings::SettingsState,
    /// When the open project first differed from what is on disk.
    dirty_since: Option<f32>,
}

impl AiDirectorApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        let ai = ai::AiBridge::new();
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
            ai,
            settings: Default::default(),
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
        self.ai.poll();
        self.library.poll(&mut self.route);
        self.sync_open_project();

        // 1. Chrome first, so every route sits under the same menu bar.
        let project_name = self
            .route
            .project()
            .and_then(|id| self.project(id))
            .map(|p| p.name.clone());
        menu_bar::show(
            ctx,
            &mut menu_bar::MenuBarCtx {
                route: &mut self.route,
                history: &mut self.history,
                modals: &mut self.modals,
                gallery_open: &mut self.gallery_open,
                project_name: project_name.as_deref(),
                service: self.services[1],
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
            AppRoute::Growth(id) => views::growth::show(
                ctx,
                &mut self.route,
                &mut self.growth,
                &mut self.modals,
                self.library.find(id),
            ),
        }

        // 3. Optional dev surface, then overlays on top of everything.
        if views::settings::show(ctx, &mut self.settings)
            == Some(views::settings::SettingsOutcome::Saved)
        {
            self.apply_credentials();
        }
        views::gallery::window(ctx, &mut self.gallery_open, &mut self.gallery, self.time);
        modals::show(ctx, &mut self.modals, &mut self.route, dt);

        self.history.track(self.route);
    }
}
