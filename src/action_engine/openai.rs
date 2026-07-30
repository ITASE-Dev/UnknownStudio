//! OpenAI implementation of `ActionProvider`. Blocking on purpose: it runs on
//! the engine's own worker thread.

use crate::action_engine::prompts::{
    chat_system_prompt, RETENTION_CONSULTANT_SYSTEM_PROMPT, SEO_STRATEGIST_SYSTEM_PROMPT,
};
use crate::action_engine::provider::{
    ActionProvider, ChatContext, ChatOutcome, PacingStats, Progress,
};
use crate::action_engine::types::EngineError;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use uuid::Uuid;

const CHAT_COMPLETIONS_ENDPOINT: &str = "https://api.openai.com/v1/chat/completions";
const IMAGE_GENERATIONS_ENDPOINT: &str = "https://api.openai.com/v1/images/generations";
const VIDEOS_ENDPOINT: &str = "https://api.openai.com/v1/videos";
const TRANSCRIPTIONS_ENDPOINT: &str = "https://api.openai.com/v1/audio/transcriptions";

const CHAT_MODEL: &str = "gpt-4o";
const VISION_MODEL: &str = "gpt-4o";
const TRANSCRIBE_MODEL: &str = "whisper-1";
const DEFAULT_IMAGE_MODEL: &str = "gpt-image-1";

/// Verified `POST /v1/videos` parameter set: `size` ∈ {720x1280, 1280x720,
/// 1024x1792, 1792x1024}, `seconds` ∈ {4, 8, 12}.
const VIDEO_MODEL: &str = "sora-2";
const VIDEO_SIZE: &str = "1280x720";
const VIDEO_POLL_TIMEOUT: Duration = Duration::from_secs(600);
const VIDEO_POLL_INTERVAL: Duration = Duration::from_secs(5);

pub struct OpenAiProvider {
    client: reqwest::blocking::Client,
    api_key: String,
    image_model: String,
    /// Where generated video assets are written.
    asset_dir: PathBuf,
}

impl OpenAiProvider {
    /// Reads `OPENAI_API_KEY` (and optionally `UNKNOWN_IMAGE_MODEL`, since image
    /// model names get retired) from the environment.
    pub fn from_env() -> Result<Self, EngineError> {
        let api_key = std::env::var("OPENAI_API_KEY")
            .ok()
            .filter(|key| !key.trim().is_empty() && key != "your_api_key_here")
            .ok_or(EngineError::MissingApiKey)?;

        let image_model = std::env::var("UNKNOWN_IMAGE_MODEL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_IMAGE_MODEL.to_string());

        Ok(Self {
            client: reqwest::blocking::Client::new(),
            api_key,
            image_model,
            asset_dir: std::env::temp_dir().join("unknown_studio_ai"),
        })
    }

    pub fn with_asset_dir(mut self, dir: PathBuf) -> Self {
        self.asset_dir = dir;
        self
    }

    fn post_json(&self, url: &str, payload: &Value) -> Result<Value, EngineError> {
        let response = self
            .client
            .post(url)
            .bearer_auth(&self.api_key)
            .json(payload)
            .send()
            .map_err(|err| EngineError::Http(err.to_string()))?;
        json_body(response)
    }

    /// `choices[0].message`. Required for tool calling: when a tool is called
    /// `content` is empty and everything lives in `tool_calls`.
    fn chat_message(&self, payload: Value) -> Result<Value, EngineError> {
        let body = self.post_json(CHAT_COMPLETIONS_ENDPOINT, &payload)?;
        body.get("choices")
            .and_then(|choices| choices.get(0))
            .and_then(|choice| choice.get("message"))
            .cloned()
            .ok_or_else(|| EngineError::Protocol("no message in response".into()))
    }

    /// Message content with any markdown fence stripped, for JSON-returning calls.
    fn chat_content(&self, payload: Value) -> Result<String, EngineError> {
        self.chat_message(payload)?
            .get("content")
            .and_then(Value::as_str)
            .map(strip_json_fence)
            .ok_or_else(|| EngineError::Protocol("no message content".into()))
    }
}

impl ActionProvider for OpenAiProvider {
    fn chat(&self, context: ChatContext<'_>) -> Result<ChatOutcome, EngineError> {
        // Only the last N turns are carried; sending the whole history grows
        // token cost without bound over a session.
        const HISTORY_TURNS: usize = 12;

        let mut messages = vec![json!({
            "role": "system",
            "content": chat_system_prompt(context.has_clip),
        })];
        let start = context.history.len().saturating_sub(HISTORY_TURNS);
        messages.extend(context.history[start..].iter().map(|turn| {
            json!({
                "role": if turn.from_user { "user" } else { "assistant" },
                "content": turn.text,
            })
        }));
        messages.push(json!({ "role": "user", "content": context.prompt }));

        let message = self.chat_message(json!({
            "model": CHAT_MODEL,
            "messages": messages,
            "tools": chat_tools(context.has_clip),
            "tool_choice": "auto",
        }))?;

        // No tool call means plain conversation — no generation API is touched.
        let Some(call) = message
            .get("tool_calls")
            .and_then(Value::as_array)
            .and_then(|calls| calls.first())
            .and_then(|call| call.get("function"))
        else {
            let text = message
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string();
            return if text.is_empty() {
                Err(EngineError::Protocol("model returned an empty reply".into()))
            } else {
                Ok(ChatOutcome::Reply(text))
            };
        };

        // `arguments` is a JSON *string*, not an object.
        let args: Value = call
            .get("arguments")
            .and_then(Value::as_str)
            .and_then(|raw| serde_json::from_str(raw).ok())
            .unwrap_or(Value::Null);
        let tool_prompt = || {
            args.get("prompt")
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| EngineError::Protocol("tool call without a prompt".into()))
        };

