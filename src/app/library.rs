//! The project list the UI shows, backed by `workspace` folders on disk.
//!
//! Every row here corresponds to a real `project.json`. Creation, loading and
//! saving run on a Tokio runtime; the UI submits and polls, never blocks.

use crate::app::router::{AppRoute, ProjectId};
use crate::app::Project;
use crate::workspace::{
    io, ChatHistory, ProjectConfig, ProjectContext, ProjectSummary, TargetPlatform,
    TimelineSnapshot, Workspace,
};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::mpsc::{self, Receiver, Sender};
use tokio::runtime::Runtime;

/// Routes are `Copy`, so the on-disk string id is carried as a stable hash.
pub fn route_id(disk_id: &str) -> ProjectId {
    let mut hasher = DefaultHasher::new();
    disk_id.hash(&mut hasher);
    hasher.finish()
}

enum LibraryEvent {
    Listed(Vec<ProjectSummary>),
    Created(ProjectSummary),
    Loaded {
        disk_id: String,
        config: Box<ProjectConfig>,
        chat: ChatHistory,
    },
    Saved(String),
    Failed(String),
}

/// What the studio hands back to be written to disk.
pub struct ProjectSave {
    pub timeline: TimelineSnapshot,
    pub chat: ChatHistory,
}

pub struct ProjectLibrary {
    runtime: Option<Runtime>,
    workspace: Option<Workspace>,
    events_tx: Sender<LibraryEvent>,
    events_rx: Receiver<LibraryEvent>,

    /// Rows the dashboard renders — only projects that exist on disk.
    pub projects: Vec<Project>,
    /// Last failure, surfaced in the UI instead of being swallowed.
    pub error: Option<String>,
    /// Project loaded into the studio, if any.
    pub open: Option<OpenProject>,
    /// Navigate to the studio once the pending creation lands.
    open_after_create: bool,
    /// Load requested for a project the studio just routed to.
    pending_load: Option<String>,
    saving: bool,
}

pub struct OpenProject {
    pub context: ProjectContext,
    pub config: ProjectConfig,
    /// State last written, so an unchanged timeline is not rewritten.
    pub saved_timeline: TimelineSnapshot,
    pub saved_chat: ChatHistory,
    /// Set when a load arrives and the studio has not consumed it yet.
    pub needs_apply: bool,
}

impl Default for ProjectLibrary {
    fn default() -> Self {
        Self::new()
    }
}

impl ProjectLibrary {
    pub fn new() -> Self {
        let (events_tx, events_rx) = mpsc::channel();
        let mut library = Self {
            runtime: Runtime::new().ok(),
            workspace: None,
            events_tx,
            events_rx,
            projects: Vec::new(),
            error: None,
            open: None,
            open_after_create: false,
            pending_load: None,
            saving: false,
        };

        match Workspace::open() {
            Ok(workspace) => {
                library.workspace = Some(workspace);
                library.refresh();
            }
            Err(err) => library.error = Some(err.to_string()),
        }
        if library.runtime.is_none() {
            library.error = Some("could not start the storage runtime".into());
        }
        library
    }

    /// Library over an explicit root — used by tests and portable installs.
    pub fn with_workspace(workspace: Workspace) -> Self {
        let (events_tx, events_rx) = mpsc::channel();
        let mut library = Self {
            runtime: Runtime::new().ok(),
            workspace: Some(workspace),
            events_tx,
            events_rx,
            projects: Vec::new(),
            error: None,
            open: None,
            open_after_create: false,
            pending_load: None,
            saving: false,
        };
        library.refresh();
        library
    }

    pub fn root_display(&self) -> String {
        self.workspace
            .as_ref()
            .map(|w| w.root().display().to_string())
            .unwrap_or_default()
    }

