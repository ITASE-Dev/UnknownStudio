//! Local project storage: an NLE-style folder per project under
//! `~/Documents/UnknownStudio/Projects/`, holding settings, timeline state,
//! media, AI analysis, chat history and exports.
#![allow(dead_code)]

pub mod fs_manager;
pub mod io;
pub mod models;

pub use fs_manager::{project_id, projects_root, slugify, ProjectContext};
pub use models::{
    AnalysisMetadata, AnalysisNote, AudioAnalysis, ChatHistory, ChatMessage, ClipKind,
    ClipSnapshot, MediaEntry, ProjectConfig, ProjectSettings, ProjectSummary, TargetPlatform,
    TimelineSnapshot, TrackKind, TrackSnapshot, VisualAnalysis, PROJECT_FORMAT_VERSION,
};

use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum WorkspaceError {
    /// No Documents folder and no platform data directory.
    NoHomeDirectory,
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Parse {
        path: PathBuf,
        source: serde_json::Error,
    },
    ProjectNotFound(String),
    ProjectExists(String),
    MediaNotFound(PathBuf),
    /// Written by a newer build than this one understands.
    UnsupportedVersion {
        found: u32,
        supported: u32,
    },
}

impl WorkspaceError {
    pub(crate) fn io(path: impl AsRef<Path>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.as_ref().to_path_buf(),
            source,
        }
    }

    pub(crate) fn parse(path: impl AsRef<Path>, source: serde_json::Error) -> Self {
        Self::Parse {
            path: path.as_ref().to_path_buf(),
            source,
        }
    }
}

impl fmt::Display for WorkspaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoHomeDirectory => write!(f, "no writable user directory found"),
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::Parse { path, source } => write!(f, "{}: {source}", path.display()),
            Self::ProjectNotFound(id) => write!(f, "project not found: {id}"),
            Self::ProjectExists(id) => write!(f, "project already exists: {id}"),
            Self::MediaNotFound(path) => write!(f, "media not found: {}", path.display()),
            Self::UnsupportedVersion { found, supported } => write!(
                f,
                "project format v{found} is newer than supported v{supported}"
            ),
        }
    }
}

impl std::error::Error for WorkspaceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Parse { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub type Result<T> = std::result::Result<T, WorkspaceError>;

/// Entry point to on-disk projects. Holds only the root path; all I/O is async
/// and per-call, so the UI can keep one of these around indefinitely.
#[derive(Clone, Debug)]
pub struct Workspace {
    root: PathBuf,
}

impl Workspace {
    /// Resolves the default projects root. Touches no files.
    pub fn open() -> Result<Self> {
        Ok(Self {
            root: projects_root()?,
        })
    }

