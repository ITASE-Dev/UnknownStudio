//! Path resolution and directory tree construction.
//!
//! Layout per project:
//! ```text
//! <projects_root>/<project_id>/
//!   project.json
//!   media/raw/
//!   media/generated/
//!   metadata/
//!   chat/history.json
//!   exports/
//! ```

use crate::workspace::WorkspaceError;
use directories::{ProjectDirs, UserDirs};
use std::path::{Path, PathBuf};
use tokio::fs;

pub const APP_DIR_NAME: &str = "UnknownStudio";
pub const PROJECTS_DIR_NAME: &str = "Projects";
pub const PROJECT_FILE: &str = "project.json";
pub const CHAT_FILE: &str = "history.json";

/// `~/Documents/UnknownStudio/Projects`, falling back to the platform data
/// directory when there is no Documents folder (headless Linux, sandboxes).
pub fn projects_root() -> Result<PathBuf, WorkspaceError> {
    if let Some(documents) = UserDirs::new().and_then(|dirs| dirs.document_dir().map(Path::to_path_buf)) {
        return Ok(documents.join(APP_DIR_NAME).join(PROJECTS_DIR_NAME));
    }

    ProjectDirs::from("dev", APP_DIR_NAME, APP_DIR_NAME)
        .map(|dirs| dirs.data_dir().join(PROJECTS_DIR_NAME))
        .ok_or(WorkspaceError::NoHomeDirectory)
}

/// Every path inside one project. Cheap to clone and holds no I/O state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectContext {
    pub id: String,
    pub root: PathBuf,
}

impl ProjectContext {
    pub fn new(id: impl Into<String>, projects_root: impl AsRef<Path>) -> Self {
        let id = id.into();
        let root = projects_root.as_ref().join(&id);
        Self { id, root }
    }

    pub fn project_file(&self) -> PathBuf {
        self.root.join(PROJECT_FILE)
    }

    pub fn media_dir(&self) -> PathBuf {
        self.root.join("media")
    }

    pub fn raw_media_dir(&self) -> PathBuf {
        self.media_dir().join("raw")
    }

    pub fn generated_media_dir(&self) -> PathBuf {
        self.media_dir().join("generated")
    }

    pub fn metadata_dir(&self) -> PathBuf {
        self.root.join("metadata")
    }

    pub fn chat_dir(&self) -> PathBuf {
        self.root.join("chat")
    }

    pub fn chat_file(&self) -> PathBuf {
        self.chat_dir().join(CHAT_FILE)
    }

    pub fn exports_dir(&self) -> PathBuf {
        self.root.join("exports")
    }

    /// Analysis file for one media key (a file stem, slugified by the caller).
    pub fn metadata_file(&self, media_key: &str) -> PathBuf {
        self.metadata_dir().join(format!("{media_key}.json"))
    }

    /// Absolute path for a project-relative one; absolute inputs pass through,
    /// so media outside the project folder still resolves.
    pub fn resolve(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        }
    }

    /// Project-relative path when the file lives inside the project, so the
    /// folder can be moved or shared without breaking references.
    pub fn relativize(&self, path: &Path) -> PathBuf {
        path.strip_prefix(&self.root)
            .map(Path::to_path_buf)
            .unwrap_or_else(|_| path.to_path_buf())
    }

    /// Creates the full tree. Existing directories are left untouched.
    pub async fn ensure_tree(&self) -> Result<(), WorkspaceError> {
        for dir in [
            self.root.clone(),
            self.raw_media_dir(),
            self.generated_media_dir(),
            self.metadata_dir(),
            self.chat_dir(),
            self.exports_dir(),
        ] {
            fs::create_dir_all(&dir)
                .await
                .map_err(|err| WorkspaceError::io(&dir, err))?;
        }
        Ok(())
    }

    pub async fn exists(&self) -> bool {
        fs::try_exists(self.project_file()).await.unwrap_or(false)
    }
}

/// Filesystem-safe, lowercase, collapsed to single dashes.
pub fn slugify(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());
    let mut dash = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            dash = false;
        } else if !slug.is_empty() && !dash {
            slug.push('-');
            dash = true;
        }
    }
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        "project".to_string()
    } else {
        slug.chars().take(48).collect()
    }
}

/// `slug-<8 hex>`, so two projects named the same never collide on disk.
pub fn project_id(name: &str) -> String {
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    format!("{}-{}", slugify(name), &suffix[..8])
}

/// Project directories under `root`, newest first by directory name is not
/// guaranteed — callers sort by the config's `modified_at`.
pub async fn project_ids(root: &Path) -> Result<Vec<String>, WorkspaceError> {
    if !fs::try_exists(root).await.unwrap_or(false) {
        return Ok(Vec::new());
    }

    let mut ids = Vec::new();
    let mut entries = fs::read_dir(root)
        .await
        .map_err(|err| WorkspaceError::io(root, err))?;

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|err| WorkspaceError::io(root, err))?
    {
        let path = entry.path();
        if !fs::try_exists(path.join(PROJECT_FILE)).await.unwrap_or(false) {
            continue;
        }
        if let Some(id) = path.file_name().and_then(|n| n.to_str()) {
            ids.push(id.to_string());
        }
    }

    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_is_filesystem_safe() {
        assert_eq!(slugify("Ep_014 · Rust in 100s"), "ep-014-rust-in-100s");
        assert_eq!(slugify("  ///  "), "project");
        assert_eq!(slugify("Ünïcode!!"), "n-code");
        assert!(slugify(&"x".repeat(200)).len() <= 48);
    }

    #[test]
    fn ids_are_unique_per_call() {
        assert_ne!(project_id("same name"), project_id("same name"));
        assert!(project_id("Demo Cut").starts_with("demo-cut-"));
    }

    #[test]
    fn layout_matches_the_documented_tree() {
        let ctx = ProjectContext::new("demo-abc12345", Path::new("/tmp/projects"));
        assert!(ctx.project_file().ends_with("demo-abc12345/project.json"));
        assert!(ctx.raw_media_dir().ends_with("media/raw"));
        assert!(ctx.generated_media_dir().ends_with("media/generated"));
        assert!(ctx.chat_file().ends_with("chat/history.json"));
        assert!(ctx.metadata_file("clip_01").ends_with("metadata/clip_01.json"));
        assert!(ctx.exports_dir().ends_with("exports"));
    }

    #[test]
    fn paths_round_trip_through_the_project_root() {
        let ctx = ProjectContext::new("demo", Path::new("/tmp/projects"));
        let inside = ctx.raw_media_dir().join("a.mp4");
        assert_eq!(ctx.relativize(&inside), Path::new("media/raw/a.mp4"));
        assert_eq!(ctx.resolve(Path::new("media/raw/a.mp4")), inside);

        // Media outside the project keeps its absolute path.
        let outside = Path::new("/elsewhere/b.mp4");
        assert_eq!(ctx.relativize(outside), outside);
        assert_eq!(ctx.resolve(outside), outside);
    }
}
