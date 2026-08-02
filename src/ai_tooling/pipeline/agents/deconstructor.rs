//! Agent 1 — the Competitor Deconstructor.
//!
//! Reads measured facts and returns judgement. It is given the transcript with
//! timings, the retention peaks and drops, and the pacing statistics, and is
//! asked the one question the numbers cannot answer: *why did the audience
//! stay, and where did they leave.*

use crate::ai_tooling::competitor::models::CompetitorVideo;
use crate::ai_tooling::pipeline::models::CompetitorDNA;
use crate::ai_tooling::pipeline::schema::{strict_schema_for, SchemaSpec};
use serde_json::{json, Value};

/// Transcript characters sent. The whole thing is rarely needed and always
/// expensive; the shape of the edit is legible from a healthy sample.
const MAX_TRANSCRIPT_CHARS: usize = 6_000;

pub const SCHEMA_NAME: &str = "competitor_dna";

pub const SYSTEM_PROMPT: &str = "\
You are a retention analyst for short- and long-form video. You are given one video's \
measured data: a timestamped transcript with the shot type on screen, the most-replayed \
peaks, the points where the audience left, and cutting statistics.

Explain why it held its audience. Be concrete and mechanical: name the technique, the \
second it happens, and the effect. \"Cuts to a close-up 0.4s before the punchline\" is \
useful; \"engaging editing\" is not.

Rules:
- Ground every claim in the data supplied. Do not invent moments that are not in it.
- transferable_rules must survive a change of topic. Write them as instructions an \
editor could follow on unrelated footage.
- For each retention note, say what is on screen AND what is in the mix.
- If the data is thin, say so in the verdict rather than filling the gap.";

/// The schema the model is held to.
pub fn schema() -> SchemaSpec {
    strict_schema_for::<CompetitorDNA>(SCHEMA_NAME)
}

/// Builds the payload from a deconstructed video.
///
/// Only the fields the analysis needs are sent. The full record carries view
/// counts, ids and per-frame scene scores that would cost tokens and buy
/// nothing.
pub fn payload(video: &CompetitorVideo) -> Value {
    let transcript: Vec<Value> = video
        .transcript
        .segments
        .iter()
        .scan(0_usize, |budget, segment| {
            if *budget > MAX_TRANSCRIPT_CHARS {
                return None;
            }
            *budget += segment.text.len();
            Some(json!({
                "at": round(segment.start_sec),
                "text": segment.text,
                "on_screen": segment.visual.label(),
            }))
        })
        .collect();

    json!({
        "video_id": video.video_id,
        "title": video.title,
        "duration_sec": round(video.duration_sec),
        "outlier_multiplier": (video.outlier_multiplier * 10.0).round() / 10.0,
        "retention": {
            "average_view_ratio": round(video.retention.average_view_ratio),
            "peaks": video.retention.peaks.iter().map(|peak| json!({
                "start": round(peak.start_sec),
                "end": round(peak.end_sec),
                "replay_intensity": round(peak.intensity),
                "note": peak.description,
            })).collect::<Vec<_>>(),
            "drops": video.retention.drops.iter().map(|drop| json!({
                "start": round(drop.start_sec),
                "end": round(drop.end_sec),
                "audience_lost": round(drop.severity),
                "measured_cause": drop.cause.label(),
            })).collect::<Vec<_>>(),
        },
        "pacing": {
            "cuts_per_minute": round(video.structure.cuts_per_minute(video.duration_sec)),
            "average_shot_sec": round(video.structure.average_shot_sec),
            "broll_coverage": round(video.structure.broll_coverage(video.duration_sec)),
            "broll_topics": video.structure.broll.iter()
                .map(|shot| json!({ "at": round(shot.start_sec), "topic": shot.semantic_topic }))
                .collect::<Vec<_>>(),
            "ending": {
                "starts_at": round(video.structure.ending.start_sec),
                "style": video.structure.ending.style.label(),
                "tail_sec": round(video.structure.ending.tail_sec),
            },
        },
        "audio": {
            "mean_dbfs": round(video.audio.mean_dbfs),
            "sfx_per_minute": round(video.audio.sfx_per_minute(video.duration_sec)),
            "transitions": video.audio.transitions.iter().map(|sfx| json!({
                "at": round(sfx.timestamp_sec),
                "effect": sfx.sfx_type,
            })).collect::<Vec<_>>(),
            "silences": video.audio.silences.iter().map(|gap| json!({
                "start": round(gap.start_sec),
                "seconds": round(gap.duration_sec()),
            })).collect::<Vec<_>>(),
        },
        "hook": {
            "opening_line": video.transcript.hook.opening_line,
            "time_to_value_sec": round(video.transcript.hook.time_to_value_sec),
            "words_per_minute": round(video.transcript.hook.words_per_minute),
            "cuts_in_hook": video.transcript.hook.cuts_in_hook,
        },
        "transcript": transcript,
    })
}

