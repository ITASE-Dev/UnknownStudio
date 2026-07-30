//! Global application shell: state, routing, menu bar and modal overlays.
#![allow(dead_code)]

pub mod ai;
pub mod menu_bar;
pub mod modals;
pub mod router;

use crate::ui::components::status::ServiceState;
use crate::ui::theme;
use crate::views;
use eframe::egui;
use modals::Modals;
use router::{AppRoute, ProjectId, RouteHistory};

#[derive(Clone)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub platform: String,
    pub blueprint: String,
    pub duration: String,
    pub modified: String,
    pub clips: usize,
}

pub struct AiDirectorApp {
    pub route: AppRoute,
    pub history: RouteHistory,
    pub modals: Modals,
    pub projects: Vec<Project>,
    pub next_id: ProjectId,
    pub services: Vec<(&'static str, ServiceState)>,
    pub time: f32,
    pub gallery_open: bool,
    pub gallery: views::gallery::GalleryState,
    pub dashboard: views::dashboard::DashboardState,
    pub onboarding: views::onboarding::OnboardingState,
    pub studio: views::studio::StudioState,
    pub growth: views::growth::GrowthState,
    pub ai: ai::AiBridge,
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
            projects: seed_projects(),
            next_id: 4,
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
        }
    }

    fn project(&self, id: ProjectId) -> Option<&Project> {
        self.projects.iter().find(|p| p.id == id)
    }
}

impl eframe::App for AiDirectorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let now = ctx.input(|i| i.time) as f32;
        let dt = (now - self.time).clamp(0.0, 0.1);
        self.time = now;
        ctx.request_repaint_after(std::time::Duration::from_millis(33));

        // 0. Drain the action engine before any view reads its state.
        self.ai.poll();

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
            AppRoute::Dashboard => views::dashboard::show(
                ctx,
                &mut self.route,
                &mut self.dashboard,
                &self.projects,
                &mut self.modals,
            ),
            AppRoute::Onboarding => views::onboarding::show(
                ctx,
                &mut self.route,
                &mut self.onboarding,
                &mut self.projects,
                &mut self.next_id,
            ),
            AppRoute::Studio(id) => views::studio::show(
                ctx,
                &mut self.route,
                &mut self.studio,
                &mut self.modals,
                self.projects.iter().find(|p| p.id == id),
                self.time,
            ),
            AppRoute::Growth(id) => views::growth::show(
                ctx,
                &mut self.route,
                &mut self.growth,
                &mut self.modals,
                self.projects.iter().find(|p| p.id == id),
            ),
        }

        // 3. Optional dev surface, then overlays on top of everything.
        views::gallery::window(ctx, &mut self.gallery_open, &mut self.gallery, self.time);
        modals::show(ctx, &mut self.modals, &mut self.route, dt);

        self.history.track(self.route);
    }
}

fn seed_projects() -> Vec<Project> {
    vec![
        Project {
            id: 1,
            name: "Ep_014 · Rust in 100s".into(),
            platform: "YouTube 16:9".into(),
            blueprint: "Fireship".into(),
            duration: "00:04:12".into(),
            modified: "2 hours ago".into(),
            clips: 34,
        },
        Project {
            id: 2,
            name: "Shorts · Borrow checker".into(),
            platform: "Shorts 9:16".into(),
            blueprint: "Hook-first".into(),
            duration: "00:00:48".into(),
            modified: "yesterday".into(),
            clips: 12,
        },
        Project {
            id: 3,
            name: "Podcast · Ep 07 highlights".into(),
            platform: "YouTube 16:9".into(),
            blueprint: "Podcast".into(),
            duration: "00:12:30".into(),
            modified: "last week".into(),
            clips: 61,
        },
    ]
}
