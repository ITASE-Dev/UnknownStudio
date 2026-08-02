//! Deconstruction pipeline: outlier in, fully analysed competitor video out,
//! written to both halves of the warehouse.
//!
//! The stages are real and ordered — heatmap, audio, scene detection, VLM
//! tagging, hook analysis — but their bodies are mocked. Each one stands where
//! a real call goes: the YouTube heatmap endpoint, an `ebur128` pass, FFmpeg
//! `scdet`, a vision model, the LLM. The delays are there so the UI is built
//! against a pipeline that actually takes time.

use crate::ai_tooling::competitor::models::*;
use crate::ai_tooling::competitor::store::{
    CompetitorDataStore, SemanticEntry, SemanticIndex, SemanticKind, StoreError,
};
use crate::ai_tooling::youtube_insights::ViralScore;
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum IngestError {
    #[error("warehouse: {0}")]
    Store(#[from] StoreError),

    #[error("{stage} failed for {video_id}: {reason}")]
    Stage {
        stage: &'static str,
        video_id: String,
        reason: String,
    },
}

pub type Result<T> = std::result::Result<T, IngestError>;

/// Ordered stages, for progress reporting in the UI.
pub const STAGES: [&str; 6] = [
    "retention heatmap",
    "audio dynamics",
    "scene detection",
    "vlm tagging",
    "hook analysis",
    "warehouse write",
];

/// Deconstructs one outlier and stores it.
///
/// Takes both stores by reference: the structured row and the embeddings are
/// written in the same pass, so a video can never land in one and miss the
/// other for the diff engine to trip over.
pub async fn deconstruct<S, V>(
    score: &ViralScore,
    channel_id: &str,
    duration_sec: f32,
    store: &S,
    index: &V,
) -> Result<CompetitorVideo>
where
    S: CompetitorDataStore,
    V: SemanticIndex,
{
    let seed = seed_of(&score.video_id);
    let duration = duration_sec.max(30.0);

    let retention = fetch_retention(&score.video_id, duration, seed).await;
    let audio = analyze_audio(&score.video_id, duration, seed).await;
    let structure = detect_scenes(&score.video_id, duration, seed).await;
    let transcript = tag_and_transcribe(&score.video_id, duration, seed, &score.title).await;

    let video = CompetitorVideo {
        video_id: score.video_id.clone(),
        channel_id: channel_id.to_string(),
        title: score.title.clone(),
        view_count: score.view_count,
        outlier_multiplier: score.multiplier,
        duration_sec: duration,
        published_at: None,
        retention,
        audio,
        structure,
        transcript,
        analyzed_at: now_unix(),
    };

    store.save_video(video.clone()).await?;
    index_meaning(&video, index).await?;

    Ok(video)
}

/// Sends the two things worth retrieving by meaning to the vector index.
async fn index_meaning<V: SemanticIndex>(video: &CompetitorVideo, index: &V) -> Result<()> {
    index
        .index(SemanticEntry {
            video_id: video.video_id.clone(),
            kind: SemanticKind::HookPromise,
            text: video.transcript.hook.promise.clone(),
            timestamp_sec: 0.0,
        })
        .await?;

    for shot in &video.structure.broll {
        index
            .index(SemanticEntry {
                video_id: video.video_id.clone(),
                kind: SemanticKind::BRollTopic,
                text: shot.semantic_topic.clone(),
                timestamp_sec: shot.start_sec,
            })
            .await?;
    }

    Ok(())
}

// ------------------------------------------------------------ mock stages

/// Stands in for the `mostReplayed` heatmap plus a retention curve.
async fn fetch_retention(_video_id: &str, duration: f32, seed: u64) -> RetentionAnalysis {
    stage_delay(180).await;

    // Peaks cluster in the first third, the way a well-cut video front-loads
    // its payoff; one late peak stands in for the climax.
    let peaks = vec![
        HeatmapPeak {
            start_sec: 2.0,
            end_sec: 8.0,
            intensity: 3.4,
            description: "Cold open — the payoff is shown before the intro.".into(),
        },
        HeatmapPeak {
            start_sec: duration * 0.22,
            end_sec: duration * 0.22 + 6.0,
            intensity: 2.1 + vary(seed, 0) * 0.6,
            description: "Demonstration beat, tight cuts over a close-up.".into(),
        },
        HeatmapPeak {
            start_sec: duration * 0.71,
            end_sec: duration * 0.71 + 5.0,
            intensity: 1.8 + vary(seed, 1) * 0.5,
            description: "Result reveal, punctuated with a riser.".into(),
        },
    ];

    let drops = vec![
        RetentionDrop {
            start_sec: duration * 0.09,
            end_sec: duration * 0.14,
            severity: 0.18 + vary(seed, 2) * 0.08,
            cause: DropCause::PacingStall,
        },
        RetentionDrop {
            start_sec: duration * 0.44,
            end_sec: duration * 0.49,
            severity: 0.11 + vary(seed, 3) * 0.06,
            cause: DropCause::VisualMonotony,
        },
        RetentionDrop {
            start_sec: duration * 0.88,
            end_sec: duration * 0.94,
            severity: 0.24 + vary(seed, 4) * 0.10,
            cause: DropCause::WeakOutro,
        },
    ];

    RetentionAnalysis {
        peaks,
        drops,
        average_view_ratio: 0.42 + vary(seed, 5) * 0.2,
    }
}

/// Stands in for a loudness pass over the decoded audio.
async fn analyze_audio(_video_id: &str, duration: f32, seed: u64) -> AudioDynamics {
    stage_delay(140).await;

    let mean_dbfs = -18.0 - vary(seed, 6) * 3.0;
    let mut volume_peaks = Vec::new();
    let mut transitions = Vec::new();

    // A transition sound roughly every twelve seconds, alternating between the
    // three effects a fast-cut video actually uses.
    let mut at = 4.0_f32;
    let kinds = ["whoosh", "riser", "sub_drop"];
    let mut n = 0usize;
    while at < duration {
        transitions.push(TransitionSound {
            timestamp_sec: at,
            sfx_type: kinds[n % kinds.len()].to_string(),
            gain_db: -9.0 + vary(seed, n as u64) * 3.0,
        });
        volume_peaks.push(VolumeEvent {
            timestamp_sec: at + 0.15,
            dbfs: mean_dbfs + 8.0,
            delta_db: 8.0,
        });
        at += 11.0 + vary(seed, n as u64 + 10) * 4.0;
        n += 1;
    }

    let volume_drops = vec![VolumeEvent {
        timestamp_sec: duration * 0.46,
        dbfs: mean_dbfs - 11.0,
        delta_db: -11.0,
    }];

    let silences = vec![
        SilenceGap {
            start_sec: duration * 0.45,
            end_sec: duration * 0.45 + 0.9,
        },
        SilenceGap {
            start_sec: duration * 0.87,
            end_sec: duration * 0.87 + 1.4,
        },
    ];

    AudioDynamics {
        volume_peaks,
        volume_drops,
        silences,
        transitions,
        mean_dbfs,
    }
}

/// Stands in for `ffmpeg -vf scdet` plus cutaway classification.
async fn detect_scenes(_video_id: &str, duration: f32, seed: u64) -> VisualAndPacingStructure {
    stage_delay(220).await;

    let shot = 2.6 + vary(seed, 7) * 1.8;
    let mut scene_cuts = Vec::new();
    let mut at = shot;
    while at < duration {
        scene_cuts.push(SceneCut {
            timestamp_sec: at,
            score: 0.6 + vary(seed, at as u64) * 0.35,
        });
        at += shot * (0.7 + vary(seed, at as u64 + 3) * 0.8);
    }

    let topics = [
        "overhead shot of the workspace, shallow depth of field",
        "screen recording of the editor with the cursor moving",
        "slow push-in on the product, rim lit against black",
        "archival clip of the original announcement",
        "animated diagram of the pipeline",
    ];
    let broll: Vec<BRollPlacement> = topics
        .iter()
        .enumerate()
        .map(|(i, topic)| {
            let start = duration * (0.12 + i as f32 * 0.17);
            BRollPlacement {
                start_sec: start,
                end_sec: start + 3.5 + vary(seed, i as u64) * 2.0,
                semantic_topic: (*topic).to_string(),
                // The push-in is the one with the presenter in frame.
                features_presenter: i == 2,
            }
        })
        .filter(|shot| shot.end_sec < duration)
        .collect();

    VisualAndPacingStructure {
        ending: VideoEndingAnalysis {
            start_sec: duration * 0.91,
            style: if seed % 2 == 0 {
                EndingStyle::NextVideoTease
            } else {
                EndingStyle::LoopBack
            },
            tail_sec: 4.0 + vary(seed, 8) * 3.0,
            loops_to_hook: seed % 2 == 1,
            call_to_action: Some("Points at the follow-up video before the energy drops.".into()),
        },
        average_shot_sec: shot,
        scene_cuts,
        broll,
    }
}

/// Stands in for Whisper plus a vision-language pass over sampled frames.
async fn tag_and_transcribe(
    _video_id: &str,
    duration: f32,
    seed: u64,
    title: &str,
) -> TranscriptAndHooks {
    stage_delay(260).await;

    let tags = [
        VlmTag::TalkingHead,
        VlmTag::BRoll,
        VlmTag::ScreenRecord,
        VlmTag::TalkingHead,
        VlmTag::TextOnScreen,
    ];

    let mut segments = Vec::new();
    let mut at = 0.0_f32;
    let mut n = 0usize;
    while at < duration {
        let len = 4.0 + vary(seed, n as u64) * 3.0;
        segments.push(TranscriptSegment {
            start_sec: at,
            end_sec: (at + len).min(duration),
            text: format!("segment {n} of the spoken track"),
            visual: tags[n % tags.len()],
        });
        at += len;
        n += 1;
    }

    TranscriptAndHooks {
        hook: HookAnalysis {
            time_to_value_sec: 1.2 + vary(seed, 9) * 2.0,
            hook_type: match seed % 4 {
                0 => HookType::ColdOpen,
                1 => HookType::Provocation,
                2 => HookType::Question,
                _ => HookType::Promise,
            },
            opening_line: format!("You are doing {title} wrong, and here is the proof."),
            words_per_minute: 165.0 + vary(seed, 10) * 40.0,
            cuts_in_hook: 4 + (seed % 3) as usize,
            promise: format!(
                "Promises a concrete, demonstrated fix for {title} within the first minute."
            ),
        },
        segments,
        language: Some("en".into()),
    }
}

async fn stage_delay(millis: u64) {
    tokio::time::sleep(Duration::from_millis(millis)).await;
}

/// Stable per-video pseudo-randomness, so the same id always deconstructs the
/// same way and the UI does not reshuffle between frames.
fn seed_of(video_id: &str) -> u64 {
    video_id
        .bytes()
        .fold(1469598103934665603_u64, |hash, byte| {
            (hash ^ byte as u64).wrapping_mul(1099511628211)
        })
}

/// A stable value in 0.0..1.0 from a seed and a salt.
fn vary(seed: u64, salt: u64) -> f32 {
    let mixed = seed.rotate_left(salt as u32 % 63 + 1) ^ salt.wrapping_mul(2654435761);
    (mixed % 1000) as f32 / 1000.0
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_tooling::competitor::store::InMemoryWarehouse;
    use crate::ai_tooling::youtube_insights::models::OutlierMethod;

    fn score(id: &str) -> ViralScore {
        ViralScore {
            video_id: id.into(),
            title: "a viral title".into(),
            view_count: 900_000,
            baseline_views: 100_000.0,
            multiplier: 9.0,
            modified_z: 7.2,
            percentile: 0.99,
            method: OutlierMethod::ModifiedZScore,
            is_outlier: true,
        }
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("runtime")
    }

    #[test]
    fn deconstruction_fills_every_section_and_lands_in_the_warehouse() {
        runtime().block_on(async {
            let warehouse = InMemoryWarehouse::new();
            let video = deconstruct(&score("vid1"), "UC1", 300.0, &warehouse, &warehouse)
                .await
                .expect("deconstruct");

            assert!(!video.retention.peaks.is_empty(), "peaks");
            assert!(!video.retention.drops.is_empty(), "drops");
            assert!(!video.audio.transitions.is_empty(), "sfx");
            assert!(!video.structure.scene_cuts.is_empty(), "cuts");
            assert!(!video.structure.broll.is_empty(), "b-roll");
            assert!(!video.transcript.segments.is_empty(), "transcript");
            assert!(!video.transcript.hook.promise.is_empty(), "hook promise");
            assert_ne!(video.structure.ending.style, EndingStyle::Unknown);

            // The relational row is queryable by channel.
            let rows = warehouse.videos_for_channel("UC1").await.expect("rows");
            assert_eq!(rows.len(), 1);
        });
    }

    #[test]
    fn the_hook_promise_and_every_broll_topic_reach_the_vector_index() {
        runtime().block_on(async {
            let warehouse = InMemoryWarehouse::new();
            let video = deconstruct(&score("vid2"), "UC1", 300.0, &warehouse, &warehouse)
                .await
                .expect("deconstruct");

            let hits = warehouse
                .search("screen recording editor cursor", SemanticKind::BRollTopic, 5)
                .await
                .expect("search");
            assert!(!hits.is_empty(), "the b-roll topics are searchable");

            let promise = warehouse
                .search("demonstrated fix", SemanticKind::HookPromise, 5)
                .await
                .expect("search");
            assert_eq!(promise.len(), 1);
            assert_eq!(promise[0].entry.video_id, video.video_id);
        });
    }

    #[test]
    fn the_same_video_always_deconstructs_the_same_way() {
        runtime().block_on(async {
            let warehouse = InMemoryWarehouse::new();
            let first = deconstruct(&score("stable"), "UC1", 240.0, &warehouse, &warehouse)
                .await
                .expect("first");
            let second = deconstruct(&score("stable"), "UC1", 240.0, &warehouse, &warehouse)
                .await
                .expect("second");

            assert_eq!(
                first.structure.average_shot_sec, second.structure.average_shot_sec,
                "the mock is seeded by id, not by chance"
            );
            assert_eq!(first.retention.drops.len(), second.retention.drops.len());
        });
    }

    #[test]
    fn different_videos_get_different_profiles() {
        runtime().block_on(async {
            let warehouse = InMemoryWarehouse::new();
            let a = deconstruct(&score("aaa"), "UC1", 240.0, &warehouse, &warehouse)
                .await
                .expect("a");
            let b = deconstruct(&score("bbb"), "UC1", 240.0, &warehouse, &warehouse)
                .await
                .expect("b");

            assert_ne!(a.structure.average_shot_sec, b.structure.average_shot_sec);
        });
    }

    #[test]
    fn a_very_short_video_does_not_produce_nonsense_spans() {
        runtime().block_on(async {
            let warehouse = InMemoryWarehouse::new();
            let video = deconstruct(&score("short"), "UC1", 1.0, &warehouse, &warehouse)
                .await
                .expect("deconstruct");

            assert!(video.duration_sec >= 30.0, "clamped to a usable floor");
            for shot in &video.structure.broll {
                assert!(shot.end_sec <= video.duration_sec, "b-roll stays inside");
            }
        });
    }
}
