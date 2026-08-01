//! Queues workflows and fetches what they produced.

use crate::ai_tooling::comfyui::models::{
    HistoryEntry, OutputFile, PromptRequest, PromptResponse, Workflow,
};
use crate::ai_tooling::comfyui::{ComfyUiError, Result};
use reqwest::Client;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;
use tokio::time::sleep;

/// Generation can be slow, but a stalled connection should not hang forever.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
/// How often `wait_for_outputs` re-checks history.
const POLL_INTERVAL: Duration = Duration::from_millis(400);

#[derive(Clone)]
pub struct ComfyUiClient {
    http: Client,
    /// Base URL without a trailing slash, e.g. `http://127.0.0.1:8188`.
    base_url: String,
    /// Identifies this app on the socket, so its events can be told apart.
    client_id: String,
}

impl ComfyUiClient {
    /// `base_url` is the ComfyUI server root; `client_id` must match the one
    /// the WebSocket listener connects with, or progress arrives for nobody.
    pub fn new(base_url: impl Into<String>, client_id: impl Into<String>) -> Result<Self> {
        Ok(Self {
            http: Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .map_err(ComfyUiError::from)?,
            base_url: base_url.into().trim_end_matches('/').to_string(),
            client_id: client_id.into(),
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    /// `ws://…/ws?clientId=…`, derived from the HTTP base so both point at the
    /// same server by construction.
    pub fn websocket_url(&self) -> String {
        let ws_base = self
            .base_url
            .replacen("https://", "wss://", 1)
            .replacen("http://", "ws://", 1);
        format!("{ws_base}/ws?clientId={}", self.client_id)
    }

    /// Queues a workflow. Returns the prompt id progress events will carry.
    pub async fn queue_prompt(&self, workflow: Workflow) -> Result<PromptResponse> {
        let body = PromptRequest {
            prompt: workflow,
            client_id: self.client_id.clone(),
        };

        let response = self
            .http
            .post(format!("{}/prompt", self.base_url))
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            return Err(ComfyUiError::Api {
                status: status.as_u16(),
                body: text,
            });
        }

        let parsed: PromptResponse =
            serde_json::from_str(&text).map_err(ComfyUiError::from)?;
        if !parsed.was_accepted() {
            return Err(ComfyUiError::Rejected {
                detail: serde_json::to_string(&parsed.node_errors).unwrap_or_default(),
            });
        }
        Ok(parsed)
    }

    /// History for one prompt, or `None` while it is still queued or running.
    pub async fn history(&self, prompt_id: &str) -> Result<Option<HistoryEntry>> {
        let response = self
            .http
            .get(format!("{}/history/{prompt_id}", self.base_url))
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            return Err(ComfyUiError::Api {
                status: status.as_u16(),
                body: response.text().await.unwrap_or_default(),
            });
        }

        // The endpoint answers `{}` until the prompt leaves the queue.
        let map: HashMap<String, HistoryEntry> = response.json().await?;
        Ok(map.into_iter().find(|(id, _)| id == prompt_id).map(|(_, entry)| entry))
    }

    /// Polls history until the prompt completes. The WebSocket is the fast path
    /// for progress; this is the fallback that guarantees an answer even if the
    /// socket dropped mid-run.
    pub async fn wait_for_outputs(
        &self,
        prompt_id: &str,
        timeout: Duration,
    ) -> Result<Vec<OutputFile>> {
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            if let Some(entry) = self.history(prompt_id).await? {
                if entry.is_complete() {
                    return Ok(entry.all_outputs());
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(ComfyUiError::Timeout {
                    prompt_id: prompt_id.to_string(),
                    seconds: timeout.as_secs(),
                });
            }
            sleep(POLL_INTERVAL).await;
        }
    }

    /// Downloads one output file's bytes.
    pub async fn fetch_output(&self, file: &OutputFile) -> Result<Vec<u8>> {
        let response = self
            .http
            .get(format!("{}/view", self.base_url))
            .query(&[
                ("filename", file.filename.as_str()),
                ("subfolder", file.subfolder.as_str()),
                ("type", file.folder_type.as_str()),
            ])
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            return Err(ComfyUiError::Api {
                status: status.as_u16(),
                body: response.text().await.unwrap_or_default(),
            });
        }
        Ok(response.bytes().await?.to_vec())
    }

