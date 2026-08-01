//! Applies parsed model commands to the editor.
//!
//! The model runs on a worker; egui state may only be touched on the UI thread.
//! So this layer is deliberately synchronous and side-effect-free about I/O: it
//! validates a command, calls the host to mutate state, and hands anything
//! heavy back as an [`AsyncJob`] for the caller to queue.
//!
//! The host implements [`EditorState`]; nothing here knows about the UI types.

use crate::ai_tooling::orchestration::models::{TextAnimation, TextStyle};
use serde::{Deserialize, Serialize};
use std::fmt::Write;
use std::sync::mpsc::Sender;
use thiserror::Error;

/// Times closer than this to a clip edge are not a split, they are a no-op.
const MIN_SPLIT_MARGIN: f32 = 0.05;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Marker {
    pub time_sec: f32,
    pub color: String,
    pub label: String,
}

/// One instruction from the model, already parsed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ActionCommand {
    AddMarker {
        time_sec: f32,
        color: String,
        label: String,
    },
    SplitClip {
        clip_id: String,
        time_sec: f32,
    },
    DeleteClip {
        clip_id: String,
    },
    /// Keep only `[start_sec, end_sec)` of a clip, in its own source seconds.
    TrimClip {
        clip_id: String,
        start_sec: f32,
        end_sec: f32,
    },
    MoveClip {
        clip_id: String,
        target_track_idx: usize,
        target_time_sec: f32,
    },
    SetPlayhead {
        time_sec: f32,
    },
    /// A caption spanning `[start_sec, end_sec)`.
    AddText {
        start_sec: f32,
        end_sec: f32,
        text: String,
        #[serde(default)]
        animation: TextAnimation,
        #[serde(default)]
        style: TextStyle,
    },
    /// A sound effect dropped at a point in time.
    AddAudio {
        start_sec: f32,
        /// Pool asset name, or a built-in effect id.
        file_id: String,
        #[serde(default)]
        volume_db: f32,
    },
    PlaceAsset {
        asset: String,
        target_track_idx: usize,
        target_time_sec: f32,
    },
    /// Heavy: generates footage and re-renders. Delegated, never inline.
    RenderBroll {
        clip_id: String,
        prompt: String,
    },
    /// Heavy: encodes the program.
    Export {
        preset: String,
    },
}

impl ActionCommand {
    /// Whether this must go to a background worker rather than mutate state.
    pub fn is_async(&self) -> bool {
        matches!(self, Self::RenderBroll { .. } | Self::Export { .. })
    }

    /// Short name, as the model wrote it.
    pub fn name(&self) -> &'static str {
        match self {
            Self::AddMarker { .. } => "ADD_MARKER",
            Self::SplitClip { .. } => "SPLIT",
            Self::DeleteClip { .. } => "DELETE",
            Self::TrimClip { .. } => "TRIM",
            Self::MoveClip { .. } => "MOVE",
            Self::SetPlayhead { .. } => "SET_PLAYHEAD",
            Self::AddText { .. } => "ADD_TEXT",
            Self::AddAudio { .. } => "ADD_AUDIO",
            Self::PlaceAsset { .. } => "PLACE",
            Self::RenderBroll { .. } => "RENDER_BROLL",
            Self::Export { .. } => "EXPORT",
        }
    }
}

/// Work handed to a background engine. The dispatcher never runs these.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AsyncJob {
    RenderBroll {
        clip_id: String,
        /// Source of the clip the B-roll covers, resolved at dispatch time so
        /// the worker needs no access to editor state.
        source: Option<String>,
        prompt: String,
    },
    Export {
        preset: String,
    },
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum DispatcherError {
    #[error("no clip with id {0}")]
    UnknownClip(String),

    #[error("no asset named {0}")]
    UnknownAsset(String),

    #[error("track {index} does not exist (timeline has {count})")]
    NoSuchTrack { index: usize, count: usize },

    #[error("{value}s is invalid here: {reason}")]
    InvalidTime { value: f32, reason: String },

    /// The host refused: a locked track, a kind mismatch, a full lane.
    #[error("rejected: {0}")]
    Rejected(String),
}

pub type Result<T> = std::result::Result<T, DispatcherError>;

