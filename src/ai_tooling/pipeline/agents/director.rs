//! Agent 2 — the Director Engine.
//!
//! Takes Agent 1's DNA and the user's *populated* timeline and finds the gaps.
//! The hard part is not spotting what the reference does; it is checking
//! whether this edit already does it. A plan that proposes work the user has
//! already done stops being read.

use crate::ai_tooling::pipeline::models::{CompetitorDNA, RevisionDraftList};
use crate::ai_tooling::pipeline::schema::{strict_schema_for, SchemaSpec};
use crate::ai_tooling::revision::timeline::{ClipRole, CurrentTimelineState};
use serde_json::{json, Value};

pub const SCHEMA_NAME: &str = "revision_plan";

pub const SYSTEM_PROMPT: &str = "\
You are the director of a video edit. You are given the DNA of a video that outperformed \
its channel, and the current state of the user's timeline: every track, every clip, its \
role and its span.

Find the gaps and return the changes that close them.

Rules:
- Map by proportion, not by absolute time. A beat 20% into the reference belongs at 20% \
of this timeline, whatever the two durations are.
- Never propose something the timeline already has. If a cutaway covers that second, or \
an audio element already sits on that cut, skip it.
- generate_and_insert_b_roll requires a track_index that is unlocked and free for the \
whole span you name. The free_video_tracks field lists what is available.
- cut_silence requires the clip_id of the clip the silence falls inside. Use the ids \
given in the timeline, exactly as written.
- fix_retention_drop is advisory: it marks a spot for the editor. Use it when the fix is \
a judgement call rather than a placement.
- impact is your estimate of retention gained, 0.0 to 1.0. Be honest; a list where \
everything is 0.9 is a list with no ordering.
- Order tasks most valuable first. Return at most 12.
- Every task must cite the moment in the reference it came from.";

pub fn schema() -> SchemaSpec {
    strict_schema_for::<RevisionDraftList>(SCHEMA_NAME)
}

/// Builds the payload: the DNA plus a compact, honest view of the timeline.
///
/// Clip ids are sent as the model must write them back, and the free-track
/// analysis is precomputed — asking a language model to work out lane
/// availability from a list of spans is asking for a plausible wrong answer.
pub fn payload(dna: &CompetitorDNA, current: &CurrentTimelineState) -> Value {
    let tracks: Vec<Value> = current
        .tracks
        .iter()
        .map(|track| {
            json!({
                "index": track.index,
                "name": track.name,
                "role": match track.role {
                    crate::ai_tooling::revision::timeline::TrackRole::Video => "video",
                    crate::ai_tooling::revision::timeline::TrackRole::Audio => "audio",
                },
                "locked": track.locked,
                "clips": track.clips.iter().map(|clip| json!({
                    "clip_id": format!("c{}", clip.id),
                    "label": clip.label,
                    "start": round(clip.start_sec),
                    "end": round(clip.end_sec),
                    "role": match clip.role {
                        ClipRole::ARoll => "a_roll",
                        ClipRole::BRoll => "b_roll",
                        ClipRole::Audio => "audio",
                    },
                })).collect::<Vec<_>>(),
            })
        })
        .collect();

    // Where a cutaway could actually go, sampled across the edit. Computed
    // here because it is a geometry question, not a language one.
    let free_video_tracks: Vec<Value> = sample_times(current.duration_sec)
        .into_iter()
        .filter_map(|at| {
            current
                .free_video_track_at(at, 3.5)
                .map(|index| json!({ "at": round(at), "track_index": index }))
        })
        .collect();

    json!({
        "reference_dna": dna,
        "current_timeline": {
            "duration_sec": round(current.duration_sec),
            "cuts_per_minute": round(current.cuts_per_minute()),
            "broll_coverage": round(current.broll_coverage()),
            "cut_times": current.cut_times().iter().map(|t| round(*t)).collect::<Vec<_>>(),
            "tracks": tracks,
        },
        "free_video_tracks": free_video_tracks,
        "first_free_audio_track": current.first_free_audio_track(),
    })
}

/// Probe points across the timeline, one every two seconds, capped so a long
/// edit cannot flood the payload.
fn sample_times(duration: f32) -> Vec<f32> {
    const STEP: f32 = 2.0;
    const MAX_SAMPLES: usize = 60;

    if duration <= 0.0 {
        return Vec::new();
    }
    let count = ((duration / STEP).ceil() as usize).min(MAX_SAMPLES);
    let step = duration / count.max(1) as f32;
    (0..count).map(|i| i as f32 * step).collect()
}

