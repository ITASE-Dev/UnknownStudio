use crate::action_engine::tools::command::{
    ensure_parent_dir, format_seconds, run_ffmpeg, validate_path,
};
use crate::action_engine::tools::error::{ActionEngineError, ActionResult};
use std::path::{Path, PathBuf};

/// How the trim window is specified.
#[derive(Debug, Clone, Copy)]
pub enum TrimWindow {
    /// `[start, start + duration)`.
    StartDuration { start: f64, duration: f64 },
    /// `[start, end)`.
    StartEnd { start: f64, end: f64 },
}

impl TrimWindow {
    pub fn start_duration(start: f64, duration: f64) -> Self {
        Self::StartDuration { start, duration }
    }

    pub fn start_end(start: f64, end: f64) -> Self {
        Self::StartEnd { start, end }
    }

    fn validated(self) -> ActionResult<(f64, f64)> {
        let (start, duration) = match self {
            Self::StartDuration { start, duration } => (start, duration),
            Self::StartEnd { start, end } => (start, end - start),
        };

        if !start.is_finite() || start < 0.0 {
            return Err(ActionEngineError::invalid("trim start must be >= 0"));
        }
        if !duration.is_finite() || duration <= 0.0 {
            return Err(ActionEngineError::invalid(
                "trim duration must be finite and > 0",
            ));
        }
        Ok((start, duration))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrimCodecMode {
    /// Stream copy when possible (fast, keyframe-aligned).
    #[default]
    Copy,
    /// Re-encode for frame-accurate cuts.
    Reencode,
}

#[derive(Debug, Clone)]
pub struct TrimOptions {
    pub window: TrimWindow,
    pub codec: TrimCodecMode,
}

/// Extracts `[start, start+duration)` (or `[start, end)`) from `input` into `output`.
pub async fn trim(
    input: impl AsRef<Path>,
    output: impl AsRef<Path>,
    options: TrimOptions,
) -> ActionResult<PathBuf> {
    let input = input.as_ref();
    let output = output.as_ref();
    validate_path(input, "input")?;
    validate_path(output, "output")?;
    ensure_parent_dir(output).await?;

    let (start, duration) = options.window.validated()?;
    let start_s = format_seconds(start);
    let duration_s = format_seconds(duration);

    let mut args: Vec<String> = Vec::with_capacity(16);
    // Input-side seek is faster; acceptable for Copy. For Reencode we still seek
    // input-side then decode, which is accurate enough for editing workflows.
    args.push("-ss".into());
    args.push(start_s);
    args.push("-i".into());
    args.push(path_arg(input));
    args.push("-t".into());
    args.push(duration_s);

    match options.codec {
        TrimCodecMode::Copy => {
            args.push("-c".into());
            args.push("copy".into());
            args.push("-avoid_negative_ts".into());
            args.push("make_zero".into());
        }
        TrimCodecMode::Reencode => {
            args.push("-c:v".into());
            args.push("libx264".into());
            args.push("-preset".into());
            args.push("veryfast".into());
            args.push("-crf".into());
            args.push("18".into());
            args.push("-c:a".into());
            args.push("aac".into());
            args.push("-b:a".into());
            args.push("192k".into());
        }
    }

    args.push(path_arg(output));
    run_ffmpeg(&args).await?;
    Ok(output.to_path_buf())
}

/// Convenience: trim with stream copy.
pub async fn trim_copy(
    input: impl AsRef<Path>,
    output: impl AsRef<Path>,
    start: f64,
    duration: f64,
) -> ActionResult<PathBuf> {
    trim(
        input,
        output,
        TrimOptions {
            window: TrimWindow::start_duration(start, duration),
            codec: TrimCodecMode::Copy,
        },
    )
    .await
}

/// Convenience: frame-accurate re-encode trim.
pub async fn trim_accurate(
    input: impl AsRef<Path>,
    output: impl AsRef<Path>,
    start: f64,
    duration: f64,
) -> ActionResult<PathBuf> {
    trim(
        input,
        output,
        TrimOptions {
            window: TrimWindow::start_duration(start, duration),
            codec: TrimCodecMode::Reencode,
        },
    )
    .await
}

fn path_arg(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