    /// Explicit root — used by tests and by portable installs.
    pub fn with_root(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn context(&self, project_id: &str) -> ProjectContext {
        ProjectContext::new(project_id, &self.root)
    }

    /// Builds the whole folder tree and writes an initial `project.json`.
    /// Existing directories are reused; an existing project is an error rather
    /// than a silent overwrite.
    pub async fn create_new_project(&self, name: &str) -> Result<ProjectContext> {
        let ctx = ProjectContext::new(project_id(name), &self.root);
        if ctx.exists().await {
            return Err(WorkspaceError::ProjectExists(ctx.id.clone()));
        }

        ctx.ensure_tree().await?;
        let mut config = ProjectConfig::new(&ctx.id, name);
        io::save_project(&ctx, &mut config).await?;
        Ok(ctx)
    }

    /// Opens an existing project, repairing any directory that went missing.
    pub async fn open_project(&self, project_id: &str) -> Result<(ProjectContext, ProjectConfig)> {
        let ctx = self.context(project_id);
        if !ctx.exists().await {
            return Err(WorkspaceError::ProjectNotFound(project_id.to_string()));
        }
        ctx.ensure_tree().await?;
        let config = io::load_project(&ctx).await?;
        Ok((ctx, config))
    }

    /// Every project on disk, most recently modified first. Projects whose
    /// `project.json` is unreadable are skipped so one bad folder cannot break
    /// the picker.
    pub async fn list_projects(&self) -> Result<Vec<ProjectSummary>> {
        let mut summaries = Vec::new();
        for id in fs_manager::project_ids(&self.root).await? {
            let ctx = self.context(&id);
            let Ok(config) = io::load_project(&ctx).await else {
                continue;
            };
            summaries.push(ProjectSummary::from_config(&config, ctx.root));
        }
        summaries.sort_by(|a, b| b.modified_at.cmp(&a.modified_at));
        Ok(summaries)
    }

    /// Deletes a project folder and everything under it.
    pub async fn delete_project(&self, project_id: &str) -> Result<()> {
        let ctx = self.context(project_id);
        if !ctx.exists().await {
            return Err(WorkspaceError::ProjectNotFound(project_id.to_string()));
        }
        tokio::fs::remove_dir_all(&ctx.root)
            .await
            .map_err(|err| WorkspaceError::io(&ctx.root, err))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use models::{AnalysisNote, ChatMessage};

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "unknown_studio_ws_{label}_{}",
            uuid::Uuid::new_v4().simple()
        ))
    }

