use crate::action_engine::tools::command::{
    ensure_parent_dir, format_seconds, run_ffmpeg, validate_path,
};
use crate::action_engine::tools::error::{ActionEngineError, ActionResult};
use std::path::{Path, PathBuf};

/// Placement of the overlay relative to the base frame.
#[derive(Debug, Clone, Copy)]
pub enum OverlayPosition {
    Absolute { x: i32, y: i32 },
    TopLeft { margin: i32 },
    TopRight { margin: i32 },
    BottomLeft { margin: i32 },
    BottomRight { margin: i32 },
    Center,
}

impl Default for OverlayPosition {
    fn default() -> Self {
        Self::BottomRight { margin: 24 }
    }
}

impl OverlayPosition {
    fn to_xy_expr(self) -> (String, String) {
        match self {
            Self::Absolute { x, y } => (x.to_string(), y.to_string()),
            Self::TopLeft { margin } => (margin.to_string(), margin.to_string()),
            Self::TopRight { margin } => (format!("W-w-{margin}"), margin.to_string()),
            Self::BottomLeft { margin } => (margin.to_string(), format!("H-h-{margin}")),
            Self::BottomRight { margin } => {
                (format!("W-w-{margin}"), format!("H-h-{margin}"))
            }
            Self::Center => ("(W-w)/2".into(), "(H-h)/2".into()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct OverlayOptions {
    pub position: OverlayPosition,
    /// Optional overlay width in pixels (height scales to preserve aspect).
    pub overlay_width: Option<u32>,
    /// Opacity in `[0.0, 1.0]`.
    pub opacity: f64,
    /// Delay before the overlay appears (seconds).
    pub start: f64,
    /// Optional end time (seconds). `None` = through end of base (or overlay).
    pub end: Option<f64>,
    /// Loop a short overlay (image/video) for the full base duration.
    pub loop_overlay: bool,
}

impl Default for OverlayOptions {
    fn default() -> Self {
        Self {
            position: OverlayPosition::default(),
            overlay_width: None,
            opacity: 1.0,
            start: 0.0,
            end: None,
            loop_overlay: false,
        }
    }
}

impl OverlayOptions {
    fn validated(&self) -> ActionResult<()> {
        if !(0.0..=1.0).contains(&self.opacity) || !self.opacity.is_finite() {
            return Err(ActionEngineError::invalid(
                "opacity must be finite and within [0.0, 1.0]",
            ));
        }
        if !self.start.is_finite() || self.start < 0.0 {
            return Err(ActionEngineError::invalid("start must be >= 0"));
        }
        if let Some(end) = self.end {
            if !end.is_finite() || end <= self.start {
                return Err(ActionEngineError::invalid(
                    "end must be finite and greater than start",
                ));
            }
        }
        if let Some(w) = self.overlay_width {
            if w == 0 {
                return Err(ActionEngineError::invalid("overlay_width must be > 0"));
            }
        }
        Ok(())
    }
}

/// Superimposes `overlay` (image or video) onto `base`, writing `output`.
pub async fn overlay(
    base: impl AsRef<Path>,
    overlay: impl AsRef<Path>,
    output: impl AsRef<Path>,
    options: OverlayOptions,
) -> ActionResult<PathBuf> {
    let base = base.as_ref();
    let overlay = overlay.as_ref();
    let output = output.as_ref();
    validate_path(base, "base")?;
    validate_path(overlay, "overlay")?;
    validate_path(output, "output")?;
    ensure_parent_dir(output).await?;
    options.validated()?;

    let (x, y) = options.position.to_xy_expr();
    let mut prep: Vec<String> = Vec::new();

    if let Some(w) = options.overlay_width {
        prep.push(format!("scale={w}:-1"));
    }
    if (options.opacity - 1.0).abs() > f64::EPSILON {
        prep.push("format=rgba".into());
        prep.push(format!(
            "colorchannelmixer=aa={opacity:.4}",
            opacity = options.opacity
        ));
    }

    let enable = match options.end {
        Some(end) => format!(
            ":enable='between(t,{},{})'",
            format_seconds(options.start),
            format_seconds(end)
        ),
        None if options.start > 0.0 => {
            format!(":enable='gte(t,{})'", format_seconds(options.start))
        }
        None => String::new(),
    };

    let overlay_label = if prep.is_empty() {
        "1:v".to_string()
    } else {
        "ov".to_string()
    };

    let filter = if prep.is_empty() {
        format!("[0:v][{overlay_label}]overlay={x}:{y}:format=auto{enable}[v]")
    } else {
        format!(
            "[1:v]{}[ov];[0:v][ov]overlay={x}:{y}:format=auto{enable}[v]",
            prep.join(",")
        )
    };

    let mut args: Vec<String> = Vec::with_capacity(20);
    args.push("-i".into());
    args.push(path_arg(base));

    if options.loop_overlay {
        // `-stream_loop -1` before the overlay input; `-shortest` keeps base length.
        args.push("-stream_loop".into());
        args.push("-1".into());
    }

    args.push("-i".into());
    args.push(path_arg(overlay));
    args.push("-filter_complex".into());
    args.push(filter);
    args.push("-map".into());
    args.push("[v]".into());
    args.push("-map".into());
    args.push("0:a?".into());
    args.push("-c:v".into());
    args.push("libx264".into());
    args.push("-preset".into());
    args.push("veryfast".into());
    args.push("-crf".into());
    args.push("18".into());
    args.push("-c:a".into());
    args.push("copy".into());

    if options.loop_overlay {
        args.push("-shortest".into());
    }

    args.push(path_arg(output));
    run_ffmpeg(&args).await?;
    Ok(output.to_path_buf())
}

/// Convenience: watermark image in the bottom-right corner.
pub async fn watermark(
    base: impl AsRef<Path>,
    image: impl AsRef<Path>,
    output: impl AsRef<Path>,
    width: u32,
    opacity: f64,
) -> ActionResult<PathBuf> {
    overlay(
        base,
        image,
        output,
        OverlayOptions {
            position: OverlayPosition::BottomRight { margin: 24 },
            overlay_width: Some(width.max(1)),
            opacity,
            start: 0.0,
            end: None,
            loop_overlay: true,
        },
    )
    .await
}

/// Convenience: full-frame B-roll overlay for a time window.
pub async fn broll(
    base: impl AsRef<Path>,
    broll: impl AsRef<Path>,
    output: impl AsRef<Path>,
    start: f64,
    end: f64,
) -> ActionResult<PathBuf> {
    overlay(
        base,
        broll,
        output,
        OverlayOptions {
            position: OverlayPosition::Absolute { x: 0, y: 0 },
            overlay_width: None,
            opacity: 1.0,
            start,
            end: Some(end),
            loop_overlay: false,
        },
    )
    .await
}

fn path_arg(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
