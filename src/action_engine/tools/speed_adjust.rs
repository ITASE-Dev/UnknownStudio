use crate::action_engine::tools::command::{ensure_parent_dir, run_ffmpeg, validate_path};
use crate::action_engine::tools::error::{ActionEngineError, ActionResult};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct SpeedOptions {
    /// Playback rate multiplier. `2.0` = 2× fast-forward, `0.5` = half-speed.
    pub factor: f64,
    /// When true, pitch-preserving `atempo` chain is applied to audio.
    pub adjust_audio: bool,
}

impl SpeedOptions {
    pub fn new(factor: f64) -> Self {
        Self {
            factor,
            adjust_audio: true,
        }
    }

    fn validated(&self) -> ActionResult<f64> {
        if !self.factor.is_finite() || self.factor <= 0.0 {
            return Err(ActionEngineError::invalid(
                "speed factor must be finite and > 0",
            ));
        }
        // Extreme rates produce unusable A/V; keep a practical editing envelope.
        if !(0.125..=8.0).contains(&self.factor) {
            return Err(ActionEngineError::invalid(
                "speed factor must be within [0.125, 8.0]",
            ));
        }
        Ok(self.factor)
    }
}

/// Alters playback speed of `input`, writing the result to `output`.
pub async fn speed_adjust(
    input: impl AsRef<Path>,
    output: impl AsRef<Path>,
    options: SpeedOptions,
) -> ActionResult<PathBuf> {
    let input = input.as_ref();
    let output = output.as_ref();
    validate_path(input, "input")?;
    validate_path(output, "output")?;
    ensure_parent_dir(output).await?;

    let factor = options.validated()?;
    let setpts = format!("setpts=PTS/{factor}");

    if options.adjust_audio {
        let atempo = build_atempo_chain(factor)?;
        let filter = format!("[0:v]{setpts}[v];[0:a]{atempo}[a]");
        run_ffmpeg([
            "-i",
            &path_arg(input),
            "-filter_complex",
            &filter,
            "-map",
            "[v]",
            "-map",
            "[a]",
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-crf",
            "18",
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            &path_arg(output),
        ])
        .await?;
    } else {
        run_ffmpeg([
            "-i",
            &path_arg(input),
            "-filter:v",
            &setpts,
            "-an",
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-crf",
            "18",
            &path_arg(output),
        ])
        .await?;
    }

    Ok(output.to_path_buf())
}

/// Convenience: fast-forward / slow-motion with audio tempo adjustment.
pub async fn change_speed(
    input: impl AsRef<Path>,
    output: impl AsRef<Path>,
    factor: f64,
) -> ActionResult<PathBuf> {
    speed_adjust(input, output, SpeedOptions::new(factor)).await
}

/// `atempo` only accepts `[0.5, 2.0]` per filter instance — chain as needed.
fn build_atempo_chain(factor: f64) -> ActionResult<String> {
    let mut remaining = factor;
    let mut parts: Vec<String> = Vec::new();

    // Normalize into successive factors inside [0.5, 2.0].
    while remaining > 2.0 {
        parts.push("atempo=2.0".into());
        remaining /= 2.0;
    }
    while remaining < 0.5 {
        parts.push("atempo=0.5".into());
        remaining /= 0.5;
    }
    parts.push(format!("atempo={remaining:.6}"));

    if parts.is_empty() {
        return Err(ActionEngineError::invalid("failed to build atempo chain"));
    }
    Ok(parts.join(","))
}

fn path_arg(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
