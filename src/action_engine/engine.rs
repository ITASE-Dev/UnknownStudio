//! Worker thread, intent routing and action execution.
//!
//! The engine runs on a dedicated OS thread with blocking I/O and talks to the
//! host over `std::sync::mpsc`. The UI thread only ever calls `submit`
//! (enqueue) and `drain` (non-blocking poll), so a frame is never blocked on
//! network or FFmpeg work.

use crate::action_engine::clip_audio;
use crate::action_engine::edits::{plan_head_trim, FrameRange, TimelineEdit};
use crate::action_engine::filters::extract_eq_spec;
use crate::action_engine::prompts::{
    with_constraint, FACIAL_IDENTITY_CONSTRAINT, INPAINT_IDENTITY_CONSTRAINT,
};
use crate::action_engine::provider::{ActionProvider, ChatContext, ChatOutcome, PacingStats};
use crate::action_engine::types::{
    ActionContext, ActionEvent, ActionKind, ActionRequest, ClipSnapshot, EngineError,
    Recommendation, SeoStrategy, VideoProposal,
};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;

pub struct ActionEngine {
    req_tx: Sender<ActionRequest>,
    evt_rx: Receiver<ActionEvent>,
}

impl ActionEngine {
    pub fn spawn<P: ActionProvider>(provider: P) -> Self {
        let (req_tx, req_rx) = mpsc::channel::<ActionRequest>();
        let (evt_tx, evt_rx) = mpsc::channel::<ActionEvent>();

        thread::spawn(move || {
            let emitter = Emitter(evt_tx);
            while let Ok(request) = req_rx.recv() {
                if !handle(&provider, request, &emitter) {
                    return;
                }
            }
        });

        Self { req_tx, evt_rx }
    }

    /// Enqueues work. Fails only if the worker thread is gone.
    pub fn submit(&self, request: ActionRequest) -> Result<(), EngineError> {
        self.req_tx
            .send(request)
            .map_err(|_| EngineError::Protocol("action engine stopped".into()))
    }

    /// Non-blocking drain of everything the worker has produced so far.
    pub fn drain(&self) -> Drain<'_> {
        Drain(&self.evt_rx)
    }
}

pub struct Drain<'a>(&'a Receiver<ActionEvent>);

impl Iterator for Drain<'_> {
    type Item = ActionEvent;

    fn next(&mut self) -> Option<ActionEvent> {
        match self.0.try_recv() {
            Ok(event) => Some(event),
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => None,
        }
    }
}

struct Emitter(Sender<ActionEvent>);

impl Emitter {
    /// Returns `false` once the host has dropped the engine.
    fn send(&self, event: ActionEvent) -> bool {
        self.0.send(event).is_ok()
    }

    fn progress(&self, status: impl Into<String>) {
        let _ = self.0.send(ActionEvent::Progress(status.into()));
    }

    fn result(&self, outcome: Result<ActionEvent, EngineError>) -> bool {
        self.send(outcome.unwrap_or_else(ActionEvent::Failed))
    }
}

/// Returns `false` when the host is gone and the worker should stop.
fn handle<P: ActionProvider>(provider: &P, request: ActionRequest, out: &Emitter) -> bool {
    match request {
        ActionRequest::Chat {
            prompt,
            history,
            target_clip_id,
        } => {
            out.progress("Thinking…");
            let outcome = provider.chat(ChatContext {
                prompt: &prompt,
                history: &history,
                has_clip: target_clip_id.is_some(),
            });

            match outcome {
                Err(err) => out.send(ActionEvent::Failed(err)),
                Ok(ChatOutcome::Reply(text)) => out.send(ActionEvent::ChatReply(text)),
                Ok(ChatOutcome::AnalyzeClip) => match target_clip_id {
                    Some(clip_id) => out.send(ActionEvent::RunClipAnalysis(clip_id)),
                    None => out.send(ActionEvent::ChatReply(
                        "Select a clip on the timeline first, then I can analyze it.".into(),
                    )),
                },
                // Video generation is never run implicitly: it costs orders of
                // magnitude more than a still, so it becomes a proposal.
                Ok(ChatOutcome::GenerateVideo { prompt, seconds }) => {
                    out.send(ActionEvent::VideoProposed(VideoProposal {
                        prompt,
                        seconds,
                        target_clip_id,
                    }))
                }
                Ok(ChatOutcome::GenerateImage { prompt }) => {
                    let Some(clip_id) = target_clip_id else {
                        return out.send(ActionEvent::ChatReply(
                            "Select a clip on the timeline so I know where to place the image."
                                .into(),
                        ));
                    };
                    out.progress("Generating AI asset…");
                    out.result(
                        generate_image(provider, &prompt, Some(FACIAL_IDENTITY_CONSTRAINT))
                            .map(|image_bytes| ActionEvent::ImageReady {
                                target_clip_id: clip_id,
                                image_bytes,
                            }),
                    )
                }
            }
        }

        ActionRequest::GenerateImage {
            prompt,
            target_clip_id,
        } => {
            out.progress("Generating AI asset…");
            out.result(
                generate_image(provider, &prompt, Some(FACIAL_IDENTITY_CONSTRAINT)).map(
                    |image_bytes| ActionEvent::ImageReady {
                        target_clip_id,
                        image_bytes,
                    },
                ),
            )
        }

        ActionRequest::GenerateVideo {
            prompt,
            seconds,
            target_clip_id,
        } => {
            let prompt = with_constraint(&prompt, Some(INPAINT_IDENTITY_CONSTRAINT));
            let report = |status: String| out.progress(status);
            out.result(
                provider
                    .generate_video(&prompt, seconds, &report)
                    .map(|path| ActionEvent::VideoReady {
                        target_clip_id,
                        path,
                    }),
            )
        }

        ActionRequest::AnalyzeClip {
            clip,
            sampled_frames,
        } => analyze(provider, clip, sampled_frames, out),

        ActionRequest::ExecuteAction { kind, context } => execute(provider, kind, context, out),

        ActionRequest::GenerateSeoStrategy {
            total_duration_seconds,
            total_clips,
            transcript,
        } => {
            if transcript.trim().is_empty() {
                return out.send(ActionEvent::Failed(EngineError::InvalidContext(
                    "transcript",
                )));
            }
            out.progress("Generating YouTube strategy…");
            let stats = PacingStats {
                total_duration_seconds,
                total_clips,
            };
            out.result(
                provider
                    .seo_strategy(stats, &transcript)
                    .and_then(|json| parse_json::<SeoStrategy>(&json))
                    .map(ActionEvent::SeoStrategyReady),
            )
        }
    }
}

