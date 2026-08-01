//! Motion analysis: how much the picture changes frame to frame, and where it
//! changes suddenly.
//!
//! The OpenCV decode path sits behind the `visual-analysis` feature so the app
//! builds without a C++ toolchain; the spike maths is always compiled.

pub mod analyzer;
pub mod models;

pub use analyzer::{analyze_motion, analyze_motion_with, AnalysisOptions, ANALYSIS_WIDTH};
pub use models::{MotionSpike, SpikeDetector, SpikeSettings, VisualTimeline};
