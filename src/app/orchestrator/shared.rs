//! State both sides touch.
//!
//! Exactly one thing is shared: the read-only projection of the timeline. The
//! live [`TimelineState`](crate::views::studio::timeline_panel::TimelineState)
//! stays owned by the UI thread, because egui reads it dozens of times per
//! frame and a worker holding a write guard across an FFmpeg call would stall
//! the render loop — the freeze this architecture exists to prevent, smuggled
//! back in through a lock.
//!
//! So the sharing is one-directional: the UI **publishes** a snapshot whenever
//! the edit changes, workers **read** it while planning, and mutations travel
//! back as [`AppEvent::ApplyActions`](super::event::AppEvent::ApplyActions) to
//! be applied on the UI thread.

use crate::ai_tooling::revision::timeline::CurrentTimelineState;
use std::sync::{Arc, RwLock};

/// The timeline as the background sees it.
///
/// Cloning shares; there is one snapshot behind any number of handles.
#[derive(Clone, Default)]
pub struct SharedTimeline {
    inner: Arc<RwLock<CurrentTimelineState>>,
}

impl SharedTimeline {
    pub fn new(state: CurrentTimelineState) -> Self {
        Self {
            inner: Arc::new(RwLock::new(state)),
        }
    }

    /// Replaces the snapshot. Called by the UI thread, once per change.
    ///
    /// The write guard covers a move and nothing else, so a reader waits for
    /// the length of a pointer swap.
    pub fn publish(&self, state: CurrentTimelineState) {
        match self.inner.write() {
            Ok(mut guard) => *guard = state,
            // A poisoned lock means a reader panicked while holding it. The
            // data is a snapshot that is about to be overwritten wholesale, so
            // there is nothing to salvage and nothing to corrupt.
            Err(poisoned) => *poisoned.into_inner() = state,
        }
    }

    /// An owned copy of the snapshot.
    ///
    /// Deliberately returns a clone rather than a guard: an `RwLockReadGuard`
    /// is not `Send`, and even if it were, holding one across an `.await`
    /// inside a handler is how this design would deadlock against `publish`.
    /// Making that impossible is worth the copy — the projection is a few
    /// hundred bytes per clip, read once per job, not per frame.
    pub fn snapshot(&self) -> CurrentTimelineState {
        match self.inner.read() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// Whether the edit has anything in it, without cloning.
    pub fn is_empty(&self) -> bool {
        match self.inner.read() {
            Ok(guard) => guard.is_empty(),
            Err(poisoned) => poisoned.into_inner().is_empty(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_tooling::revision::timeline::{ClipRole, ClipView, TrackRole, TrackView};

    fn populated(duration: f32) -> CurrentTimelineState {
        CurrentTimelineState {
            tracks: vec![TrackView {
                index: 0,
                name: "V1".into(),
                role: TrackRole::Video,
                locked: false,
                clips: vec![ClipView {
                    id: 1,
                    label: "a.mp4".into(),
                    start_sec: 0.0,
                    end_sec: duration,
                    role: ClipRole::ARoll,
                }],
            }],
            caption_spans: Vec::new(),
            duration_sec: duration,
        }
    }

    #[test]
    fn a_publish_is_visible_through_every_handle() {
        let ui_side = SharedTimeline::default();
        let worker_side = ui_side.clone();

        assert!(worker_side.is_empty());
        ui_side.publish(populated(30.0));

        assert_eq!(worker_side.snapshot().duration_sec, 30.0);
        assert!(!worker_side.is_empty());
    }

    #[test]
    fn the_snapshot_is_a_copy_so_a_later_publish_cannot_change_it_underneath() {
        let shared = SharedTimeline::new(populated(30.0));
        let taken = shared.snapshot();

        shared.publish(populated(90.0));

        assert_eq!(taken.duration_sec, 30.0, "the job plans against what it read");
        assert_eq!(shared.snapshot().duration_sec, 90.0);
    }

    #[test]
    fn readers_and_writers_from_several_threads_agree_on_a_final_value() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::thread;

        let shared = SharedTimeline::new(populated(1.0));
        let stop = Arc::new(AtomicBool::new(false));

        // Two readers hammering while a writer republishes: the point is that
        // this terminates rather than deadlocking, and that every read returns
        // one of the published values, never a torn one.
        let readers: Vec<_> = (0..2)
            .map(|_| {
                let shared = shared.clone();
                let stop = stop.clone();
                thread::spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        let seen = shared.snapshot().duration_sec;
                        assert!(seen >= 1.0, "never a partially written value");
                    }
                })
            })
            .collect();

        for i in 2..200 {
            shared.publish(populated(i as f32));
        }
        stop.store(true, Ordering::Relaxed);
        for reader in readers {
            reader.join().expect("reader panicked");
        }

        assert_eq!(shared.snapshot().duration_sec, 199.0);
    }

    #[test]
    fn a_poisoned_lock_is_recovered_rather_than_propagated() {
        use std::panic::{catch_unwind, AssertUnwindSafe};

        let shared = SharedTimeline::new(populated(10.0));
        let inner = shared.clone();

        // Poison it: panic while holding the write guard.
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let _guard = inner.inner.write().expect("write");
            panic!("simulated panic under the lock");
        }));

        // A cache that stops working because a thread died once is worse than
        // the panic was.
        shared.publish(populated(20.0));
        assert_eq!(shared.snapshot().duration_sec, 20.0);
    }
}