/// One decimal place. Full f32 precision in a prompt is noise the model pays
/// for by the token.
fn round(value: f32) -> f32 {
    (value * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_tooling::competitor::models::*;

    fn video() -> CompetitorVideo {
        CompetitorVideo {
            video_id: "v1".into(),
            channel_id: "UC1".into(),
            title: "how to edit".into(),
            view_count: 1_000_000,
            outlier_multiplier: 8.25,
            duration_sec: 120.0,
            published_at: None,
            retention: RetentionAnalysis {
                peaks: vec![HeatmapPeak {
                    start_sec: 24.0,
                    end_sec: 30.0,
                    intensity: 3.0,
                    description: "cutaway".into(),
                }],
                drops: vec![RetentionDrop {
                    start_sec: 40.0,
                    end_sec: 46.0,
                    severity: 0.2,
                    cause: DropCause::PacingStall,
                }],
                average_view_ratio: 0.51,
            },
            audio: AudioDynamics {
                transitions: vec![TransitionSound {
                    timestamp_sec: 24.0,
                    sfx_type: "whoosh".into(),
                    gain_db: -9.0,
                }],
                silences: vec![SilenceGap { start_sec: 40.0, end_sec: 41.2 }],
                mean_dbfs: -17.5,
                ..Default::default()
            },
            structure: VisualAndPacingStructure {
                scene_cuts: (0..40)
                    .map(|i| SceneCut { timestamp_sec: i as f32 * 3.0, score: 0.8 })
                    .collect(),
                broll: vec![BRollPlacement {
                    start_sec: 23.0,
                    end_sec: 29.0,
                    semantic_topic: "overhead desk".into(),
                    features_presenter: false,
                }],
                ending: VideoEndingAnalysis {
                    start_sec: 110.0,
                    style: EndingStyle::NextVideoTease,
                    tail_sec: 4.0,
                    loops_to_hook: false,
                    call_to_action: None,
                },
                average_shot_sec: 3.0,
            },
            transcript: TranscriptAndHooks {
                segments: (0..5)
                    .map(|i| TranscriptSegment {
                        start_sec: i as f32 * 5.0,
                        end_sec: i as f32 * 5.0 + 5.0,
                        text: format!("line {i}"),
                        visual: VlmTag::TalkingHead,
                    })
                    .collect(),
                hook: HookAnalysis {
                    opening_line: "you are doing this wrong".into(),
                    time_to_value_sec: 1.5,
                    words_per_minute: 180.0,
                    cuts_in_hook: 5,
                    ..Default::default()
                },
                language: Some("en".into()),
            },
            analyzed_at: 0,
        }
    }

    #[test]
    fn the_payload_carries_all_three_inputs_the_role_needs() {
        let payload = payload(&video());

        assert!(payload["transcript"].as_array().is_some_and(|t| !t.is_empty()));
        assert!(!payload["retention"]["peaks"].as_array().expect("peaks").is_empty());
        assert!(!payload["retention"]["drops"].as_array().expect("drops").is_empty());
        assert!(payload["pacing"]["cuts_per_minute"].is_number());
    }

    #[test]
    fn each_transcript_line_keeps_its_time_and_its_shot_type() {
        let payload = payload(&video());
        let first = &payload["transcript"][0];

        assert_eq!(first["at"], 0.0);
        assert_eq!(first["on_screen"], "talking head");
        assert_eq!(first["text"], "line 0");
    }

    #[test]
    fn a_very_long_transcript_is_cut_off_rather_than_sent_whole() {
        let mut long = video();
        long.transcript.segments = (0..4000)
            .map(|i| TranscriptSegment {
                start_sec: i as f32,
                end_sec: i as f32 + 1.0,
                text: "a reasonably long spoken line of dialogue".into(),
                visual: VlmTag::TalkingHead,
            })
            .collect();

        let sent = payload(&long)["transcript"].as_array().expect("lines").len();
        assert!(sent < 4000, "the budget was not applied");
        assert!(sent > 0, "and it did not drop everything");
    }

    #[test]
    fn numbers_are_rounded_so_the_prompt_is_not_full_of_float_noise() {
        let payload = payload(&video());

        assert_eq!(payload["outlier_multiplier"], 8.3);
        assert_eq!(payload["audio"]["mean_dbfs"], -17.5);
        assert_eq!(payload["retention"]["average_view_ratio"], 0.5);
    }

    #[test]
    fn the_schema_is_named_and_strict() {
        let spec = schema();

        assert_eq!(spec.name, SCHEMA_NAME);
        assert_eq!(spec.schema["additionalProperties"], false);
        assert_eq!(spec.schema["type"], "object");
    }

    #[test]
    fn the_system_prompt_forbids_inventing_moments() {
        assert!(SYSTEM_PROMPT.contains("Do not invent"));
        assert!(SYSTEM_PROMPT.contains("transferable_rules"));
    }
}
