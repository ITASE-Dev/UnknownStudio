//! Frame-differencing motion analysis.
//!
//! Decoding is blocking and CPU-bound, so it runs on `spawn_blocking` rather
//! than on an async worker thread.

#[cfg(feature = "visual-analysis")]
use crate::ai_tooling::visual_analysis::models::SpikeDetector;
use crate::ai_tooling::visual_analysis::models::{SpikeSettings, VisualTimeline};
use crate::ai_tooling::AiToolingError;
use std::path::{Path, PathBuf};

/// Analysis width. 4K frames carry no more motion information than 480p ones,
/// and differencing them costs ~50x more per frame.
pub const ANALYSIS_WIDTH: i32 = 480;

/// Every Nth frame is measured. Impacts last longer than a frame, so sampling
/// at ~15fps still catches them at a third of the work.
pub const FRAME_STRIDE: u64 = 2;

/// Assumed frame rate when the container does not report one.
#[cfg(feature = "visual-analysis")]
const FALLBACK_FPS: f64 = 30.0;

#[derive(Debug, Clone, Copy)]
pub struct AnalysisOptions {
    pub width: i32,
    pub stride: u64,
    pub spikes: SpikeSettings,
}

impl Default for AnalysisOptions {
    fn default() -> Self {
        Self {
            width: ANALYSIS_WIDTH,
            stride: FRAME_STRIDE,
            spikes: SpikeSettings::default(),
        }
    }
}

/// Measures motion across a video and returns the spikes it found.
pub async fn analyze_motion(video_path: &Path) -> Result<VisualTimeline, AiToolingError> {
    analyze_motion_with(video_path, AnalysisOptions::default()).await
}

pub async fn analyze_motion_with(
    video_path: &Path,
    options: AnalysisOptions,
) -> Result<VisualTimeline, AiToolingError> {
    let path: PathBuf = video_path.to_path_buf();

    tokio::task::spawn_blocking(move || analyze_blocking(&path, options))
        .await
        .map_err(|err| AiToolingError::Vision(format!("analysis task failed: {err}")))?
}

#[cfg(feature = "visual-analysis")]
fn analyze_blocking(
    video_path: &Path,
    options: AnalysisOptions,
) -> Result<VisualTimeline, AiToolingError> {
    use opencv::core::{absdiff, sum_elems, Mat, Size};
    use opencv::imgproc::{cvt_color_def, resize, COLOR_BGR2GRAY, INTER_AREA};
    use opencv::prelude::*;
    use opencv::videoio::{self, VideoCapture, VideoCaptureTrait, VideoCaptureTraitConst};

    let vision = |message: String| AiToolingError::Vision(message);

    let mut capture = VideoCapture::from_file(&video_path.to_string_lossy(), videoio::CAP_ANY)
        .map_err(|err| vision(err.to_string()))?;
    if !capture.is_opened().map_err(|err| vision(err.to_string()))? {
        return Err(vision(format!("cannot open {}", video_path.display())));
    }

    let fps = match capture.get(videoio::CAP_PROP_FPS) {
        Ok(value) if value.is_finite() && value > 1.0 => value,
        _ => FALLBACK_FPS,
    };

    let mut detector = SpikeDetector::new(options.spikes);
    let (mut frame, mut gray, mut small) = (Mat::default(), Mat::default(), Mat::default());
    let (mut previous, mut difference) = (Mat::default(), Mat::default());
    let mut has_previous = false;
    let mut index: u64 = 0;
    let stride = options.stride.max(1);

    while capture
        .read(&mut frame)
        .map_err(|err| vision(err.to_string()))?
    {
        if frame.empty() {
            break;
        }
        index += 1;
        if index % stride != 0 {
            continue;
        }

        // Grayscale first, then downscale: one channel is a third of the work,
        // and colour tells us nothing about how much moved.
        cvt_color_def(&frame, &mut gray, COLOR_BGR2GRAY).map_err(|err| vision(err.to_string()))?;

        let size = scaled_size(gray.cols(), gray.rows(), options.width);
        resize(&gray, &mut small, size, 0.0, 0.0, INTER_AREA)
            .map_err(|err| vision(err.to_string()))?;

        if has_previous {
            absdiff(&previous, &small, &mut difference).map_err(|err| vision(err.to_string()))?;

            let total = sum_elems(&difference).map_err(|err| vision(err.to_string()))?[0];
            let pixels = (small.rows() * small.cols()).max(1) as f64;
            // Mean absolute change per pixel: comparable across resolutions.
            let intensity = (total / pixels) as f32;

            let timestamp = position_seconds(&capture, index, fps);
            detector.push(timestamp, intensity);
        }

        small
            .copy_to(&mut previous)
            .map_err(|err| vision(err.to_string()))?;
        has_previous = true;
    }

    Ok(detector.finish())
}

/// Media time from the decoder, falling back to frame arithmetic when the
/// container reports no position.
#[cfg(feature = "visual-analysis")]
fn position_seconds(
    capture: &opencv::videoio::VideoCapture,
    frame_index: u64,
    fps: f64,
) -> f32 {
    use opencv::videoio::{self, VideoCaptureTraitConst};

    match capture.get(videoio::CAP_PROP_POS_MSEC) {
        Ok(millis) if millis.is_finite() && millis > 0.0 => (millis / 1000.0) as f32,
        _ => (frame_index as f64 / fps) as f32,
    }
}

/// Target size preserving the source aspect, never upscaling.
#[cfg(feature = "visual-analysis")]
fn scaled_size(width: i32, height: i32, target_width: i32) -> opencv::core::Size {
    use opencv::core::Size;

    if width <= 0 || height <= 0 || width <= target_width {
        return Size::new(width.max(1), height.max(1));
    }
    let scale = target_width as f64 / width as f64;
    Size::new(target_width, ((height as f64 * scale) as i32).max(1))
}

/// Without the `visual-analysis` feature the module still compiles and every
/// caller still type-checks; analysis simply reports that it is unavailable.
#[cfg(not(feature = "visual-analysis"))]
fn analyze_blocking(
    _video_path: &Path,
    _options: AnalysisOptions,
) -> Result<VisualTimeline, AiToolingError> {
    Err(AiToolingError::Vision(
        "motion analysis needs the 'visual-analysis' feature and an OpenCV build".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_missing_file_is_an_error_not_a_panic() {
        let result = analyze_motion(Path::new("no-such-clip.mp4")).await;
        assert!(matches!(result, Err(AiToolingError::Vision(_))));
    }

    #[test]
    fn options_default_to_a_downscaled_stride() {
        let options = AnalysisOptions::default();
        assert_eq!(options.width, ANALYSIS_WIDTH);
        assert!(options.stride >= 1);
    }

    #[cfg(feature = "visual-analysis")]
    #[test]
    fn downscaling_preserves_aspect_and_never_upscales() {
        // 4K 16:9 → 480 wide, still 16:9.
        let uhd = scaled_size(3840, 2160, ANALYSIS_WIDTH);
        assert_eq!((uhd.width, uhd.height), (480, 270));

        // Vertical footage keeps its shape.
        let vertical = scaled_size(1080, 1920, ANALYSIS_WIDTH);
        assert_eq!((vertical.width, vertical.height), (480, 853));

        // Already small: left alone rather than blown up.
        let small = scaled_size(320, 180, ANALYSIS_WIDTH);
        assert_eq!((small.width, small.height), (320, 180));
    }
}