    fn spawn<F>(&self, task: F)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        if let Some(runtime) = &self.runtime {
            runtime.spawn(task);
        }
    }

    /// Re-reads the projects folder.
    pub fn refresh(&mut self) {
        let (Some(workspace), Some(_)) = (self.workspace.clone(), self.runtime.as_ref()) else {
            return;
        };
        let tx = self.events_tx.clone();
        self.spawn(async move {
            let event = match workspace.list_projects().await {
                Ok(list) => LibraryEvent::Listed(list),
                Err(err) => LibraryEvent::Failed(err.to_string()),
            };
            let _ = tx.send(event);
        });
    }

    /// Creates the folder tree, writes `project.json`, then opens the studio.
    pub fn create_and_open(&mut self, name: &str, target: TargetPlatform, blueprint: &str) {
        let Some(workspace) = self.workspace.clone() else {
            self.error = Some("no writable projects folder".into());
            return;
        };
        let (name, blueprint) = (name.to_string(), blueprint.to_string());
        let tx = self.events_tx.clone();
        self.open_after_create = true;

        self.spawn(async move {
            let event = match create(&workspace, &name, target, &blueprint).await {
                Ok(summary) => LibraryEvent::Created(summary),
                Err(message) => LibraryEvent::Failed(message),
            };
            let _ = tx.send(event);
        });
    }

    /// Loads a project into `open`, unless it is already the open one.
    pub fn request_open(&mut self, id: ProjectId) {
        let Some(disk_id) = self.disk_id(id) else {
            return;
        };
        if self.open.as_ref().is_some_and(|p| p.config.id == disk_id) {
            return;
        }
        if self.pending_load.as_deref() == Some(disk_id.as_str()) {
            return;
        }

        let Some(workspace) = self.workspace.clone() else {
            return;
        };
        self.pending_load = Some(disk_id.clone());
        let tx = self.events_tx.clone();

        self.spawn(async move {
            let event = match workspace.open_project(&disk_id).await {
                Ok((ctx, config)) => {
                    let chat = io::load_chat(&ctx).await.unwrap_or_default();
                    LibraryEvent::Loaded {
                        disk_id,
                        config: Box::new(config),
                        chat,
                    }
                }
                Err(err) => LibraryEvent::Failed(err.to_string()),
            };
            let _ = tx.send(event);
        });
    }

    /// Writes the open project when something actually changed.
    pub fn save_open(&mut self, save: ProjectSave) {
        let Some(open) = &mut self.open else {
            return;
        };
        if self.saving || (save.timeline == open.saved_timeline && save.chat == open.saved_chat) {
            return;
        }

        open.config.timeline = save.timeline.clone();
        open.saved_timeline = save.timeline;
        open.saved_chat = save.chat.clone();

        let ctx = open.context.clone();
        let mut config = open.config.clone();
        let chat = save.chat;
        let tx = self.events_tx.clone();
        self.saving = true;

        self.spawn(async move {
            let event = match io::save_project(&ctx, &mut config).await {
                Ok(()) => match io::save_chat(&ctx, &chat).await {
                    Ok(()) => LibraryEvent::Saved(ctx.id.clone()),
                    Err(err) => LibraryEvent::Failed(err.to_string()),
                },
                Err(err) => LibraryEvent::Failed(err.to_string()),
            };
            let _ = tx.send(event);
        });
    }

    /// Drains background results. Navigates to a freshly created project.
    pub fn poll(&mut self, route: &mut AppRoute) {
        let events: Vec<LibraryEvent> =
            std::iter::from_fn(|| self.events_rx.try_recv().ok()).collect();

        for event in events {
            match event {
                LibraryEvent::Listed(list) => {
                    self.projects = list.iter().map(Project::from_summary).collect();
                    self.error = None;
                }
                LibraryEvent::Created(summary) => {
                    let id = route_id(&summary.id);
                    self.projects.insert(0, Project::from_summary(&summary));
                    self.error = None;
                    if self.open_after_create {
                        self.open_after_create = false;
                        *route = AppRoute::Studio(id);
                    }
                }
                LibraryEvent::Loaded {
                    disk_id,
                    config,
                    chat,
                } => {
                    if self.pending_load.as_deref() == Some(disk_id.as_str()) {
                        self.pending_load = None;
                    }
                    let Some(workspace) = &self.workspace else {
                        continue;
                    };
                    self.open = Some(OpenProject {
                        context: workspace.context(&disk_id),
                        saved_timeline: config.timeline.clone(),
                        saved_chat: chat,
                        config: *config,
                        needs_apply: true,
                    });
                }
                LibraryEvent::Saved(_) => {
                    self.saving = false;
                    self.refresh();
                }
                LibraryEvent::Failed(message) => {
                    self.saving = false;
                    self.open_after_create = false;
                    self.pending_load = None;
                    self.error = Some(message);
                }
            }
        }
    }

    pub fn find(&self, id: ProjectId) -> Option<&Project> {
        self.projects.iter().find(|p| p.id == id)
    }

    fn disk_id(&self, id: ProjectId) -> Option<String> {
        self.find(id).map(|p| p.disk_id.clone())
    }
}