    /// Downloads an output straight into `directory`, returning the written path.
    pub async fn download_output(
        &self,
        file: &OutputFile,
        directory: &Path,
    ) -> Result<std::path::PathBuf> {
        let bytes = self.fetch_output(file).await?;
        tokio::fs::create_dir_all(directory).await?;

        let path = directory.join(&file.filename);
        tokio::fs::write(&path, &bytes).await?;
        Ok(path)
    }

    /// Asks the server to drop the running job.
    pub async fn interrupt(&self) -> Result<()> {
        let response = self
            .http
            .post(format!("{}/interrupt", self.base_url))
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            return Err(ComfyUiError::Api {
                status: status.as_u16(),
                body: response.text().await.unwrap_or_default(),
            });
        }
        Ok(())
    }

    /// True when the server answers at all — used to show the service light.
    pub async fn is_reachable(&self) -> bool {
        self.http
            .get(format!("{}/system_stats", self.base_url))
            .timeout(Duration::from_secs(3))
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
    }
}

/// Sets a value inside a workflow node's inputs, e.g. the prompt text or seed.
///
/// Workflows are user-authored graphs, so this addresses nodes by id rather
/// than assuming any particular layout.
pub fn set_node_input(
    workflow: &mut Workflow,
    node_id: &str,
    input: &str,
    value: Value,
) -> Result<()> {
    workflow
        .get_mut(node_id)
        .and_then(|node| node.get_mut("inputs"))
        .and_then(Value::as_object_mut)
        .map(|inputs| {
            inputs.insert(input.to_string(), value);
        })
        .ok_or_else(|| ComfyUiError::Workflow {
            detail: format!("node '{node_id}' has no '{input}' input"),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn client() -> ComfyUiClient {
        ComfyUiClient::new("http://127.0.0.1:8188/", "studio-1").expect("client")
    }

    #[test]
    fn the_socket_url_follows_the_http_base() {
        assert_eq!(
            client().websocket_url(),
            "ws://127.0.0.1:8188/ws?clientId=studio-1"
        );

        let secure = ComfyUiClient::new("https://comfy.example.com", "abc").expect("client");
        assert_eq!(
            secure.websocket_url(),
            "wss://comfy.example.com/ws?clientId=abc"
        );
    }

    #[test]
    fn trailing_slashes_do_not_double_up() {
        assert_eq!(client().base_url(), "http://127.0.0.1:8188");
    }

    #[test]
    fn workflow_inputs_can_be_rewritten_before_queueing() {
        let mut workflow = json!({
            "6": { "class_type": "CLIPTextEncode", "inputs": { "text": "old", "clip": ["4", 1] } }
        });

        set_node_input(&mut workflow, "6", "text", json!("a server rack")).expect("set");
        assert_eq!(workflow["6"]["inputs"]["text"], "a server rack");
        // Untouched inputs survive.
        assert_eq!(workflow["6"]["inputs"]["clip"][0], "4");
    }

    #[test]
    fn addressing_a_missing_node_is_an_error_not_a_silent_no_op() {
        let mut workflow = json!({ "6": { "inputs": { "text": "x" } } });

        let missing_node = set_node_input(&mut workflow, "99", "text", json!("y"));
        assert!(matches!(missing_node, Err(ComfyUiError::Workflow { .. })));

        let no_inputs = set_node_input(&mut json!({ "6": {} }), "6", "text", json!("y"));
        assert!(matches!(no_inputs, Err(ComfyUiError::Workflow { .. })));
    }
}
