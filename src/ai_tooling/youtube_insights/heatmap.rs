//! Pacing analysis: turns a transcript (and, when available, motion data) into
//! the rhythm numbers an edit can be matched against.

use crate::ai_tooling::audio_analysis::models::{TranscriptOutput, Word};
use crate::ai_tooling::visual_analysis::models::VisualTimeline;
use crate::ai_tooling::youtube_insights::models::{PacingHeatmap, PacingWindow};

/// The hook is judged over the opening seconds — the window a viewer decides in.
pub const HOOK_WINDOW_SEC: f32 = 10.0;

/// Heatmap resolution. Short enough to show a rhythm change, long enough that
/// one fast sentence does not read as a trend.
pub const WINDOW_SEC: f32 = 5.0;

/// A silence at least this long is where an editor would cut.
pub const CUT_GAP_SEC: f32 = 0.35;

/// Words with their timings, the minimum a heatmap needs.
#[derive(Debug, Clone, Copy)]
pub struct TimedWord {
    pub start: f32,
    pub end: f32,
    pub gap_before: f32,
}

impl From<&Word> for TimedWord {
    fn from(word: &Word) -> Self {
        Self {
            start: word.start,
            end: word.end,
            gap_before: word.gap_before,
        }
    }
}

/// Measures a local transcript produced by `audio_analysis`.
pub fn from_transcript(
    video_id: &str,
    transcript: &TranscriptOutput,
    visual: Option<&VisualTimeline>,
) -> PacingHeatmap {
    let words: Vec<TimedWord> = transcript.transcript.words.iter().map(TimedWord::from).collect();
    build(video_id, &words, transcript.media.duration_sec, visual)
}

/// Measures any timed word list — the shape a scraped transcript arrives in.
pub fn build(
    video_id: &str,
    words: &[TimedWord],
    duration_sec: f32,
    visual: Option<&VisualTimeline>,
) -> PacingHeatmap {
    let duration = duration_sec.max(words.last().map_or(0.0, |word| word.end));

    PacingHeatmap {
        video_id: video_id.to_string(),
        hook_retention_wpm: hook_retention_wpm(words),
        overall_wpm: overall_wpm(words, duration),
        jump_cut_frequency: jump_cut_frequency(words, duration),
        mean_gap_sec: mean_gap(words),
        max_gap_sec: max_gap(words),
        broll_density: visual.map(|timeline| broll_density(timeline, duration)),
        windows: windows(words, duration),
        duration_sec: duration,
    }
}

/// Words per minute across the opening window.
///
/// Measured against the window, not against the words' own span: a hook that
/// starts two seconds late is slower, and the number should say so.
pub fn hook_retention_wpm(words: &[TimedWord]) -> f32 {
    let spoken = words.iter().filter(|word| word.start < HOOK_WINDOW_SEC).count();
    if spoken == 0 {
        return 0.0;
    }
    spoken as f32 / (HOOK_WINDOW_SEC / 60.0)
}

pub fn overall_wpm(words: &[TimedWord], duration_sec: f32) -> f32 {
    if words.is_empty() || duration_sec <= 0.0 {
        return 0.0;
    }
    words.len() as f32 / (duration_sec / 60.0)
}

/// Cuts per minute, counting every silence long enough to hide one.
pub fn jump_cut_frequency(words: &[TimedWord], duration_sec: f32) -> f32 {
    if duration_sec <= 0.0 {
        return 0.0;
    }
    let cuts = words
        .iter()
        .filter(|word| word.gap_before >= CUT_GAP_SEC)
        .count();
    cuts as f32 / (duration_sec / 60.0)
}

pub fn mean_gap(words: &[TimedWord]) -> f32 {
    // The first word's gap is not a pause in the edit, it is the lead-in.
    let gaps: Vec<f32> = words.iter().skip(1).map(|word| word.gap_before).collect();
    if gaps.is_empty() {
        return 0.0;
    }
    gaps.iter().sum::<f32>() / gaps.len() as f32
}

pub fn max_gap(words: &[TimedWord]) -> f32 {
    words
        .iter()
        .skip(1)
        .map(|word| word.gap_before)
        .fold(0.0, f32::max)
}

/// Cutaways per minute, estimated from motion spikes.
///
/// This is a proxy: a spike marks a visual change, which is usually a cut or a
/// cutaway. It measures cutting rhythm, not a true B-roll/A-roll label, and the
/// blueprint treats it as such.
pub fn broll_density(visual: &VisualTimeline, duration_sec: f32) -> f32 {
    if duration_sec <= 0.0 {
        return 0.0;
    }
    visual.spikes.len() as f32 / (duration_sec / 60.0)
}

