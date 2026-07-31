use crate::action_engine::tools::command::{ensure_parent_dir, run_ffmpeg, validate_path};
use crate::action_engine::tools::error::{ActionEngineError, ActionResult};
use std::path::{Path, PathBuf};

/// Fit strategy when source and target aspect ratios differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FitMode {
    /// Scale up to cover, then center-crop to exact size (no letterbox).
    #[default]
    Cover,
    /// Scale down to fit, then pad to exact size (letterbox/pillarbox).
    Contain,
    /// Stretch to exact size, ignoring aspect ratio.
    Stretch,
}

#[derive(Debug, Clone, Copy)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

impl Size {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// Common short-form vertical canvas.
    pub const VERTICAL_1080X1920: Self = Self::new(1080, 1920);

    /// Common long-form horizontal canvas.
    pub const HORIZONTAL_1920X1080: Self = Self::new(1920, 1080);

    fn validated(self) -> ActionResult<Self> {
        if self.width == 0 || self.height == 0 {
            return Err(ActionEngineError::invalid(
                "width and height must be > 0",
            ));
        }
        // libx264 prefers even dimensions.
        if self.width % 2 != 0 || self.height % 2 != 0 {
            return Err(ActionEngineError::invalid(
                "width and height must be even",
            ));
        }
        Ok(self)
    }
}

#[derive(Debug, Clone)]
pub struct CropScaleOptions {
    pub size: Size,
    pub fit: FitMode,
    /// Pad color when using [`FitMode::Contain`] (`black`, `white`, or `0xRRGGBB`).
    pub pad_color: String,
}

impl Default for CropScaleOptions {
    fn default() -> Self {
        Self {
            size: Size::VERTICAL_1080X1920,
            fit: FitMode::Cover,
            pad_color: "black".into(),
        }
    }
}

/// Resizes / crops / pads `input` to the target canvas and writes `output`.
pub async fn crop_and_scale(
    input: impl AsRef<Path>,
    output: impl AsRef<Path>,
    options: CropScaleOptions,
) -> ActionResult<PathBuf> {
    let input = input.as_ref();
    let output = output.as_ref();
    validate_path(input, "input")?;
    validate_path(output, "output")?;
    ensure_parent_dir(output).await?;

    let size = options.size.validated()?;
    let vf = build_vf(size, options.fit, &options.pad_color)?;

    run_ffmpeg([
        "-i",
        &path_arg(input),
        "-vf",
        &vf,
        "-c:v",
        "libx264",
        "-preset",
        "veryfast",
        "-crf",
        "18",
        "-c:a",
        "copy",
        &path_arg(output),
    ])
    .await?;

    Ok(output.to_path_buf())
}

/// Convert a 16:9 long-form clip to a 9:16 short via center cover-crop.
pub async fn to_vertical_short(
    input: impl AsRef<Path>,
    output: impl AsRef<Path>,
) -> ActionResult<PathBuf> {
    crop_and_scale(
        input,
        output,
        CropScaleOptions {
            size: Size::VERTICAL_1080X1920,
            fit: FitMode::Cover,
            pad_color: "black".into(),
        },
    )
    .await
}

/// Convert a 9:16 short to a 16:9 canvas with letterboxing.
pub async fn to_horizontal_letterbox(
    input: impl AsRef<Path>,
    output: impl AsRef<Path>,
) -> ActionResult<PathBuf> {
    crop_and_scale(
        input,
        output,
        CropScaleOptions {
            size: Size::HORIZONTAL_1920X1080,
            fit: FitMode::Contain,
            pad_color: "black".into(),
        },
    )
    .await
}

fn build_vf(size: Size, fit: FitMode, pad_color: &str) -> ActionResult<String> {
    let w = size.width;
    let h = size.height;
    let color = sanitize_color(pad_color)?;

    let vf = match fit {
        FitMode::Cover => format!(
            "scale={w}:{h}:force_original_aspect_ratio=increase,\
             crop={w}:{h}"
        ),
        FitMode::Contain => format!(
            "scale={w}:{h}:force_original_aspect_ratio=decrease,\
             pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:color={color}"
        ),
        FitMode::Stretch => format!("scale={w}:{h}"),
    };
    Ok(vf)
}

fn sanitize_color(color: &str) -> ActionResult<&str> {
    let trimmed = color.trim();
    if trimmed.is_empty() {
        return Err(ActionEngineError::invalid("pad_color is empty"));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '#' || c == '_')
    {
        return Err(ActionEngineError::invalid(
            "pad_color contains unsupported characters",
        ));
    }
    Ok(trimmed)
}

fn path_arg(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