fn round(value: f32) -> f32 {
    (value * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_tooling::pipeline::models::{AudioDna, HookDna, PacingDna, VisualDna};
    use crate::ai_tooling::revision::timeline::{ClipView, TrackRole, TrackView};

    fn dna() -> CompetitorDNA {
        CompetitorDNA {
            video_id: "ref1".into(),
            verdict: "fast cuts and constant coverage".into(),
            hook: HookDna {
                hook_type: "cold_open".into(),
                promise: "shows the result first".into(),
                time_to_value_sec: 1.2,
                assessment: "strong".into(),
            },
            pacing: PacingDna {
                cuts_per_minute: 20.0,
                max_silence_sec: 0.3,
                words_per_minute: 170.0,
                front_loaded: true,
            },
            audio: AudioDna {
                sfx_per_minute: 5.0,
                signature_effects: vec!["whoosh".into()],
                dynamics_note: "punctuates every cut".into(),
            },
            visual: VisualDna {
                broll_coverage: 0.4,
                longest_static_hold_sec: 4.0,
                recurring_topics: vec!["overhead desk".into()],
                ending_style: "next_video_tease".into(),
            },
            retention_notes: Vec::new(),
            transferable_rules: vec!["cut every 3 seconds".into()],
        }
    }

    fn timeline() -> CurrentTimelineState {
        CurrentTimelineState {
            tracks: vec![
                TrackView {
                    index: 0,
                    name: "V1".into(),
                    role: TrackRole::Video,
                    locked: false,
                    clips: vec![ClipView {
                        id: 12,
                        label: "interview.mp4".into(),
                        start_sec: 0.0,
                        end_sec: 60.0,
                        role: ClipRole::ARoll,
                    }],
                },
                TrackView {
                    index: 1,
                    name: "V2".into(),
                    role: TrackRole::Video,
                    locked: false,
                    clips: Vec::new(),
                },
                TrackView {
                    index: 2,
                    name: "A1".into(),
                    role: TrackRole::Audio,
                    locked: false,
                    clips: Vec::new(),
                },
            ],
            caption_spans: Vec::new(),
            duration_sec: 60.0,
        }
    }

    #[test]
    fn the_payload_carries_both_inputs_the_role_requires() {
        let payload = payload(&dna(), &timeline());

        assert_eq!(payload["reference_dna"]["video_id"], "ref1");
        assert_eq!(payload["current_timeline"]["duration_sec"], 60.0);
    }

    #[test]
    fn clip_ids_are_sent_in_the_form_the_model_must_write_back() {
        let payload = payload(&dna(), &timeline());
        let clip = &payload["current_timeline"]["tracks"][0]["clips"][0];

        // The dispatcher parses `c12`; anything else is an unknown clip.
        assert_eq!(clip["clip_id"], "c12");
        assert_eq!(clip["role"], "a_roll");
    }

    #[test]
    fn lane_availability_is_computed_for_the_model_not_left_to_it() {
        let payload = payload(&dna(), &timeline());
        let free = payload["free_video_tracks"].as_array().expect("free tracks");

        assert!(!free.is_empty(), "V2 is empty and should be offered");
        assert!(free.iter().all(|slot| slot["track_index"] == 1));
    }

    #[test]
    fn a_fully_occupied_timeline_offers_no_lanes_rather_than_a_wrong_one() {
        let mut busy = timeline();
        busy.tracks[1].clips.push(ClipView {
            id: 99,
            label: "cover.mp4".into(),
            start_sec: 0.0,
            end_sec: 60.0,
            role: ClipRole::BRoll,
        });

        let payload = payload(&dna(), &busy);
        assert!(payload["free_video_tracks"].as_array().expect("array").is_empty());
    }

    #[test]
    fn the_sample_grid_is_capped_so_a_long_edit_cannot_flood_the_prompt() {
        assert!(sample_times(10_000.0).len() <= 60);
        assert!(sample_times(0.0).is_empty());
        assert_eq!(sample_times(10.0).len(), 5, "one every two seconds");
    }

    #[test]
    fn the_schema_offers_every_action_the_executor_can_run() {
        let json = schema().schema.to_string();

        for tag in [
            "generate_and_insert_b_roll",
            "add_transition_audio",
            "fix_retention_drop",
            "revise_ending",
            "cut_silence",
        ] {
            assert!(json.contains(tag), "{tag} unavailable to the director");
        }
    }

    #[test]
    fn the_prompt_states_the_rules_the_executor_will_enforce_anyway() {
        // Cheaper to be told than to fail validation and retry.
        assert!(SYSTEM_PROMPT.contains("clip_id"));
        assert!(SYSTEM_PROMPT.contains("track_index"));
        assert!(SYSTEM_PROMPT.contains("Never propose something the timeline already has"));
    }
}
