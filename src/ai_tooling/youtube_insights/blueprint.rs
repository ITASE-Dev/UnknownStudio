//! Translates a viral video's pacing into edits on the user's timeline.
//!
//! Every action is anchored to a measured moment in the user's own transcript.
//! Nothing is invented: if the local audio shows no long silence, no cut is
//! proposed, however fast the reference video runs.

use crate::ai_tooling::audio_analysis::models::TranscriptOutput;
use crate::ai_tooling::orchestration::dispatcher::ActionCommand;
use crate::ai_tooling::youtube_insights::heatmap::{self, CUT_GAP_SEC};
use crate::ai_tooling::youtube_insights::models::{PacingHeatmap, ViralBlueprint};

/// Ceiling on proposed edits. A plan the user cannot review is a plan they
/// cannot accept, and 40 cuts is already a long review.
pub const MAX_ACTIONS: usize = 40;

/// Below this difference in words per minute the edits are not worth making.
const WPM_TOLERANCE: f32 = 15.0;

#[derive(Debug, Clone, Copy)]
pub struct BlueprintSettings {
    pub max_actions: usize,
    /// Silences longer than the reference's mean gap by this factor are cut.
    pub gap_tolerance: f32,
    /// Whether to mark cutaway opportunities from the reference's density.
    pub suggest_broll: bool,
}

impl Default for BlueprintSettings {
    fn default() -> Self {
        Self {
            max_actions: MAX_ACTIONS,
            gap_tolerance: 1.5,
            suggest_broll: true,
        }
    }
}

/// Builds a plan that moves the user's clip towards the reference's rhythm.
pub fn generate(
    clip_id: &str,
    transcript: &TranscriptOutput,
    target: &PacingHeatmap,
    settings: BlueprintSettings,
) -> ViralBlueprint {
    let current = heatmap::from_transcript(clip_id, transcript, None);
    let mut actions = Vec::new();
    let mut notes = Vec::new();

    let cut_threshold = cut_threshold(target, settings.gap_tolerance);
    let mut removed_silence = 0.0;

    // Long silences, longest first: those are the cuts that move the pacing
    // most, and the budget should be spent on them before the marginal ones.
    let mut candidates: Vec<(usize, f32, f32)> = transcript
        .transcript
        .words
        .iter()
        .enumerate()
        .skip(1)
        .filter(|(_, word)| word.gap_before >= cut_threshold)
        .map(|(index, word)| (index, word.start, word.gap_before))
        .collect();
    candidates.sort_by(|a, b| b.2.total_cmp(&a.2));
    candidates.truncate(settings.max_actions.saturating_sub(2));

    if candidates.is_empty() {
        notes.push(format!(
            "No silence over {cut_threshold:.2}s — the pacing is already tighter than the reference."
        ));
    } else {
        notes.push(format!(
            "{} silences over {:.2}s, {:.1}s of dead air in total.",
            candidates.len(),
            cut_threshold,
            candidates.iter().map(|(_, _, gap)| gap).sum::<f32>()
        ));
    }

    // Back to timeline order so the plan reads top to bottom.
    candidates.sort_by(|a, b| a.1.total_cmp(&b.1));
    for (_, start, gap) in &candidates {
        removed_silence += gap;
        // Split at the point speech resumes; the head clip then holds the
        // silence and can be trimmed or deleted by the user.
        actions.push(ActionCommand::SplitClip {
            clip_id: clip_id.to_string(),
            time_sec: *start,
        });
    }

    if current.hook_retention_wpm + WPM_TOLERANCE < target.hook_retention_wpm {
        actions.push(ActionCommand::AddMarker {
            time_sec: 0.0,
            color: "red".into(),
            label: format!(
                "Hook: {:.0} wpm vs {:.0} on the reference",
                current.hook_retention_wpm, target.hook_retention_wpm
            ),
        });
        notes.push(format!(
            "The opening runs {:.0} wpm against the reference's {:.0} — tighten or re-record it.",
            current.hook_retention_wpm, target.hook_retention_wpm
        ));
    }

    if settings.suggest_broll {
        if let Some(density) = target.broll_density.filter(|density| *density > 0.0) {
            let interval = 60.0 / density;
            let marks = broll_marks(current.duration_sec, interval, &mut actions, settings.max_actions);
            if marks > 0 {
                notes.push(format!(
                    "Reference cuts away {density:.1}x a minute; {marks} cutaway points marked."
                ));
            }
        }
    }

    actions.truncate(settings.max_actions);

    ViralBlueprint {
        reference_video_id: target.video_id.clone(),
        projected_wpm: projected_wpm(&current, removed_silence),
        target: target.clone(),
        current,
        actions,
        notes,
    }
}