async fn create(
    workspace: &Workspace,
    name: &str,
    target: TargetPlatform,
    blueprint: &str,
) -> Result<ProjectSummary, String> {
    let ctx = workspace
        .create_new_project(name)
        .await
        .map_err(|err| err.to_string())?;

    let mut config = io::load_project(&ctx).await.map_err(|err| err.to_string())?;
    config.settings.target = target;
    config.settings.blueprint = Some(blueprint.to_string());
    io::save_project(&ctx, &mut config)
        .await
        .map_err(|err| err.to_string())?;

    Ok(ProjectSummary::from_config(&config, ctx.root))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{io, TimelineSnapshot};
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "unknown_studio_lib_{label}_{}",
            uuid::Uuid::new_v4().simple()
        ))
    }

    /// Pumps the event loop until `done`, or gives up.
    fn pump(library: &mut ProjectLibrary, route: &mut AppRoute, done: impl Fn(&ProjectLibrary) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            library.poll(route);
            if done(library) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("library did not settle in time");
    }

    #[test]
    fn created_projects_are_written_to_disk_and_listed() {
        let root = temp_root("create");
        let mut library = ProjectLibrary::with_workspace(Workspace::with_root(&root));
        let mut route = AppRoute::Onboarding;

        // Nothing on disk yet, so nothing is listed.
        pump(&mut library, &mut route, |l| l.error.is_none());
        assert!(library.projects.is_empty());

        library.create_and_open("Ep 21 Borrow Checker", TargetPlatform::YouTubeShorts, "Fireship");
        pump(&mut library, &mut route, |l| !l.projects.is_empty());

        let project = &library.projects[0];
        assert_eq!(project.name, "Ep 21 Borrow Checker");
        assert_eq!(project.platform, "Shorts 9:16");
        assert!(root.join(&project.disk_id).join("project.json").exists());
        // Creation navigates straight into the studio.
        assert_eq!(route, AppRoute::Studio(project.id));

        // A fresh library over the same root sees the same project.
        let mut reopened = ProjectLibrary::with_workspace(Workspace::with_root(&root));
        pump(&mut reopened, &mut route, |l| !l.projects.is_empty());
        assert_eq!(reopened.projects[0].disk_id, project.disk_id);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn saving_persists_the_timeline_for_the_next_session() {
        let root = temp_root("save");
        let mut library = ProjectLibrary::with_workspace(Workspace::with_root(&root));
        let mut route = AppRoute::Dashboard;

        library.create_and_open("Persisted", TargetPlatform::YouTubeLandscape, "Vlog");
        pump(&mut library, &mut route, |l| !l.projects.is_empty());
        let id = library.projects[0].id;

        library.request_open(id);
        pump(&mut library, &mut route, |l| l.open.is_some());
        library.open.as_mut().expect("open").needs_apply = false;

        let mut timeline = TimelineSnapshot::default();
        timeline.playhead_seconds = 4.25;
        library.save_open(ProjectSave {
            timeline: timeline.clone(),
            chat: ChatHistory::default(),
        });
        pump(&mut library, &mut route, |l| !l.saving);

        let ctx = Workspace::with_root(&root).context(&library.projects[0].disk_id);
        let config = Runtime::new()
            .expect("rt")
            .block_on(io::load_project(&ctx))
            .expect("load");
        assert_eq!(config.timeline.playhead_seconds, 4.25);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn route_ids_are_stable_per_folder() {
        assert_eq!(route_id("demo-abc12345"), route_id("demo-abc12345"));
        assert_ne!(route_id("demo-abc12345"), route_id("demo-abc12346"));
    }
}
