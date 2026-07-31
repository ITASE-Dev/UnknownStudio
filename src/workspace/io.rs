//! Async JSON and media I/O. Every write is atomic: a temp file next to the
//! target is renamed over it, so a crash mid-save cannot truncate a project.

use crate::workspace::fs_manager::ProjectContext;
use crate::workspace::models::{
    AnalysisMetadata, ChatHistory, ProjectConfig, PROJECT_FORMAT_VERSION,
};
use crate::workspace::WorkspaceError;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::path::{Path, PathBuf};
use tokio::fs;

pub async fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, WorkspaceError> {
    let bytes = fs::read(path)
        .await
        .map_err(|err| WorkspaceError::io(path, err))?;
    serde_json::from_slice(&bytes).map_err(|err| WorkspaceError::parse(path, err))
}

/// Reads `path`, or returns the default when the file does not exist yet.
/// A malformed file is still an error — silently discarding it would lose work.
pub async fn read_json_or_default<T: DeserializeOwned + Default>(
    path: &Path,
) -> Result<T, WorkspaceError> {
    match fs::read(path).await {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|err| WorkspaceError::parse(path, err)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(err) => Err(WorkspaceError::io(path, err)),
    }
}

pub async fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), WorkspaceError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|err| WorkspaceError::io(parent, err))?;
    }

    let bytes = serde_json::to_vec_pretty(value).map_err(|err| WorkspaceError::parse(path, err))?;
    let temp = path.with_extension("json.tmp");

    fs::write(&temp, &bytes)
        .await
        .map_err(|err| WorkspaceError::io(&temp, err))?;
    fs::rename(&temp, path)
        .await
        .map_err(|err| WorkspaceError::io(path, err))?;
    Ok(())
}

pub async fn load_project(ctx: &ProjectContext) -> Result<ProjectConfig, WorkspaceError> {
    let config: ProjectConfig = read_json(&ctx.project_file()).await?;
    if config.version > PROJECT_FORMAT_VERSION {
        return Err(WorkspaceError::UnsupportedVersion {
            found: config.version,
            supported: PROJECT_FORMAT_VERSION,
        });
    }
    Ok(config)
}

/// Saves the project, stamping `modified_at`.
pub async fn save_project(
    ctx: &ProjectContext,
    config: &mut ProjectConfig,
) -> Result<(), WorkspaceError> {
    config.touch();
    write_json(&ctx.project_file(), config).await
}

pub async fn load_chat(ctx: &ProjectContext) -> Result<ChatHistory, WorkspaceError> {
    read_json_or_default(&ctx.chat_file()).await
}

pub async fn save_chat(ctx: &ProjectContext, history: &ChatHistory) -> Result<(), WorkspaceError> {
    write_json(&ctx.chat_file(), history).await
}

pub async fn load_analysis(
    ctx: &ProjectContext,
    media_key: &str,
) -> Result<Option<AnalysisMetadata>, WorkspaceError> {
    let path = ctx.metadata_file(media_key);
    match fs::try_exists(&path).await {
        Ok(true) => read_json(&path).await.map(Some),
        _ => Ok(None),
    }
}

pub async fn save_analysis(
    ctx: &ProjectContext,
    media_key: &str,
    analysis: &AnalysisMetadata,
) -> Result<(), WorkspaceError> {
    write_json(&ctx.metadata_file(media_key), analysis).await
}

/// Every analysis file in the project, keyed by file stem.
pub async fn list_analysis_keys(ctx: &ProjectContext) -> Result<Vec<String>, WorkspaceError> {
    let dir = ctx.metadata_dir();
    if !fs::try_exists(&dir).await.unwrap_or(false) {
        return Ok(Vec::new());
    }

    let mut keys = Vec::new();
    let mut entries = fs::read_dir(&dir)
        .await
        .map_err(|err| WorkspaceError::io(&dir, err))?;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|err| WorkspaceError::io(&dir, err))?
    {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            keys.push(stem.to_string());
        }
    }
    Ok(keys)
}

/// Stable metadata key for a media file: its slugified stem.
pub fn media_key(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("media");
    crate::workspace::fs_manager::slugify(stem)
}

/// Copies footage into `media/raw/`, returning the project-relative path.
/// Copying rather than linking keeps the folder self-contained; a name clash
/// gets a numeric suffix instead of overwriting.
pub async fn import_raw_media(
    ctx: &ProjectContext,
    source: &Path,
) -> Result<PathBuf, WorkspaceError> {
    copy_into(ctx, source, ctx.raw_media_dir()).await
}

/// Same, for AI-generated assets.
pub async fn import_generated_media(
    ctx: &ProjectContext,
    source: &Path,
) -> Result<PathBuf, WorkspaceError> {
    copy_into(ctx, source, ctx.generated_media_dir()).await
}

async fn copy_into(
    ctx: &ProjectContext,
    source: &Path,
    dir: PathBuf,
) -> Result<PathBuf, WorkspaceError> {
    if !fs::try_exists(source).await.unwrap_or(false) {
        return Err(WorkspaceError::MediaNotFound(source.to_path_buf()));
    }
    fs::create_dir_all(&dir)
        .await
        .map_err(|err| WorkspaceError::io(&dir, err))?;

    let name = source
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| WorkspaceError::MediaNotFound(source.to_path_buf()))?;
    let destination = unique_destination(&dir, name).await;

    fs::copy(source, &destination)
        .await
        .map_err(|err| WorkspaceError::io(&destination, err))?;
    Ok(ctx.relativize(&destination))
}

async fn unique_destination(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !fs::try_exists(&candidate).await.unwrap_or(false) {
        return candidate;
    }

    let path = Path::new(name);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("media");
    let extension = path.extension().and_then(|e| e.to_str());

    for index in 1..1000 {
        let attempt = match extension {
            Some(ext) => dir.join(format!("{stem}-{index}.{ext}")),
            None => dir.join(format!("{stem}-{index}")),
        };
        if !fs::try_exists(&attempt).await.unwrap_or(false) {
            return attempt;
        }
    }
    dir.join(format!("{stem}-{}", uuid::Uuid::new_v4().simple()))
}

/// Path a render should be written to, inside `exports/`.
pub async fn export_path(
    ctx: &ProjectContext,
    file_name: &str,
) -> Result<PathBuf, WorkspaceError> {
    let dir = ctx.exports_dir();
    fs::create_dir_all(&dir)
        .await
        .map_err(|err| WorkspaceError::io(&dir, err))?;
    Ok(unique_destination(&dir, file_name).await)
}
