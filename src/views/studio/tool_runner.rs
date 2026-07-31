//! Runs `action_engine::tools` operations off the UI thread.
//!
//! The tools are async FFmpeg processes, so they get their own Tokio runtime.
//! The UI submits a job and polls for the outcome; it never awaits.

use crate::action_engine::tools::{
    self, AudioExtractCodec, ConcatOptions, SpeedOptions, TrimOptions, TrimWindow,
};
use crate::ui::components::timeline::tools::Tool;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use tokio::runtime::Runtime;
use uuid::Uuid;

/// What the UI wants done, resolved to concrete inputs.
pub enum ToolJob {
    Trim {
        input: PathBuf,
        start: f64,
        duration: f64,
    },
    Speed {
        input: PathBuf,
        factor: f64,
    },
    Crop {
        input: PathBuf,
    },
    Overlay {
        base: PathBuf,
        top: PathBuf,
        /// Window on the base clip the overlay covers.
        start: f64,
        end: f64,
    },
    Concat {
        inputs: Vec<PathBuf>,
    },
    ExtractAudio {
        input: PathBuf,
    },
    MuteAudio {
        input: PathBuf,
    },
}

impl ToolJob {
    fn extension(&self) -> &'static str {
        match self {
            Self::ExtractAudio { .. } => "m4a",
            _ => "mp4",
        }
    }
}

pub struct ToolOutcome {
    pub tool: Tool,
    /// Which clip the UI should apply this to (its id at submit time).
    pub clip_id: u64,
    pub result: Result<ToolProduct, String>,
}

pub struct ToolProduct {
    pub path: PathBuf,
    /// New source length when the operation changed it (speed, trim, concat).
    pub seconds: Option<f32>,
    /// The product is a standalone asset rather than a replacement source.
    pub as_new_asset: bool,
}

pub struct ToolRunner {
    runtime: Runtime,
    outcomes_tx: Sender<ToolOutcome>,
    outcomes_rx: Receiver<ToolOutcome>,
    output_dir: PathBuf,
    /// The job in flight, so the strip can show what is running.
    running: Option<Tool>,
}

impl ToolRunner {
    /// `None` when a runtime cannot be created; the strip then stays disabled
    /// rather than taking the whole studio down.
    pub fn new() -> Option<Self> {
        let runtime = Runtime::new().ok()?;
        let (outcomes_tx, outcomes_rx) = mpsc::channel();
        Some(Self {
            runtime,
            outcomes_tx,
            outcomes_rx,
            output_dir: std::env::temp_dir().join("unknown_studio_render"),
            running: None,
        })
    }

    pub fn running(&self) -> Option<Tool> {
        self.running
    }

    pub fn submit(&mut self, tool: Tool, clip_id: u64, job: ToolJob) {
        let output = self
            .output_dir
            .join(format!("{}.{}", Uuid::new_v4(), job.extension()));
        let tx = self.outcomes_tx.clone();
        self.running = Some(tool);

        self.runtime.spawn(async move {
            let result = run(job, output).await;
            let _ = tx.send(ToolOutcome {
                tool,
                clip_id,
                result,
            });
        });
    }

    /// Non-blocking; clears the running marker as outcomes arrive.
    pub fn poll(&mut self) -> Vec<ToolOutcome> {
        let outcomes: Vec<ToolOutcome> = std::iter::from_fn(|| self.outcomes_rx.try_recv().ok()).collect();
        if !outcomes.is_empty() {
            self.running = None;
        }
        outcomes
    }
}

async fn run(job: ToolJob, output: PathBuf) -> Result<ToolProduct, String> {
    let to_error = |err: tools::ActionEngineError| err.to_string();

    match job {
        ToolJob::Trim {
            input,
            start,
            duration,
        } => {
            tools::trim(
                &input,
                &output,
                TrimOptions {
                    window: TrimWindow::start_duration(start, duration),
                    // Frame-accurate: a copy-mode cut snaps to keyframes and
                    // would not match the edit the user made on the timeline.
                    codec: tools::TrimCodecMode::Reencode,
                },
            )
            .await
            .map_err(to_error)?;
            Ok(ToolProduct {
                path: output,
                seconds: Some(duration as f32),
                as_new_asset: false,
            })
        }

        ToolJob::Speed { input, factor } => {
            tools::speed_adjust(&input, &output, SpeedOptions::new(factor))
                .await
                .map_err(to_error)?;
            Ok(ToolProduct {
                path: output,
                seconds: None,
                as_new_asset: false,
            })
        }

        ToolJob::Crop { input } => {
            tools::to_vertical_short(&input, &output)
                .await
                .map_err(to_error)?;
            Ok(ToolProduct {
                path: output,
                seconds: None,
                as_new_asset: false,
            })
        }

        ToolJob::Overlay {
            base,
            top,
            start,
            end,
        } => {
            tools::broll(&base, &top, &output, start, end)
                .await
                .map_err(to_error)?;
            Ok(ToolProduct {
                path: output,
                seconds: None,
                as_new_asset: false,
            })
        }

        ToolJob::Concat { inputs } => {
            tools::concatenate(&inputs, &output, ConcatOptions::default())
                .await
                .map_err(to_error)?;
            Ok(ToolProduct {
                path: output,
                seconds: None,
                as_new_asset: false,
            })
        }

        ToolJob::ExtractAudio { input } => {
            tools::extract_audio(&input, &output, AudioExtractCodec::Aac)
                .await
                .map_err(to_error)?;
            Ok(ToolProduct {
                path: output,
                seconds: None,
                as_new_asset: true,
            })
        }

        ToolJob::MuteAudio { input } => {
            tools::mute(&input, &output).await.map_err(to_error)?;
            Ok(ToolProduct {
                path: output,
                seconds: None,
                as_new_asset: false,
            })
        }
    }
}