/// The silence length worth cutting: the reference's own mean gap, widened by
/// the tolerance, and never below what an editor would call a pause.
fn cut_threshold(target: &PacingHeatmap, tolerance: f32) -> f32 {
    (target.mean_gap_sec * tolerance).max(CUT_GAP_SEC)
}

/// Marks cutaway points on the beat implied by the reference's density.
fn broll_marks(
    duration_sec: f32,
    interval_sec: f32,
    actions: &mut Vec<ActionCommand>,
    max_actions: usize,
) -> usize {
    if interval_sec <= 0.0 || duration_sec <= interval_sec {
        return 0;
    }

    let mut marks = 0;
    let mut at = interval_sec;
    while at < duration_sec && actions.len() < max_actions {
        actions.push(ActionCommand::AddMarker {
            time_sec: at,
            color: "purple".into(),
            label: "B-roll".into(),
        });
        marks += 1;
        at += interval_sec;
    }
    marks
}

/// Speaking rate once the proposed silence is gone.
fn projected_wpm(current: &PacingHeatmap, removed_silence_sec: f32) -> f32 {
    let remaining = (current.duration_sec - removed_silence_sec).max(1.0);
    let words = current.overall_wpm * (current.duration_sec / 60.0);
    words / (remaining / 60.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_tooling::audio_analysis::models::{
        AnalysisConfig, Media, Meta, Transcript, TranscriptStats, Word,
    };

    fn transcript(words: Vec<Word>, duration: f32) -> TranscriptOutput {
        let stats = TranscriptStats::from_words(&words, &AnalysisConfig::default().pacing);
        TranscriptOutput::new(
            Media {
                path: "/clips/a.mp4".into(),
                filename: "a.mp4".into(),
                duration_sec: duration,
                sample_rate: 16_000,
                channels: 1,
            },
            Meta {
                generated_at: "2026-01-01T00:00:00Z".into(),
                generator: "test".into(),
                generator_version: "0".into(),
                language: Some("en".into()),
                analysis_duration_sec: 0.0,
            },
            AnalysisConfig::default(),
            Transcript {
                text: String::new(),
                segments: Vec::new(),
                words,
                stats,
            },
        )
    }

    fn word(start: f32, gap: f32) -> Word {
        Word {
            text: "word".into(),
            start,
            end: start + 0.4,
            probability: 0.9,
            gap_before: gap,
            mean_dbfs: -20.0,
        }
    }

    /// Reference: fast, tight, cuts away 4x a minute.
    fn target() -> PacingHeatmap {
        PacingHeatmap {
            video_id: "viral1".into(),
            hook_retention_wpm: 200.0,
            overall_wpm: 180.0,
            jump_cut_frequency: 20.0,
            mean_gap_sec: 0.15,
            max_gap_sec: 0.5,
            broll_density: Some(4.0),
            windows: Vec::new(),
            duration_sec: 60.0,
        }
    }

    /// Slow local clip: three long silences.
    fn slow_clip() -> TranscriptOutput {
        let mut words = vec![word(0.0, 0.0)];
        let mut at = 0.5;
        for index in 1..20 {
            let gap = if index % 6 == 0 { 2.0 } else { 0.1 };
            at += gap;
            words.push(word(at, gap));
            at += 0.4;
        }
        transcript(words, 30.0)
    }

    #[test]
    fn long_silences_become_cuts_in_timeline_order() {
        let blueprint = generate("c1", &slow_clip(), &target(), BlueprintSettings::default());

        let cuts: Vec<f32> = blueprint
            .actions
            .iter()
            .filter_map(|action| match action {
                ActionCommand::SplitClip { time_sec, .. } => Some(*time_sec),
                _ => None,
            })
            .collect();

        assert_eq!(cuts.len(), 3, "one per two-second silence");
        assert!(cuts.windows(2).all(|pair| pair[0] < pair[1]), "in order");
        assert!(blueprint.notes[0].contains("dead air"));
    }

    #[test]
    fn every_cut_names_the_clip_it_applies_to() {
        let blueprint = generate("c42", &slow_clip(), &target(), BlueprintSettings::default());

        assert!(blueprint.actions.iter().all(|action| match action {
            ActionCommand::SplitClip { clip_id, .. } => clip_id == "c42",
            _ => true,
        }));
    }

    #[test]
    fn a_clip_already_tighter_than_the_reference_gets_no_cuts() {
        // Every gap 0.1s: nothing to remove.
        let words: Vec<Word> = (0..20)
            .map(|index| word(index as f32 * 0.5, if index == 0 { 0.0 } else { 0.1 }))
            .collect();

        let blueprint = generate("c1", &transcript(words, 10.0), &target(), BlueprintSettings::default());

        assert!(!blueprint
            .actions
            .iter()
            .any(|action| matches!(action, ActionCommand::SplitClip { .. })));
        assert!(blueprint.notes[0].contains("already tighter"));
    }

    #[test]
    fn a_slow_hook_is_marked_with_both_rates() {
        let blueprint = generate("c1", &slow_clip(), &target(), BlueprintSettings::default());

        let hook = blueprint
            .actions
            .iter()
            .find(|action| matches!(action, ActionCommand::AddMarker { time_sec, .. } if *time_sec == 0.0))
            .expect("hook marker");

        let ActionCommand::AddMarker { label, color, .. } = hook else {
            panic!("expected a marker");
        };
        assert!(label.contains("200"), "states the target: {label}");
        assert_eq!(color, "red");
    }

    #[test]
    fn broll_marks_follow_the_reference_density_and_can_be_disabled() {
        let with_broll = generate("c1", &slow_clip(), &target(), BlueprintSettings::default());
        let marks = with_broll
            .actions
            .iter()
            .filter(|action| matches!(action, ActionCommand::AddMarker { label, .. } if label == "B-roll"))
            .count();
        // 4 per minute over 30s ≈ one every 15s.
        assert_eq!(marks, 1);

        let without = generate(
            "c1",
            &slow_clip(),
            &target(),
            BlueprintSettings { suggest_broll: false, ..BlueprintSettings::default() },
        );
        assert!(!without
            .actions
            .iter()
            .any(|action| matches!(action, ActionCommand::AddMarker { label, .. } if label == "B-roll")));
    }

    #[test]
    fn an_unknown_broll_density_proposes_no_cutaways() {
        let mut reference = target();
        reference.broll_density = None;

        let blueprint = generate("c1", &slow_clip(), &reference, BlueprintSettings::default());
        assert!(!blueprint
            .actions
            .iter()
            .any(|action| matches!(action, ActionCommand::AddMarker { label, .. } if label == "B-roll")));
    }

    #[test]
    fn the_plan_stays_within_its_budget() {
        // A transcript full of long silences would otherwise produce hundreds.
        let words: Vec<Word> = (0..300)
            .map(|index| word(index as f32 * 3.0, if index == 0 { 0.0 } else { 2.5 }))
            .collect();

        let blueprint = generate(
            "c1",
            &transcript(words, 900.0),
            &target(),
            BlueprintSettings { max_actions: 10, ..BlueprintSettings::default() },
        );
        assert!(blueprint.actions.len() <= 10, "{}", blueprint.actions.len());
    }

    #[test]
    fn the_projection_reports_the_pacing_the_cuts_would_buy() {
        let blueprint = generate("c1", &slow_clip(), &target(), BlueprintSettings::default());

        assert!(blueprint.projected_wpm > blueprint.current.overall_wpm);
        assert!(blueprint.wpm_gap() > 0.0, "the reference is faster");
        assert!(!blueprint.is_empty());
        assert_eq!(blueprint.reference_video_id, "viral1");
    }
}