fn generate_image<P: ActionProvider>(
    provider: &P,
    prompt: &str,
    constraint: Option<&str>,
) -> Result<Vec<u8>, EngineError> {
    provider.generate_image(&with_constraint(prompt, constraint))
}

/// Ears before eyes: transcribe, publish the transcript for later SEO reuse,
/// then run vision analysis. A failed transcription degrades to visual-only
/// analysis instead of aborting.
fn analyze<P: ActionProvider>(
    provider: &P,
    clip: ClipSnapshot,
    sampled_frames: Vec<Vec<u8>>,
    out: &Emitter,
) -> bool {
    if sampled_frames.is_empty() {
        return out.send(ActionEvent::Failed(EngineError::Media(
            "no sampled frames for analysis".into(),
        )));
    }

    out.progress("Transcribing audio…");
    let transcript = clip_audio::clip_wav(&clip)
        .and_then(|wav| provider.transcribe(wav))
        .unwrap_or_default();

    if !transcript.trim().is_empty()
        && !out.send(ActionEvent::TranscriptReady {
            clip_id: clip.id,
            transcript: transcript.clone(),
        })
    {
        return false;
    }

    out.progress("Analyzing clip for retention issues…");
    out.result(
        provider
            .analyze(&sampled_frames, &transcript)
            .and_then(|json| parse_json::<Vec<Recommendation>>(&json))
            .map(|recommendations| ActionEvent::AnalysisReady {
                clip_id: clip.id,
                recommendations,
            }),
    )
}

/// Executes a user-approved recommendation.
///
/// `Trim` and `TrimSilence` are pure local math — no network call, and no
/// timestamp is ever invented by the model. Everything that can regenerate
/// pixels of a real subject carries an identity constraint.
fn execute<P: ActionProvider>(
    provider: &P,
    kind: ActionKind,
    context: ActionContext,
    out: &Emitter,
) -> bool {
    match kind {
        ActionKind::Trim => out.send(ActionEvent::Edit(plan_head_trim(&context.clip))),

        ActionKind::TrimSilence => {
            out.progress("Scanning for silence…");
            let clip = &context.clip;
            out.result(
                clip_audio::silence_ranges(clip).map(|ranges: Vec<FrameRange>| {
                    ActionEvent::Edit(TimelineEdit::CutRanges {
                        clip_id: clip.id,
                        ranges,
                    })
                }),
            )
        }

        // Color correction re-grades existing frames declaratively: no network
        // call, no identity constraint, no pixels regenerated.
        ActionKind::ColorCorrect => {
            let source = context
                .ai_context
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(&context.proposed_action);
            match extract_eq_spec(source) {
                Some(filter_spec) => out.send(ActionEvent::Edit(TimelineEdit::SetVideoFilter {
                    clip_id: context.clip.id,
                    filter_spec,
                })),
                None => out.send(ActionEvent::Failed(EngineError::InvalidContext(
                    "eq filter values",
                ))),
            }
        }

        // B-roll shows no faces by convention, so it is the one generation path
        // without the identity constraint.
        ActionKind::GenerateBroll | ActionKind::InpaintVideo => {
            let constraint = match kind {
                ActionKind::GenerateBroll => None,
                _ => Some(INPAINT_IDENTITY_CONSTRAINT),
            };
            out.progress("Generating AI asset…");
            out.result(
                generate_image(provider, &context.generation_prompt(), constraint).map(
                    |image_bytes| ActionEvent::ImageReady {
                        target_clip_id: context.clip.id,
                        image_bytes,
                    },
                ),
            )
        }
    }
}

fn parse_json<T: serde::de::DeserializeOwned>(json: &str) -> Result<T, EngineError> {
    serde_json::from_str(json).map_err(|err| EngineError::Protocol(err.to_string()))
}
