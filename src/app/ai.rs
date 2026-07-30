//! Host-side bridge to `action_engine`. The rest of the app talks to this
//! struct only; nothing outside it knows the engine's internals.

use crate::action_engine::{
    ActionEngine, ActionEvent, ActionRequest, ChatTurn, ClipId, OpenAiProvider, Recommendation,
    SeoStrategy, TimelineEdit, VideoProposal,
};
use std::collections::HashMap;

#[derive(Default)]
pub struct AiBridge {
    engine: Option<ActionEngine>,
    /// Set when the engine could not start (e.g. no API key); shown as a
    /// service state instead of failing the app.
    pub offline_reason: Option<String>,

    pub history: Vec<ChatTurn>,
    /// Transient status line; cleared by the next terminal event.
    pub status: Option<String>,
    pub recommendations: Option<(ClipId, Vec<Recommendation>)>,
    pub seo_strategy: Option<SeoStrategy>,
    pub video_proposal: Option<VideoProposal>,
    /// Transcripts accumulate per clip so the SEO pass never re-transcribes.
    pub transcripts: HashMap<ClipId, String>,
    /// Edits awaiting application by whoever owns the timeline.
    pub pending_edits: Vec<TimelineEdit>,
    /// Clip analysis the model asked for; the host samples frames and submits.
    pub analysis_requests: Vec<ClipId>,
    /// Generated media waiting to be injected into the timeline.
    pub generated_assets: Vec<GeneratedAsset>,
}

pub enum GeneratedAsset {
    Image {
        target_clip_id: ClipId,
        bytes: Vec<u8>,
    },
    Video {
        target_clip_id: Option<ClipId>,
        path: std::path::PathBuf,
    },
}

impl AiBridge {
    pub fn new() -> Self {
        match OpenAiProvider::from_env() {
            Ok(provider) => Self {
                engine: Some(ActionEngine::spawn(provider)),
                ..Default::default()
            },
            Err(err) => Self {
                offline_reason: Some(err.to_string()),
                ..Default::default()
            },
        }
    }

    pub fn is_online(&self) -> bool {
        self.engine.is_some()
    }

    pub fn submit(&mut self, request: ActionRequest) {
        let Some(engine) = &self.engine else {
            self.push_assistant("AI engine is offline.");
            return;
        };
        if let Err(err) = engine.submit(request) {
            self.offline_reason = Some(err.to_string());
        }
    }

    pub fn send_chat(&mut self, prompt: String, target_clip_id: Option<ClipId>) {
        self.history.push(ChatTurn {
            from_user: true,
            text: prompt.clone(),
        });
        let history = self
            .history
            .iter()
            .map(|turn| ChatTurn {
                from_user: turn.from_user,
                text: turn.text.clone(),
            })
            .collect();
        self.submit(ActionRequest::Chat {
            prompt,
            history,
            target_clip_id,
        });
    }

    /// Call once per frame. Non-blocking.
    pub fn poll(&mut self) {
        let Some(engine) = &self.engine else {
            return;
        };
        let events: Vec<ActionEvent> = engine.drain().collect();

        for event in events {
            match event {
                ActionEvent::Progress(status) => self.status = Some(status),
                ActionEvent::ChatReply(text) => {
                    self.status = None;
                    self.push_assistant(text);
                }
                ActionEvent::RunClipAnalysis(clip_id) => {
                    self.status = None;
                    self.analysis_requests.push(clip_id);
                }
                ActionEvent::AnalysisReady {
                    clip_id,
                    recommendations,
                } => {
                    self.status = None;
                    self.push_assistant(format!("{} recommendations found.", recommendations.len()));
                    self.recommendations = Some((clip_id, recommendations));
                }
                ActionEvent::TranscriptReady {
                    clip_id,
                    transcript,
                } => {
                    self.transcripts.insert(clip_id, transcript);
                }
                ActionEvent::SeoStrategyReady(strategy) => {
                    self.status = None;
                    self.seo_strategy = Some(strategy);
                }
                ActionEvent::VideoProposed(proposal) => {
                    self.status = None;
                    self.video_proposal = Some(proposal);
                }
                ActionEvent::ImageReady {
                    target_clip_id,
                    image_bytes,
                } => {
                    self.status = None;
                    self.generated_assets.push(GeneratedAsset::Image {
                        target_clip_id,
                        bytes: image_bytes,
                    });
                }
                ActionEvent::VideoReady {
                    target_clip_id,
                    path,
                } => {
                    self.status = None;
                    self.generated_assets.push(GeneratedAsset::Video {
                        target_clip_id,
                        path,
                    });
                }
                ActionEvent::Edit(edit) => {
                    self.status = None;
                    self.pending_edits.push(edit);
                }
                ActionEvent::Failed(err) => {
                    self.status = None;
                    self.push_assistant(err.to_string());
                }
            }
        }
    }

    fn push_assistant(&mut self, text: impl Into<String>) {
        self.history.push(ChatTurn {
            from_user: false,
            text: text.into(),
        });
    }
}
