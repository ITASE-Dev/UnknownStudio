//! Wire types for ComfyUI's HTTP API and WebSocket stream.
//!
//! Workflows stay `serde_json::Value`: node graphs are user-authored and their
//! shape changes with every custom node, so typing them would only get in the
//! way. Everything the client reasons about is typed.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// A workflow graph in ComfyUI's API format: node id → node definition.
pub type Workflow = Value;

/// `POST /prompt` body.
#[derive(Debug, Clone, Serialize)]
pub struct PromptRequest {
    pub prompt: Workflow,
    pub client_id: String,
}

/// `POST /prompt` reply.
#[derive(Debug, Clone, Deserialize)]
pub struct PromptResponse {
    pub prompt_id: String,
    #[serde(default)]
    pub number: i64,
    /// Node id → validation errors, present when the graph was rejected.
    #[serde(default)]
    pub node_errors: HashMap<String, Value>,
}

impl PromptResponse {
    pub fn was_accepted(&self) -> bool {
        self.node_errors.is_empty()
    }
}

/// One output file ComfyUI wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputFile {
    pub filename: String,
    #[serde(default)]
    pub subfolder: String,
    /// `output`, `temp` or `input`.
    #[serde(default = "default_output_type", rename = "type")]
    pub folder_type: String,
}

fn default_output_type() -> String {
    "output".to_string()
}

