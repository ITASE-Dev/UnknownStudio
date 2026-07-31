use crate::action_engine::tools::error::{ActionEngineError, ActionResult};
use std::ffi::OsStr;
use std::path::Path;
use tokio::process::Command;

const FFMPEG_BIN: &str = "ffmpeg";

/// Builds a quiet FFmpeg process (`-hide_banner -v error -y`).
pub fn ffmpeg_command() -> Command {
    let mut cmd = Command::new(FFMPEG_BIN);
    cmd.arg("-hide_banner")
        .arg("-v")
        .arg("error")
        .arg("-y")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    cmd
}

/// Runs FFmpeg with the given arguments (excluding the binary name).
pub async fn run_ffmpeg<I, S>(args: I) -> ActionResult<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut cmd = ffmpeg_command();
    cmd.args(args);

    let status = cmd.status().await?;
    if status.success() {
        Ok(())
    } else {
        Err(ActionEngineError::from_status(status))
    }
}

/// Ensures parent directories for `path` exist.
pub async fn ensure_parent_dir(path: &Path) -> ActionResult<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            tokio::fs::create_dir_all(parent).await?;
        }
    }
    Ok(())
}

/// Formats a non-negative duration in seconds for FFmpeg (`HH:MM:SS.mmm` or plain seconds).
pub fn format_seconds(seconds: f64) -> String {
    if !seconds.is_finite() || seconds < 0.0 {
        return "0".into();
    }
    // Prefer fractional seconds — FFmpeg accepts them everywhere `-ss`/`-t` appear.
    if seconds.fract() == 0.0 {
        format!("{}", seconds as u64)
    } else {
        format!("{seconds:.3}")
    }
}

pub fn validate_path(path: &Path, label: &str) -> ActionResult<()> {
    if path.as_os_str().is_empty() {
        return Err(ActionEngineError::invalid(format!("{label} path is empty")));
    }
    Ok(())
}
