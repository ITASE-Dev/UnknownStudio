//! The director prompt: minified telemetry in, strict editing rulebook out.

use crate::ai_tooling::scraping::PeakAnalysis;
use serde_json::{json, Value};

/// Deliberately terse: every sentence buys tokens on each call.
pub const SYSTEM_PROMPT: &str = "Role: elite short-form video Director. \
Input: minified JSON telemetry from the highest-retention moment of a viral video. \
peak_sec=timestamp of max replay. wpm=words per minute in peak +/-10s. \
hook=verbatim transcript of that window. \
Task: reverse-engineer the editing rulebook that produced that retention spike. \
Rules: infer, do not describe. Numbers must be actionable edit-suite settings. \
target_wpm=pacing target int. max_silence_ms=max dead air before cut int. \
hook_style=short label of the rhetorical device used. \
b_roll_frequency_sec=seconds between visual cutaways float. \
heatmap_peak_actions=3-6 imperative editor directives, each <=12 words. \
Output: JSON only. No prose. No markdown. No preamble.";

/// Transcript sent to the model. Long hooks are truncated — the retention
/// signal is in the opening, and the tail only costs tokens.
const MAX_HOOK_CHARS: usize = 1200;

/// Minified telemetry payload; field names match what the prompt describes.
pub fn build_payload(peak: &PeakAnalysis) -> Value {
    json!({
        "peak_sec": (peak.peak_seconds * 10.0).round() / 10.0,
        "wpm": peak.wpm.round() as i64,
        "hook": truncate(&peak.hook_text, MAX_HOOK_CHARS),
    })
}

/// JSON schema both providers enforce, so the answer needs no repair.
pub fn output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "target_wpm": { "type": "integer" },
            "max_silence_ms": { "type": "integer" },
            "hook_style": { "type": "string" },
            "b_roll_frequency_sec": { "type": "number" },
            "heatmap_peak_actions": { "type": "array", "items": { "type": "string" } }
        },
        "required": [
            "target_wpm",
            "max_silence_ms",
            "hook_style",
            "b_roll_frequency_sec",
            "heatmap_peak_actions"
        ],
        "additionalProperties": false
    })
}

/// Truncates on a character boundary so multi-byte text cannot panic.
fn truncate(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peak(hook: &str) -> PeakAnalysis {
        PeakAnalysis {
            video_id: "abc".into(),
            peak_seconds: 42.44,
            peak_value: 0.9,
            hook_text: hook.into(),
            wpm: 183.6,
            segment_start: 32.0,
            segment_end: 52.0,
            word_count: 61,
        }
    }

    #[test]
    fn payload_rounds_the_telemetry() {
        let payload = build_payload(&peak("hook text"));
        assert_eq!(payload["peak_sec"], 42.4);
        assert_eq!(payload["wpm"], 184);
        assert_eq!(payload["hook"], "hook text");
    }

    #[test]
    fn long_hooks_are_cut_on_char_boundaries() {
        let payload = build_payload(&peak(&"é".repeat(2_000)));
        let hook = payload["hook"].as_str().expect("hook");
        assert_eq!(hook.chars().count(), MAX_HOOK_CHARS);
    }

    #[test]
    fn schema_forbids_extra_fields() {
        let schema = output_schema();
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["required"].as_array().expect("required").len(), 5);
    }
}