/// `GET /history/{prompt_id}` entry, reduced to the outputs.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct HistoryEntry {
    /// Node id → that node's outputs.
    #[serde(default)]
    pub outputs: HashMap<String, NodeOutputs>,
    #[serde(default)]
    pub status: Option<HistoryStatus>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct NodeOutputs {
    #[serde(default)]
    pub images: Vec<OutputFile>,
    #[serde(default)]
    pub gifs: Vec<OutputFile>,
    #[serde(default)]
    pub videos: Vec<OutputFile>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct HistoryStatus {
    #[serde(default)]
    pub completed: bool,
    #[serde(default)]
    pub status_str: String,
}

impl HistoryEntry {
    /// Every produced file, whatever node made it and whatever kind it is.
    pub fn all_outputs(&self) -> Vec<OutputFile> {
        self.outputs
            .values()
            .flat_map(|node| {
                node.images
                    .iter()
                    .chain(node.gifs.iter())
                    .chain(node.videos.iter())
                    .cloned()
            })
            .collect()
    }

    pub fn is_complete(&self) -> bool {
        self.status.as_ref().is_some_and(|status| status.completed)
    }
}

/// A WebSocket frame from ComfyUI, in its own envelope.
#[derive(Debug, Clone, Deserialize)]
pub struct WsEnvelope {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub data: Value,
}

/// Progress the UI can render. Anything ComfyUI sends that does not affect the
/// UI (`crystools.monitor`, previews, …) never becomes one of these.
#[derive(Debug, Clone, PartialEq)]
pub enum ComfyEvent {
    /// Jobs still queued, including the running one.
    QueueLength { remaining: u64 },
    /// A node started running. `None` means the graph finished.
    NodeStarted {
        prompt_id: String,
        node: Option<String>,
    },
    /// Sampling progress within the running node.
    Progress {
        prompt_id: String,
        value: u64,
        max: u64,
    },
    /// A node produced output.
    NodeFinished {
        prompt_id: String,
        node: String,
        outputs: Vec<OutputFile>,
    },
    /// The whole prompt is done; its outputs are ready to fetch.
    PromptFinished { prompt_id: String },
    /// Execution stopped early.
    Failed {
        prompt_id: String,
        reason: String,
    },
    /// The socket dropped; the listener will not send more without a reconnect.
    Disconnected,
}

impl ComfyEvent {
    /// `0.0..=1.0` for events that carry a measurable fraction.
    pub fn fraction(&self) -> Option<f32> {
        match self {
            Self::Progress { value, max, .. } if *max > 0 => {
                Some((*value as f32 / *max as f32).clamp(0.0, 1.0))
            }
            Self::PromptFinished { .. } => Some(1.0),
            _ => None,
        }
    }

    pub fn prompt_id(&self) -> Option<&str> {
        match self {
            Self::NodeStarted { prompt_id, .. }
            | Self::Progress { prompt_id, .. }
            | Self::NodeFinished { prompt_id, .. }
            | Self::PromptFinished { prompt_id }
            | Self::Failed { prompt_id, .. } => Some(prompt_id),
            _ => None,
        }
    }
}

/// Translates one frame into a UI event, or `None` when it carries nothing the
/// UI needs.
pub fn parse_event(raw: &str) -> Option<ComfyEvent> {
    let envelope: WsEnvelope = serde_json::from_str(raw).ok()?;
    let data = &envelope.data;
    let prompt_id = || {
        data.get("prompt_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };

    match envelope.kind.as_str() {
        "status" => {
            let remaining = data
                .pointer("/status/exec_info/queue_remaining")
                .and_then(Value::as_u64)?;
            Some(ComfyEvent::QueueLength { remaining })
        }

        "executing" => {
            let node = data.get("node").and_then(Value::as_str).map(str::to_string);
            // A null node marks the end of the graph.
            match node {
                None => Some(ComfyEvent::PromptFinished {
                    prompt_id: prompt_id(),
                }),
                node => Some(ComfyEvent::NodeStarted {
                    prompt_id: prompt_id(),
                    node,
                }),
            }
        }

        "progress" => Some(ComfyEvent::Progress {
            prompt_id: prompt_id(),
            value: data.get("value").and_then(Value::as_u64).unwrap_or(0),
            max: data.get("max").and_then(Value::as_u64).unwrap_or(0),
        }),

        "executed" => {
            let node = data.get("node").and_then(Value::as_str)?.to_string();
            let outputs = data
                .get("output")
                .map(|output| {
                    ["images", "gifs", "videos"]
                        .iter()
                        .filter_map(|key| output.get(*key))
                        .filter_map(Value::as_array)
                        .flatten()
                        .filter_map(|file| serde_json::from_value(file.clone()).ok())
                        .collect()
                })
                .unwrap_or_default();

            Some(ComfyEvent::NodeFinished {
                prompt_id: prompt_id(),
                node,
                outputs,
            })
        }

        "execution_success" => Some(ComfyEvent::PromptFinished {
            prompt_id: prompt_id(),
        }),

        "execution_error" | "execution_interrupted" => Some(ComfyEvent::Failed {
            prompt_id: prompt_id(),
            reason: data
                .get("exception_message")
                .and_then(Value::as_str)
                .unwrap_or("execution interrupted")
                .to_string(),
        }),

        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_status_reports_what_is_left() {
        let event = parse_event(
            r#"{"type":"status","data":{"status":{"exec_info":{"queue_remaining":3}}}}"#,
        );
        assert_eq!(event, Some(ComfyEvent::QueueLength { remaining: 3 }));
    }

    #[test]
    fn a_null_executing_node_means_the_graph_finished() {
        let running = parse_event(r#"{"type":"executing","data":{"node":"7","prompt_id":"p1"}}"#);
        assert_eq!(
            running,
            Some(ComfyEvent::NodeStarted {
                prompt_id: "p1".into(),
                node: Some("7".into())
            })
        );

        let done = parse_event(r#"{"type":"executing","data":{"node":null,"prompt_id":"p1"}}"#);
        assert_eq!(
            done,
            Some(ComfyEvent::PromptFinished {
                prompt_id: "p1".into()
            })
        );
    }

    #[test]
    fn progress_becomes_a_fraction() {
        let event = parse_event(
            r#"{"type":"progress","data":{"value":5,"max":20,"prompt_id":"p1"}}"#,
        )
        .expect("event");

        assert_eq!(event.fraction(), Some(0.25));
        assert_eq!(event.prompt_id(), Some("p1"));

        // A zero maximum must not divide by zero.
        let unknown = parse_event(r#"{"type":"progress","data":{"value":0,"max":0}}"#)
            .expect("event");
        assert_eq!(unknown.fraction(), None);
    }

    #[test]
    fn executed_frames_carry_every_output_kind() {
        let raw = r#"{"type":"executed","data":{"node":"9","prompt_id":"p1","output":{
            "images":[{"filename":"a.png","subfolder":"","type":"output"}],
            "gifs":[{"filename":"b.webp","subfolder":"anim","type":"output"}]
        }}}"#;

        let event = parse_event(raw).expect("event");
        let ComfyEvent::NodeFinished { outputs, node, .. } = event else {
            panic!("expected NodeFinished");
        };
        assert_eq!(node, "9");
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs[0].filename, "a.png");
        assert_eq!(outputs[1].subfolder, "anim");
    }

    #[test]
    fn errors_carry_their_message() {
        let event = parse_event(
            r#"{"type":"execution_error","data":{"prompt_id":"p1","exception_message":"OOM"}}"#,
        );
        assert_eq!(
            event,
            Some(ComfyEvent::Failed {
                prompt_id: "p1".into(),
                reason: "OOM".into()
            })
        );
    }

    #[test]
    fn noise_and_malformed_frames_are_ignored() {
        assert!(parse_event(r#"{"type":"crystools.monitor","data":{"cpu":12}}"#).is_none());
        assert!(parse_event("not json").is_none());
        assert!(parse_event(r#"{"type":"status","data":{}}"#).is_none());
    }

    #[test]
    fn history_gathers_outputs_across_nodes() {
        let entry: HistoryEntry = serde_json::from_str(
            r#"{"status":{"completed":true,"status_str":"success"},"outputs":{
                "9":{"images":[{"filename":"a.png"}]},
                "12":{"gifs":[{"filename":"b.webp","subfolder":"anim"}]}
            }}"#,
        )
        .expect("parse");

        assert!(entry.is_complete());
        let mut names: Vec<String> = entry
            .all_outputs()
            .into_iter()
            .map(|file| file.filename)
            .collect();
        names.sort();
        assert_eq!(names, vec!["a.png", "b.webp"]);
        // The default folder is where ComfyUI writes finished work.
        assert_eq!(entry.all_outputs()[0].folder_type, "output");
    }

    #[test]
    fn a_rejected_graph_is_visible_in_the_response() {
        let rejected: PromptResponse = serde_json::from_str(
            r#"{"prompt_id":"p1","number":1,"node_errors":{"3":{"message":"missing model"}}}"#,
        )
        .expect("parse");
        assert!(!rejected.was_accepted());

        let accepted: PromptResponse =
            serde_json::from_str(r#"{"prompt_id":"p2","number":2}"#).expect("parse");
        assert!(accepted.was_accepted());
    }
}