    #[tokio::test]
    async fn create_new_project_builds_the_whole_tree() {
        let root = temp_root("create");
        let workspace = Workspace::with_root(&root);

        let ctx = workspace
            .create_new_project("Ep_014 · Rust in 100s")
            .await
            .expect("create");

        assert!(ctx.id.starts_with("ep-014-rust-in-100s-"));
        for path in [
            ctx.project_file(),
            ctx.raw_media_dir(),
            ctx.generated_media_dir(),
            ctx.metadata_dir(),
            ctx.chat_dir(),
            ctx.exports_dir(),
        ] {
            assert!(path.exists(), "missing {}", path.display());
        }

        let config = io::load_project(&ctx).await.expect("load");
        assert_eq!(config.name, "Ep_014 · Rust in 100s");
        assert_eq!(config.version, PROJECT_FORMAT_VERSION);

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn recreating_the_same_tree_is_safe() {
        let root = temp_root("idempotent");
        let workspace = Workspace::with_root(&root);
        let ctx = workspace.create_new_project("Demo").await.expect("create");

        // Re-running initialization must not clear existing content.
        tokio::fs::write(ctx.raw_media_dir().join("keep.txt"), b"x")
            .await
            .expect("write");
        ctx.ensure_tree().await.expect("ensure twice");
        assert!(ctx.raw_media_dir().join("keep.txt").exists());

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn project_chat_and_analysis_round_trip() {
        let root = temp_root("roundtrip");
        let workspace = Workspace::with_root(&root);
        let ctx = workspace.create_new_project("Round Trip").await.expect("create");

        let mut config = io::load_project(&ctx).await.expect("load");
        config.settings.target = TargetPlatform::YouTubeShorts;
        config.timeline.tracks.push(TrackSnapshot {
            name: "V1".into(),
            kind: TrackKind::Video,
            muted: false,
            locked: false,
            clips: vec![ClipSnapshot {
                id: 1,
                label: "a.mp4".into(),
                source: Some(PathBuf::from("media/raw/a.mp4")),
                kind: ClipKind::ARoll,
                start_seconds: 0.0,
                duration_seconds: 4.0,
                trim_in_seconds: 0.0,
                source_seconds: 6.0,
                has_audio: true,
                gain: 1.0,
            }],
        });
        io::save_project(&ctx, &mut config).await.expect("save");

        let reloaded = io::load_project(&ctx).await.expect("reload");
        assert_eq!(reloaded.settings.target.resolution(), (1080, 1920));
        assert_eq!(reloaded.timeline.tracks[0].clips[0].duration_seconds, 4.0);

        let mut history = io::load_chat(&ctx).await.expect("empty chat");
        assert!(history.messages.is_empty());
        history.push(ChatMessage::user("tighten the intro"));
        history.push(ChatMessage::assistant("trimmed 1.2s of silence"));
        io::save_chat(&ctx, &history).await.expect("save chat");
        assert_eq!(io::load_chat(&ctx).await.expect("chat").messages.len(), 2);

        let mut analysis = AnalysisMetadata::new("media/raw/a.mp4");
        analysis.visual.notes.push(AnalysisNote {
            id: "n1".into(),
            critique: "too dark".into(),
            proposed_action: "brighten".into(),
            action_type: "COLOR_CORRECT".into(),
            context: Some("brightness=0.1".into()),
            applied: false,
        });
        analysis.audio.transcript = Some("hello".into());
        io::save_analysis(&ctx, "a", &analysis).await.expect("save analysis");

        let loaded = io::load_analysis(&ctx, "a").await.expect("read").expect("some");
        assert_eq!(loaded.visual.notes.len(), 1);
        assert_eq!(loaded.audio.transcript.as_deref(), Some("hello"));
        assert!(io::load_analysis(&ctx, "missing").await.expect("read").is_none());

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn media_import_copies_and_never_overwrites() {
        let root = temp_root("media");
        let workspace = Workspace::with_root(&root);
        let ctx = workspace.create_new_project("Media").await.expect("create");

        let source = root.join("outside.mp4");
        tokio::fs::write(&source, b"one").await.expect("write source");

        let first = io::import_raw_media(&ctx, &source).await.expect("import");
        assert_eq!(first, PathBuf::from("media/raw").join("outside.mp4"));

        let second = io::import_raw_media(&ctx, &source).await.expect("reimport");
        assert_ne!(first, second, "a clash must not overwrite the first copy");
        assert!(ctx.resolve(&second).exists());

        let missing = io::import_raw_media(&ctx, Path::new("nope.mp4")).await;
        assert!(matches!(missing, Err(WorkspaceError::MediaNotFound(_))));

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn listing_sorts_by_modification_and_skips_broken_folders() {
        let root = temp_root("list");
        let workspace = Workspace::with_root(&root);

        let first = workspace.create_new_project("First").await.expect("first");
        let second = workspace.create_new_project("Second").await.expect("second");

        // Make the first the most recently modified.
        let mut config = io::load_project(&first).await.expect("load");
        config.modified_at = models::now_unix() + 60;
        io::write_json(&first.project_file(), &config).await.expect("write");

        // A folder with an unreadable project file must not break the listing.
        let broken = root.join("broken-project");
        tokio::fs::create_dir_all(&broken).await.expect("dir");
        tokio::fs::write(broken.join("project.json"), b"{ not json")
            .await
            .expect("write");

        let projects = workspace.list_projects().await.expect("list");
        assert_eq!(projects.len(), 2);
        assert_eq!(projects[0].id, first.id);
        assert_eq!(projects[1].id, second.id);

        workspace.delete_project(&second.id).await.expect("delete");
        assert_eq!(workspace.list_projects().await.expect("list").len(), 1);
        assert!(matches!(
            workspace.open_project(&second.id).await,
            Err(WorkspaceError::ProjectNotFound(_))
        ));

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn a_newer_format_is_refused_rather_than_misread() {
        let root = temp_root("version");
        let workspace = Workspace::with_root(&root);
        let ctx = workspace.create_new_project("Future").await.expect("create");

        let mut config = io::load_project(&ctx).await.expect("load");
        config.version = PROJECT_FORMAT_VERSION + 1;
        io::write_json(&ctx.project_file(), &config).await.expect("write");

        assert!(matches!(
            io::load_project(&ctx).await,
            Err(WorkspaceError::UnsupportedVersion { .. })
        ));

        let _ = tokio::fs::remove_dir_all(&root).await;
    }
}