/// The editor, as the dispatcher needs to see it. Implemented by the host so
/// this module stays free of UI types — and so it can be tested without one.
pub trait EditorState {
    fn track_count(&self) -> usize;

    /// `[start, end)` of a clip in timeline seconds.
    fn clip_span(&self, clip_id: &str) -> Option<(f32, f32)>;

    /// Source path or file name behind a clip, when it has one.
    fn clip_source(&self, clip_id: &str) -> Option<String>;

    fn has_asset(&self, name: &str) -> bool;

    fn add_marker(&mut self, marker: Marker) -> Result<()>;
    fn split_clip(&mut self, clip_id: &str, time_sec: f32) -> Result<()>;
    fn delete_clip(&mut self, clip_id: &str) -> Result<()>;
    fn trim_clip(&mut self, clip_id: &str, start_sec: f32, end_sec: f32) -> Result<()>;
    fn move_clip(&mut self, clip_id: &str, track_idx: usize, time_sec: f32) -> Result<()>;
    fn set_playhead(&mut self, time_sec: f32);

    fn add_text(
        &mut self,
        start_sec: f32,
        end_sec: f32,
        text: &str,
        animation: TextAnimation,
        style: TextStyle,
    ) -> Result<()>;

    fn add_audio(&mut self, start_sec: f32, file_id: &str, volume_db: f32) -> Result<()>;
    fn place_asset(&mut self, asset: &str, track_idx: usize, time_sec: f32) -> Result<()>;
}

/// What one command did.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// State changed on the UI thread.
    Applied,
    /// Handed to a worker.
    Deferred(AsyncJob),
}

/// The result of a batch. Partial success is normal: one bad id must not
/// discard the edits around it.
#[derive(Debug, Default)]
pub struct DispatchReport {
    pub applied: Vec<String>,
    pub jobs: Vec<AsyncJob>,
    /// `(command name, why)` — phrased for the model to read back.
    pub failures: Vec<(String, DispatcherError)>,
}

impl DispatchReport {
    pub fn is_empty(&self) -> bool {
        self.applied.is_empty() && self.jobs.is_empty() && self.failures.is_empty()
    }

    pub fn had_failures(&self) -> bool {
        !self.failures.is_empty()
    }

    /// Summary to feed back to the model, so a rejected command can be retried
    /// with the reason in context rather than blindly.
    pub fn feedback(&self) -> String {
        let mut out = String::new();
        if !self.applied.is_empty() {
            let _ = writeln!(out, "applied: {}", self.applied.join(", "));
        }
        if !self.jobs.is_empty() {
            let _ = writeln!(out, "queued: {} background job(s)", self.jobs.len());
        }
        for (name, error) in &self.failures {
            let _ = writeln!(out, "failed {name}: {error}");
        }
        out
    }
}

/// Applies a batch, collecting per-command outcomes. Commands are independent:
/// a failure is recorded and the rest still run.
pub fn apply_actions(
    actions: Vec<ActionCommand>,
    state: &mut dyn EditorState,
) -> DispatchReport {
    let mut report = DispatchReport::default();

    for action in actions {
        let name = action.name().to_string();
        match apply_action(&action, state) {
            Ok(Outcome::Applied) => report.applied.push(name),
            Ok(Outcome::Deferred(job)) => report.jobs.push(job),
            Err(error) => report.failures.push((name, error)),
        }
    }

    report
}

/// Same, forwarding deferred work to a worker as it is produced. Jobs are still
/// listed in the report, so a dead channel is visible rather than silent.
pub fn apply_actions_with_worker(
    actions: Vec<ActionCommand>,
    state: &mut dyn EditorState,
    worker: &Sender<AsyncJob>,
) -> DispatchReport {
    let mut report = apply_actions(actions, state);

    for job in &report.jobs {
        if worker.send(job.clone()).is_err() {
            report.failures.push((
                "worker".to_string(),
                DispatcherError::Rejected("background engine is not running".into()),
            ));
            break;
        }
    }

    report
}