        match call.get("name").and_then(Value::as_str).unwrap_or_default() {
            "generate_image" => Ok(ChatOutcome::GenerateImage {
                prompt: tool_prompt()?,
            }),
            "generate_video" => Ok(ChatOutcome::GenerateVideo {
                prompt: tool_prompt()?,
                seconds: match args.get("seconds").and_then(Value::as_u64).unwrap_or(4) {
                    s @ (4 | 8 | 12) => s as u32,
                    _ => 4,
                },
            }),
            "analyze_clip" => Ok(ChatOutcome::AnalyzeClip),
            other => Err(EngineError::Protocol(format!("unknown tool call: {other}"))),
        }
    }

    fn generate_image(&self, prompt: &str) -> Result<Vec<u8>, EngineError> {
        // `response_format` is not sent: `gpt-image-*` rejects it and always
        // returns base64.
        let body = self.post_json(
            IMAGE_GENERATIONS_ENDPOINT,
            &json!({
                "model": self.image_model,
                "prompt": prompt,
                "size": "1024x1024",
                "n": 1,
            }),
        )?;

        let item = body
            .get("data")
            .and_then(|data| data.get(0))
            .ok_or_else(|| EngineError::Protocol("no image data in response".into()))?;

        if let Some(encoded) = item.get("b64_json").and_then(Value::as_str) {
            return BASE64
                .decode(encoded)
                .map_err(|err| EngineError::Protocol(err.to_string()));
        }

        // Older models return a URL instead; both shapes are supported.
        let url = item
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::Protocol("neither 'b64_json' nor 'url' present".into()))?;
        self.client
            .get(url)
            .send()
            .and_then(|response| response.bytes())
            .map(|bytes| bytes.to_vec())
            .map_err(|err| EngineError::Http(err.to_string()))
    }

    fn generate_video(
        &self,
        prompt: &str,
        seconds: u32,
        progress: Progress<'_>,
    ) -> Result<PathBuf, EngineError> {
        progress("Creating video job…".into());

        let job = self.post_json(
            VIDEOS_ENDPOINT,
            &json!({
                "model": VIDEO_MODEL,
                "prompt": prompt,
                "size": VIDEO_SIZE,
                "seconds": seconds.to_string(),
            }),
        )?;
        let job_id = job
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::Protocol("video job has no id".into()))?
            .to_string();

        let started = Instant::now();
        loop {
            if started.elapsed() > VIDEO_POLL_TIMEOUT {
                return Err(EngineError::Http(format!("video job {job_id} timed out")));
            }
            std::thread::sleep(VIDEO_POLL_INTERVAL);

            let state = json_body(
                self.client
                    .get(format!("{VIDEOS_ENDPOINT}/{job_id}"))
                    .bearer_auth(&self.api_key)
                    .send()
                    .map_err(|err| EngineError::Http(err.to_string()))?,
            )?;

            match state.get("status").and_then(Value::as_str).unwrap_or("unknown") {
                "completed" | "succeeded" => break,
                status @ ("failed" | "cancelled" | "error") => {
                    return Err(EngineError::Http(format!("video generation {status}")));
                }
                status => {
                    let pct = state
                        .get("progress")
                        .and_then(Value::as_u64)
                        .map(|p| format!(" {p}%"))
                        .unwrap_or_default();
                    progress(format!("Generating video{pct} ({status})…"));
                }
            }
        }

        progress("Downloading video…".into());
        let bytes = self
            .client
            .get(format!("{VIDEOS_ENDPOINT}/{job_id}/content"))
            .bearer_auth(&self.api_key)
            .send()
            .and_then(|response| response.bytes())
            .map_err(|err| EngineError::Http(err.to_string()))?;

        std::fs::create_dir_all(&self.asset_dir)
            .map_err(|err| EngineError::Media(err.to_string()))?;
        let path = self.asset_dir.join(format!("{}.mp4", Uuid::new_v4()));
        std::fs::write(&path, &bytes).map_err(|err| EngineError::Media(err.to_string()))?;
        Ok(path)
    }

    fn transcribe(&self, wav: Vec<u8>) -> Result<String, EngineError> {
        let part = reqwest::blocking::multipart::Part::bytes(wav)
            .file_name("clip.wav")
            .mime_str("audio/wav")
            .map_err(|err| EngineError::Protocol(err.to_string()))?;
        let form = reqwest::blocking::multipart::Form::new()
            .text("model", TRANSCRIBE_MODEL)
            .part("file", part);

        let response = self
            .client
            .post(TRANSCRIPTIONS_ENDPOINT)
            .bearer_auth(&self.api_key)
            .multipart(form)
            .send()
            .map_err(|err| EngineError::Http(err.to_string()))?;

        json_body(response)?
            .get("text")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| EngineError::Protocol("no 'text' in transcription".into()))
    }

    fn analyze(&self, frames: &[Vec<u8>], transcript: &str) -> Result<String, EngineError> {
        let transcript_note = if transcript.trim().is_empty() {
            "(No speech detected / transcript unavailable for this clip.)".to_string()
        } else {
            format!("Spoken transcript for this clip:\n\"{transcript}\"")
        };

        let mut content = Vec::with_capacity(frames.len() + 1);
        content.push(json!({
            "type": "text",
            "text": format!(
                "Analyze these {} evenly-spaced frames from a developer video clip.\n\n{transcript_note}",
                frames.len()
            ),
        }));
        content.extend(frames.iter().map(|frame| {
            json!({
                "type": "image_url",
                "image_url": { "url": format!("data:image/jpeg;base64,{}", BASE64.encode(frame)) },
            })
        }));

        self.chat_content(json!({
            "model": VISION_MODEL,
            "messages": [
                { "role": "system", "content": RETENTION_CONSULTANT_SYSTEM_PROMPT },
                { "role": "user", "content": content },
            ],
            "max_tokens": 1000,
        }))
    }

    fn seo_strategy(&self, stats: PacingStats, transcript: &str) -> Result<String, EngineError> {
        let total = stats.total_duration_seconds;
        let average_clip = if stats.total_clips > 0 {
            total / stats.total_clips as f64
        } else {
            0.0
        };

        let user_content = format!(
            "Video pacing stats:\n\
             - Total duration: {total:.1} seconds ({:.1} minutes)\n\
             - Total clips / cuts on the timeline: {}\n\
             - Average clip length: {average_clip:.1} seconds\n\n\
             Full spoken transcript:\n{transcript}",
            total / 60.0,
            stats.total_clips,
        );

        self.chat_content(json!({
            "model": CHAT_MODEL,
            "messages": [
                { "role": "system", "content": SEO_STRATEGIST_SYSTEM_PROMPT },
                { "role": "user", "content": user_content },
            ],
            "max_tokens": 1500,
        }))
    }
}