/// Per-window speaking rate and silence share.
pub fn windows(words: &[TimedWord], duration_sec: f32) -> Vec<PacingWindow> {
    if duration_sec <= 0.0 {
        return Vec::new();
    }

    let count = (duration_sec / WINDOW_SEC).ceil() as usize;
    (0..count)
        .map(|index| {
            let start = index as f32 * WINDOW_SEC;
            let end = (start + WINDOW_SEC).min(duration_sec);
            let span = (end - start).max(f32::EPSILON);

            let inside: Vec<&TimedWord> = words
                .iter()
                .filter(|word| word.start >= start && word.start < end)
                .collect();

            let speaking: f32 = inside.iter().map(|word| (word.end - word.start).max(0.0)).sum();

            PacingWindow {
                start_sec: start,
                end_sec: end,
                words_per_minute: inside.len() as f32 / (span / 60.0),
                silence_ratio: (1.0 - speaking / span).clamp(0.0, 1.0),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_tooling::visual_analysis::models::MotionSpike;

    /// `count` words per second, each 0.4s long with a 0.1s gap.
    fn steady(count: usize) -> Vec<TimedWord> {
        (0..count)
            .map(|index| {
                let start = index as f32 * 0.5;
                TimedWord {
                    start,
                    end: start + 0.4,
                    gap_before: if index == 0 { 0.0 } else { 0.1 },
                }
            })
            .collect()
    }

    #[test]
    fn a_front_loaded_hook_measures_faster_than_the_body() {
        // 30 words in the first 10s, then 10 words over the next 50s.
        let mut words = steady(30);
        for index in 0..10 {
            let start = 15.0 + index as f32 * 5.0;
            words.push(TimedWord { start, end: start + 0.4, gap_before: 4.6 });
        }

        let heatmap = build("v1", &words, 60.0, None);
        assert_eq!(heatmap.hook_retention_wpm, 120.0, "20 words in 10s");
        assert!(heatmap.overall_wpm < heatmap.hook_retention_wpm);
        assert!(heatmap.hook_is_front_loaded());
    }

    #[test]
    fn the_hook_is_measured_over_the_window_not_the_words() {
        // Speech starts late: the same words score lower, which is the point.
        let late: Vec<TimedWord> = (0..5)
            .map(|index| {
                let start = 8.0 + index as f32 * 0.5;
                TimedWord { start, end: start + 0.4, gap_before: 0.1 }
            })
            .collect();

        let heatmap = build("v1", &late, 20.0, None);
        assert_eq!(heatmap.hook_retention_wpm, 24.0, "4 words inside 10s");
    }

    #[test]
    fn cuts_are_counted_only_where_a_cut_would_fit() {
        let mut words = steady(10);
        // Two long silences, plus the existing 0.1s articulation gaps.
        words[3].gap_before = 1.2;
        words[7].gap_before = 0.8;

        let heatmap = build("v1", &words, 60.0, None);
        assert_eq!(heatmap.jump_cut_frequency, 2.0, "2 cuts per minute");
        assert!((heatmap.max_gap_sec - 1.2).abs() < 1e-5);
        assert!(heatmap.mean_gap_sec > 0.1 && heatmap.mean_gap_sec < 0.4);
    }

    #[test]
    fn windows_tile_the_duration_and_report_silence() {
        let heatmap = build("v1", &steady(20), 10.0, None);

        assert_eq!(heatmap.windows.len(), 2, "10s at 5s resolution");
        assert_eq!(heatmap.windows[0].start_sec, 0.0);
        assert_eq!(heatmap.windows[1].end_sec, 10.0);
        // 0.4s spoken of every 0.5s: 20% silence.
        assert!((heatmap.windows[0].silence_ratio - 0.2).abs() < 0.05);
        assert!(heatmap.peak_window().is_some());
    }

    #[test]
    fn broll_density_comes_from_motion_and_is_absent_without_it() {
        let visual = VisualTimeline {
            spikes: (0..6)
                .map(|i| MotionSpike { timestamp_sec: i as f32 * 10.0, intensity: 50.0 })
                .collect(),
            avg_intensity: 12.0,
            sampled_frames: 900,
            duration_sec: 60.0,
        };

        let with_visual = build("v1", &steady(10), 60.0, Some(&visual));
        assert_eq!(with_visual.broll_density, Some(6.0), "6 cutaways per minute");

        let without = build("v1", &steady(10), 60.0, None);
        assert_eq!(without.broll_density, None, "unknown, not zero");
    }

    #[test]
    fn an_empty_transcript_produces_a_zeroed_heatmap_not_a_panic() {
        let heatmap = build("v1", &[], 0.0, None);

        assert_eq!(heatmap.overall_wpm, 0.0);
        assert_eq!(heatmap.hook_retention_wpm, 0.0);
        assert_eq!(heatmap.jump_cut_frequency, 0.0);
        assert!(heatmap.windows.is_empty());
        assert!(!heatmap.hook_is_front_loaded());
    }

    #[test]
    fn the_duration_never_ends_before_the_last_word() {
        // A wrong container duration must not inflate the WPM.
        let heatmap = build("v1", &steady(20), 1.0, None);
        assert!(heatmap.duration_sec >= 9.9, "{}", heatmap.duration_sec);
    }
}