/// Validates one command, then mutates or defers. Validation lives here rather
/// than in the host so every caller reports failures the same way.
pub fn apply_action(action: &ActionCommand, state: &mut dyn EditorState) -> Result<Outcome> {
    match action {
        ActionCommand::AddMarker {
            time_sec,
            color,
            label,
        } => {
            let time_sec = finite_time(*time_sec)?;
            state.add_marker(Marker {
                time_sec,
                color: color.clone(),
                label: label.clone(),
            })?;
            Ok(Outcome::Applied)
        }

        ActionCommand::SplitClip { clip_id, time_sec } => {
            let time_sec = finite_time(*time_sec)?;
            let (start, end) = span(state, clip_id)?;

            // A cut at (or next to) an edge produces a zero-length clip, which
            // is never what was meant.
            if time_sec <= start + MIN_SPLIT_MARGIN || time_sec >= end - MIN_SPLIT_MARGIN {
                return Err(DispatcherError::InvalidTime {
                    value: time_sec,
                    reason: format!("clip {clip_id} spans {start:.2}-{end:.2}"),
                });
            }

            state.split_clip(clip_id, time_sec)?;
            Ok(Outcome::Applied)
        }

        ActionCommand::DeleteClip { clip_id } => {
            span(state, clip_id)?;
            state.delete_clip(clip_id)?;
            Ok(Outcome::Applied)
        }

        ActionCommand::TrimClip {
            clip_id,
            start_sec,
            end_sec,
        } => {
            let start = finite_time(*start_sec)?;
            let end = finite_time(*end_sec)?;
            span(state, clip_id)?;

            if end <= start + MIN_SPLIT_MARGIN {
                return Err(DispatcherError::InvalidTime {
                    value: end,
                    reason: format!("end must be at least {MIN_SPLIT_MARGIN}s after start {start:.2}"),
                });
            }

            state.trim_clip(clip_id, start, end)?;
            Ok(Outcome::Applied)
        }

        ActionCommand::MoveClip {
            clip_id,
            target_track_idx,
            target_time_sec,
        } => {
            let time_sec = finite_time(*target_time_sec)?;
            span(state, clip_id)?;
            track(state, *target_track_idx)?;
            state.move_clip(clip_id, *target_track_idx, time_sec)?;
            Ok(Outcome::Applied)
        }

        ActionCommand::SetPlayhead { time_sec } => {
            state.set_playhead(finite_time(*time_sec)?);
            Ok(Outcome::Applied)
        }

        ActionCommand::AddText {
            start_sec,
            end_sec,
            text,
            animation,
            style,
        } => {
            let start = finite_time(*start_sec)?;
            let end = finite_time(*end_sec)?;

            if text.trim().is_empty() {
                return Err(DispatcherError::Rejected("caption text is empty".into()));
            }
            // A zero-length caption is on screen for no frames at all.
            if end <= start {
                return Err(DispatcherError::InvalidTime {
                    value: end,
                    reason: format!("caption must end after it starts ({start:.2}s)"),
                });
            }

            state.add_text(start, end, text, *animation, *style)?;
            Ok(Outcome::Applied)
        }

        ActionCommand::AddAudio {
            start_sec,
            file_id,
            volume_db,
        } => {
            let start = finite_time(*start_sec)?;
            if file_id.trim().is_empty() {
                return Err(DispatcherError::Rejected("no sound effect named".into()));
            }
            if !volume_db.is_finite() {
                return Err(DispatcherError::Rejected("volume is not a number".into()));
            }

            state.add_audio(start, file_id, *volume_db)?;
            Ok(Outcome::Applied)
        }

        ActionCommand::PlaceAsset {
            asset,
            target_track_idx,
            target_time_sec,
        } => {
            let time_sec = finite_time(*target_time_sec)?;
            if !state.has_asset(asset) {
                return Err(DispatcherError::UnknownAsset(asset.clone()));
            }
            track(state, *target_track_idx)?;
            state.place_asset(asset, *target_track_idx, time_sec)?;
            Ok(Outcome::Applied)
        }

        // Heavy work: validated here, executed elsewhere.
        ActionCommand::RenderBroll { clip_id, prompt } => {
            span(state, clip_id)?;
            Ok(Outcome::Deferred(AsyncJob::RenderBroll {
                clip_id: clip_id.clone(),
                source: state.clip_source(clip_id),
                prompt: prompt.clone(),
            }))
        }

        ActionCommand::Export { preset } => Ok(Outcome::Deferred(AsyncJob::Export {
            preset: preset.clone(),
        })),
    }
}