/// Tool descriptions stay deliberately narrow: the model must pick them only on
/// an explicit request.
fn chat_tools(has_clip: bool) -> Value {
    let clip_note = if has_clip {
        "A clip is currently selected."
    } else {
        "No clip is selected right now."
    };

    json!([
        {
            "type": "function",
            "function": {
                "name": "generate_image",
                "description": format!(
                    "Generate a still image (schema, diagram, thumbnail, B-roll) and place it on \
                     the timeline. Only use when the user explicitly asks to create an image. {clip_note}"
                ),
                "parameters": {
                    "type": "object",
                    "properties": {
                        "prompt": {
                            "type": "string",
                            "description": "Detailed English description of the image to generate."
                        }
                    },
                    "required": ["prompt"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "generate_video",
                "description":
                    "Generate a short AI video clip. Use when the user explicitly asks for a \
                     generated VIDEO (not a still image). Call this directly — the application \
                     shows its own cost-confirmation card, so do not ask the user first.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "prompt": {
                            "type": "string",
                            "description": "Detailed English description of the video to generate."
                        },
                        "seconds": {
                            "type": "integer",
                            "description": "Clip length in seconds. Must be 4, 8 or 12.",
                            "enum": [4, 8, 12]
                        }
                    },
                    "required": ["prompt"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "analyze_clip",
                "description": format!(
                    "Analyze the currently selected clip for YouTube retention (pacing, framing, \
                     lighting, speech). Only use when the user asks for an analysis/review. {clip_note}"
                ),
                "parameters": { "type": "object", "properties": {} }
            }
        }
    ])
}

fn json_body(response: reqwest::blocking::Response) -> Result<Value, EngineError> {
    let status = response.status();
    if !status.is_success() {
        let body = response.text().unwrap_or_default();
        return Err(EngineError::Http(format!("{status}: {body}")));
    }
    response
        .json()
        .map_err(|err| EngineError::Protocol(err.to_string()))
}

/// Models wrap JSON in ```` ```json ```` fences despite instructions.
fn strip_json_fence(raw: &str) -> String {
    raw.trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim()
        .to_string()
}
