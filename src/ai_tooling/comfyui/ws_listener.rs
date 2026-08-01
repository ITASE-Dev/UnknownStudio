//! Background task that turns the ComfyUI socket into UI events.
//!
//! The UI thread never awaits: the listener runs on the Tokio runtime and
//! relays [`ComfyEvent`]s over a `std::sync::mpsc` channel the UI drains with
//! `try_recv` once per frame.

use crate::ai_tooling::comfyui::models::{parse_event, ComfyEvent};
use futures_util::StreamExt;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::runtime::Handle;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

/// Delay before a dropped socket is retried. ComfyUI restarts are common during
/// model swaps, so reconnecting quietly beats surfacing a failure each time.
const RECONNECT_DELAY: Duration = Duration::from_secs(2);

/// Handle held by the UI. Dropping it stops the listener.
pub struct ProgressListener {
    events: Receiver<ComfyEvent>,
    running: Arc<AtomicBool>,
}

impl ProgressListener {
    /// Spawns the listener on `runtime` and returns the handle to poll.
    pub fn spawn(url: String, runtime: &Handle) -> Self {
        let (sender, events) = std::sync::mpsc::channel();
        let running = Arc::new(AtomicBool::new(true));

        runtime.spawn(listen(url, sender, running.clone()));

        Self { events, running }
    }

    /// Non-blocking drain, safe to call every frame.
    pub fn poll(&self) -> impl Iterator<Item = ComfyEvent> + '_ {
        std::iter::from_fn(move || match self.events.try_recv() {
            Ok(event) => Some(event),
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => None,
        })
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
    }
}

impl Drop for ProgressListener {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Connect, relay, reconnect. Returns when the handle is dropped or the UI
/// stops listening.
async fn listen(url: String, sender: Sender<ComfyEvent>, running: Arc<AtomicBool>) {
    while running.load(Ordering::Relaxed) {
        let Ok((stream, _)) = connect_async(&url).await else {
            // Unreachable server: report once, then wait and try again.
            if sender.send(ComfyEvent::Disconnected).is_err() {
                return;
            }
            if !sleep_while_running(&running).await {
                return;
            }
            continue;
        };

        let (_, mut incoming) = stream.split();
        while let Some(Ok(message)) = incoming.next().await {
            if !running.load(Ordering::Relaxed) {
                return;
            }

            // Binary frames are preview images; the UI has its own monitor, so
            // they are skipped rather than decoded.
            let Message::Text(text) = message else {
                continue;
            };
            let Some(event) = parse_event(&text) else {
                continue;
            };
            if sender.send(event).is_err() {
                return; // The UI is gone.
            }
        }

        // The stream ended: the server closed or the connection dropped.
        if sender.send(ComfyEvent::Disconnected).is_err() {
            return;
        }
        if !sleep_while_running(&running).await {
            return;
        }
    }
}

/// Waits out the reconnect delay, checking often enough that a dropped handle
/// stops the task promptly. `false` means the caller should return.
async fn sleep_while_running(running: &AtomicBool) -> bool {
    const STEP: Duration = Duration::from_millis(100);
    let mut waited = Duration::ZERO;

    while waited < RECONNECT_DELAY {
        if !running.load(Ordering::Relaxed) {
            return false;
        }
        tokio::time::sleep(STEP).await;
        waited += STEP;
    }
    running.load(Ordering::Relaxed)
}

/// Progress of one queued prompt, folded from the event stream so the UI can
/// render a bar without tracking frames itself.
#[derive(Debug, Clone, Default)]
pub struct JobProgress {
    pub prompt_id: String,
    pub node: Option<String>,
    pub fraction: f32,
    pub queue_remaining: u64,
    pub finished: bool,
    pub error: Option<String>,
}

impl JobProgress {
    pub fn new(prompt_id: impl Into<String>) -> Self {
        Self {
            prompt_id: prompt_id.into(),
            ..Self::default()
        }
    }

    /// Folds one event in. Events for other prompts are ignored, so several
    /// jobs can share one socket.
    pub fn apply(&mut self, event: &ComfyEvent) {
        if let Some(id) = event.prompt_id() {
            if id != self.prompt_id && !self.prompt_id.is_empty() {
                return;
            }
        }

        match event {
            ComfyEvent::QueueLength { remaining } => self.queue_remaining = *remaining,
            ComfyEvent::NodeStarted { node, .. } => self.node = node.clone(),
            ComfyEvent::Progress { .. } => {
                self.fraction = event.fraction().unwrap_or(self.fraction);
            }
            ComfyEvent::NodeFinished { .. } => {}
            ComfyEvent::PromptFinished { .. } => {
                self.fraction = 1.0;
                self.finished = true;
                self.node = None;
            }
            ComfyEvent::Failed { reason, .. } => {
                self.finished = true;
                self.error = Some(reason.clone());
            }
            ComfyEvent::Disconnected => {
                self.error = Some("lost connection to ComfyUI".into());
            }
        }
    }

    pub fn is_running(&self) -> bool {
        !self.finished && self.error.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn progress(prompt: &str) -> JobProgress {
        JobProgress::new(prompt)
    }

    #[test]
    fn a_run_folds_into_a_single_progress_view() {
        let mut job = progress("p1");

        job.apply(&ComfyEvent::QueueLength { remaining: 2 });
        job.apply(&ComfyEvent::NodeStarted {
            prompt_id: "p1".into(),
            node: Some("7".into()),
        });
        job.apply(&ComfyEvent::Progress {
            prompt_id: "p1".into(),
            value: 5,
            max: 10,
        });

        assert_eq!(job.queue_remaining, 2);
        assert_eq!(job.node.as_deref(), Some("7"));
        assert_eq!(job.fraction, 0.5);
        assert!(job.is_running());

        job.apply(&ComfyEvent::PromptFinished {
            prompt_id: "p1".into(),
        });
        assert_eq!(job.fraction, 1.0);
        assert!(job.finished && job.node.is_none());
        assert!(!job.is_running());
    }

    #[test]
    fn events_for_another_prompt_are_ignored() {
        let mut job = progress("p1");
        job.apply(&ComfyEvent::Progress {
            prompt_id: "p2".into(),
            value: 9,
            max: 10,
        });

        assert_eq!(job.fraction, 0.0, "another job's progress is not ours");
    }

    #[test]
    fn a_failure_stops_the_job_and_keeps_the_reason() {
        let mut job = progress("p1");
        job.apply(&ComfyEvent::Failed {
            prompt_id: "p1".into(),
            reason: "CUDA out of memory".into(),
        });

        assert!(job.finished);
        assert!(!job.is_running());
        assert_eq!(job.error.as_deref(), Some("CUDA out of memory"));
    }

    #[test]
    fn a_dropped_socket_is_reported_without_a_prompt_id() {
        let mut job = progress("p1");
        job.apply(&ComfyEvent::Disconnected);

        assert_eq!(job.error.as_deref(), Some("lost connection to ComfyUI"));
        assert!(!job.finished, "the job may still finish after a reconnect");
    }

    #[test]
    fn queue_length_applies_to_any_job() {
        // Status frames carry no prompt id, so they must not be filtered out.
        let mut job = progress("p1");
        job.apply(&ComfyEvent::QueueLength { remaining: 4 });
        assert_eq!(job.queue_remaining, 4);
    }
}