fn finite_time(value: f32) -> Result<f32> {
    if !value.is_finite() || value < 0.0 {
        return Err(DispatcherError::InvalidTime {
            value,
            reason: "must be a finite, non-negative number of seconds".into(),
        });
    }
    Ok(value)
}

fn span(state: &dyn EditorState, clip_id: &str) -> Result<(f32, f32)> {
    state
        .clip_span(clip_id)
        .ok_or_else(|| DispatcherError::UnknownClip(clip_id.to_string()))
}

fn track(state: &dyn EditorState, index: usize) -> Result<()> {
    let count = state.track_count();
    if index >= count {
        return Err(DispatcherError::NoSuchTrack { index, count });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    /// Minimal host: enough state to prove the validation and the calls.
    #[derive(Default)]
    struct FakeEditor {
        clips: Vec<(String, f32, f32)>,
        assets: Vec<String>,
        tracks: usize,
        markers: Vec<Marker>,
        playhead: f32,
        calls: Vec<String>,
        /// Simulates a locked track refusing the mutation.
        refuse: bool,
    }

    impl FakeEditor {
        fn with_clip() -> Self {
            Self {
                clips: vec![("c1".into(), 0.0, 10.0)],
                assets: vec!["vo.wav".into()],
                tracks: 2,
                ..Self::default()
            }
        }

        fn guard(&mut self, call: &str) -> Result<()> {
            self.calls.push(call.to_string());
            if self.refuse {
                return Err(DispatcherError::Rejected("track is locked".into()));
            }
            Ok(())
        }
    }

    impl EditorState for FakeEditor {
        fn track_count(&self) -> usize {
            self.tracks
        }

        fn clip_span(&self, clip_id: &str) -> Option<(f32, f32)> {
            self.clips
                .iter()
                .find(|(id, _, _)| id == clip_id)
                .map(|(_, start, end)| (*start, *end))
        }

        fn clip_source(&self, clip_id: &str) -> Option<String> {
            self.clip_span(clip_id).map(|_| format!("{clip_id}.mp4"))
        }

        fn has_asset(&self, name: &str) -> bool {
            self.assets.iter().any(|asset| asset == name)
        }

        fn add_marker(&mut self, marker: Marker) -> Result<()> {
            self.guard("add_marker")?;
            self.markers.push(marker);
            Ok(())
        }

        fn split_clip(&mut self, _clip_id: &str, _time_sec: f32) -> Result<()> {
            self.guard("split")
        }

        fn delete_clip(&mut self, clip_id: &str) -> Result<()> {
            self.guard("delete")?;
            self.clips.retain(|(id, _, _)| id != clip_id);
            Ok(())
        }

        fn trim_clip(&mut self, _clip_id: &str, _start: f32, _end: f32) -> Result<()> {
            self.guard("trim")
        }

        fn move_clip(&mut self, _clip_id: &str, _track_idx: usize, _time_sec: f32) -> Result<()> {
            self.guard("move")
        }

        fn set_playhead(&mut self, time_sec: f32) {
            self.calls.push("set_playhead".into());
            self.playhead = time_sec;
        }

        fn add_text(
            &mut self,
            _start: f32,
            _end: f32,
            _text: &str,
            _animation: TextAnimation,
            _style: TextStyle,
        ) -> Result<()> {
            self.guard("add_text")
        }

        fn add_audio(&mut self, _start: f32, _file_id: &str, _volume_db: f32) -> Result<()> {
            self.guard("add_audio")
        }

        fn place_asset(&mut self, _asset: &str, _track: usize, _at: f32) -> Result<()> {
            self.guard("place")
        }
    }

    #[test]
    fn a_marker_lands_with_its_label_and_colour() {
        let mut editor = FakeEditor::with_clip();
        let report = apply_actions(
            vec![ActionCommand::AddMarker {
                time_sec: 4.5,
                color: "red".into(),
                label: "hook ends".into(),
            }],
            &mut editor,
        );

        assert!(!report.had_failures());
        assert_eq!(
            editor.markers,
            vec![Marker {
                time_sec: 4.5,
                color: "red".into(),
                label: "hook ends".into()
            }]
        );
    }

    #[test]
    fn splitting_outside_the_clip_is_refused_with_its_real_span() {
        let mut editor = FakeEditor::with_clip();

        let inside = apply_action(
            &ActionCommand::SplitClip {
                clip_id: "c1".into(),
                time_sec: 4.0,
            },
            &mut editor,
        );
        assert_eq!(inside, Ok(Outcome::Applied));

        // On the edge: would produce a zero-length clip.
        let edge = apply_action(
            &ActionCommand::SplitClip {
                clip_id: "c1".into(),
                time_sec: 10.0,
            },
            &mut editor,
        );
        assert!(matches!(edge, Err(DispatcherError::InvalidTime { .. })));
        // The message carries the span so the model can correct itself.
        assert!(edge.unwrap_err().to_string().contains("0.00-10.00"));
    }

    #[test]
    fn unknown_ids_never_reach_the_host() {
        let mut editor = FakeEditor::with_clip();

        let missing_clip = apply_action(
            &ActionCommand::DeleteClip {
                clip_id: "nope".into(),
            },
            &mut editor,
        );
        assert_eq!(
            missing_clip,
            Err(DispatcherError::UnknownClip("nope".into()))
        );

        let missing_asset = apply_action(
            &ActionCommand::PlaceAsset {
                asset: "ghost.wav".into(),
                target_track_idx: 0,
                target_time_sec: 0.0,
            },
            &mut editor,
        );
        assert_eq!(
            missing_asset,
            Err(DispatcherError::UnknownAsset("ghost.wav".into()))
        );

        assert!(editor.calls.is_empty(), "no mutation was attempted");
    }

    #[test]
    fn out_of_range_tracks_and_times_are_caught() {
        let mut editor = FakeEditor::with_clip();

        let bad_track = apply_action(
            &ActionCommand::MoveClip {
                clip_id: "c1".into(),
                target_track_idx: 9,
                target_time_sec: 0.0,
            },
            &mut editor,
        );
        assert_eq!(
            bad_track,
            Err(DispatcherError::NoSuchTrack { index: 9, count: 2 })
        );

        let bad_time = apply_action(
            &ActionCommand::SetPlayhead {
                time_sec: f32::NAN,
            },
            &mut editor,
        );
        assert!(matches!(bad_time, Err(DispatcherError::InvalidTime { .. })));
        assert!(editor.calls.is_empty());
    }

    #[test]
    fn a_host_refusal_is_reported_not_swallowed() {
        let mut editor = FakeEditor::with_clip();
        editor.refuse = true;

        let report = apply_actions(
            vec![ActionCommand::MoveClip {
                clip_id: "c1".into(),
                target_track_idx: 1,
                target_time_sec: 2.0,
            }],
            &mut editor,
        );

        assert!(report.applied.is_empty());
        assert!(report.feedback().contains("track is locked"));
    }

    #[test]
    fn one_bad_command_does_not_discard_the_good_ones() {
        let mut editor = FakeEditor::with_clip();
        let report = apply_actions(
            vec![
                ActionCommand::SetPlayhead { time_sec: 3.0 },
                ActionCommand::DeleteClip {
                    clip_id: "ghost".into(),
                },
                ActionCommand::AddMarker {
                    time_sec: 1.0,
                    color: "blue".into(),
                    label: "beat".into(),
                },
            ],
            &mut editor,
        );

        assert_eq!(report.applied, vec!["SET_PLAYHEAD", "ADD_MARKER"]);
        assert_eq!(report.failures.len(), 1);
        assert_eq!(editor.playhead, 3.0);
        assert_eq!(editor.markers.len(), 1);

        let feedback = report.feedback();
        assert!(feedback.contains("applied: SET_PLAYHEAD, ADD_MARKER"));
        assert!(feedback.contains("failed DELETE: no clip with id ghost"));
    }

    #[test]
    fn heavy_work_is_deferred_with_its_source_resolved() {
        let mut editor = FakeEditor::with_clip();
        let (tx, rx) = mpsc::channel();

        let report = apply_actions_with_worker(
            vec![
                ActionCommand::RenderBroll {
                    clip_id: "c1".into(),
                    prompt: "server rack diagram".into(),
                },
                ActionCommand::Export {
                    preset: "youtube_1080p".into(),
                },
            ],
            &mut editor,
            &tx,
        );

        assert!(report.applied.is_empty(), "nothing ran inline");
        assert_eq!(report.jobs.len(), 2);
        assert_eq!(
            rx.recv().expect("job reaches the worker"),
            AsyncJob::RenderBroll {
                clip_id: "c1".into(),
                source: Some("c1.mp4".into()),
                prompt: "server rack diagram".into(),
            }
        );
        assert_eq!(
            rx.recv().expect("second job"),
            AsyncJob::Export {
                preset: "youtube_1080p".into()
            }
        );
    }

    #[test]
    fn a_dead_worker_is_surfaced() {
        let mut editor = FakeEditor::with_clip();
        let (tx, rx) = mpsc::channel();
        drop(rx);

        let report = apply_actions_with_worker(
            vec![ActionCommand::Export {
                preset: "draft".into(),
            }],
            &mut editor,
            &tx,
        );

        assert!(report.had_failures());
        assert!(report.feedback().contains("background engine is not running"));
    }

    #[test]
    fn a_backwards_trim_is_refused() {
        let mut editor = FakeEditor::with_clip();
        let backwards = apply_action(
            &ActionCommand::TrimClip {
                clip_id: "c1".into(),
                start_sec: 5.0,
                end_sec: 5.0,
            },
            &mut editor,
        );

        assert!(matches!(backwards, Err(DispatcherError::InvalidTime { .. })));
        assert!(editor.calls.is_empty());

        let forwards = apply_action(
            &ActionCommand::TrimClip {
                clip_id: "c1".into(),
                start_sec: 1.0,
                end_sec: 6.0,
            },
            &mut editor,
        );
        assert_eq!(forwards, Ok(Outcome::Applied));
    }

    #[test]
    fn a_caption_needs_text_and_a_span_to_live_in() {
        let mut editor = FakeEditor::with_clip();

        let good = apply_action(
            &ActionCommand::AddText {
                start_sec: 1.0,
                end_sec: 2.5,
                text: "hello there".into(),
                animation: TextAnimation::Pop,
                style: TextStyle::Highlight,
            },
            &mut editor,
        );
        assert_eq!(good, Ok(Outcome::Applied));

        let empty = apply_action(
            &ActionCommand::AddText {
                start_sec: 1.0,
                end_sec: 2.0,
                text: "   ".into(),
                animation: TextAnimation::None,
                style: TextStyle::Default,
            },
            &mut editor,
        );
        assert!(matches!(empty, Err(DispatcherError::Rejected(_))));

        let zero_length = apply_action(
            &ActionCommand::AddText {
                start_sec: 2.0,
                end_sec: 2.0,
                text: "flash".into(),
                animation: TextAnimation::None,
                style: TextStyle::Default,
            },
            &mut editor,
        );
        assert!(matches!(zero_length, Err(DispatcherError::InvalidTime { .. })));
    }

    #[test]
    fn an_unnamed_sound_effect_is_refused() {
        let mut editor = FakeEditor::with_clip();

        let named = apply_action(
            &ActionCommand::AddAudio {
                start_sec: 3.0,
                file_id: "whoosh".into(),
                volume_db: -12.0,
            },
            &mut editor,
        );
        assert_eq!(named, Ok(Outcome::Applied));

        let unnamed = apply_action(
            &ActionCommand::AddAudio {
                start_sec: 3.0,
                file_id: String::new(),
                volume_db: -12.0,
            },
            &mut editor,
        );
        assert!(matches!(unnamed, Err(DispatcherError::Rejected(_))));
    }

    #[test]
    fn commands_round_trip_through_json() {
        let action = ActionCommand::SplitClip {
            clip_id: "c1".into(),
            time_sec: 4.0,
        };
        let json = serde_json::to_string(&action).expect("serialize");
        assert!(json.contains(r#""action":"split_clip""#));
        assert_eq!(
            serde_json::from_str::<ActionCommand>(&json).expect("deserialize"),
            action
        );
    }
}
