//! Kinetic captions and automatic sound design.
//!
//! Both halves work from measurements the project already has — Whisper word
//! timings, motion spikes, and the cuts a blueprint proposed — so every caption
//! sits on real speech and every effect lands on a real event.

use crate::ai_tooling::audio_analysis::models::{TranscriptOutput, Word};
use crate::ai_tooling::orchestration::dispatcher::ActionCommand;
use crate::ai_tooling::orchestration::models::{TextAnimation, TextStyle};
use crate::ai_tooling::visual_analysis::models::VisualTimeline;

/// Built-in effect ids the host resolves to files.
pub const SFX_BOOM: &str = "sfx_boom";
pub const SFX_WHOOSH: &str = "sfx_whoosh";
pub const SFX_RISER: &str = "sfx_riser";

/// Words per caption. Three fits a phone screen at a readable size; five is the
/// most a viewer takes in before the line changes.
const MIN_WORDS: usize = 3;
const MAX_WORDS: usize = 5;

/// A silence this long ends the caption early — a line should not straddle a
/// pause the viewer can hear.
const CAPTION_BREAK_SEC: f32 = 0.6;

/// Captions shorter than this are held anyway, or they flash.
const MIN_CAPTION_SEC: f32 = 0.4;

/// Below this confidence a word is not emphasised: an uncertain transcription
/// is the last thing that should be animated into the middle of the screen.
const EMPHASIS_MIN_PROBABILITY: f32 = 0.65;

/// Words that carry the beat of a sentence. Deliberately small and readable —
/// a longer list is a model's job, not a constant's.
const EMPHASIS_WORDS: [&str; 34] = [
    "never", "always", "everything", "nothing", "everyone", "nobody", "huge", "massive", "insane",
    "crazy", "wrong", "right", "best", "worst", "first", "last", "free", "instantly", "actually",
    "literally", "seriously", "stop", "watch", "look", "listen", "wait", "remember", "warning",
    "secret", "mistake", "hack", "boom", "wow", "why",
];

#[derive(Debug, Clone, Copy)]
pub struct CaptionSettings {
    pub min_words: usize,
    pub max_words: usize,
    /// Animation applied to a line containing an emphasised word.
    pub emphasis_animation: TextAnimation,
    pub base_animation: TextAnimation,
}

impl Default for CaptionSettings {
    fn default() -> Self {
        Self {
            min_words: MIN_WORDS,
            max_words: MAX_WORDS,
            emphasis_animation: TextAnimation::Pop,
            base_animation: TextAnimation::SlideUp,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SfxSettings {
    /// A motion spike with speech this close is scored, not empty — dropping a
    /// boom under a sentence buries the line.
    pub speech_guard_sec: f32,
    /// Effects closer together than this are thinned; a stack of booms is noise.
    pub min_spacing_sec: f32,
    pub boom_db: f32,
    pub whoosh_db: f32,
    pub riser_db: f32,
    /// A riser leads its cut, so it resolves on the edit.
    pub riser_lead_sec: f32,
}

impl Default for SfxSettings {
    fn default() -> Self {
        Self {
            speech_guard_sec: 0.4,
            min_spacing_sec: 1.0,
            boom_db: -6.0,
            whoosh_db: -12.0,
            riser_db: -10.0,
            riser_lead_sec: 0.8,
        }
    }
}

/// Turns word timings into caption commands.
pub fn generate_captions(
    transcript: &TranscriptOutput,
    settings: CaptionSettings,
) -> Vec<ActionCommand> {
    chunk_words(&transcript.transcript.words, settings)
        .into_iter()
        .map(|chunk| caption_for(&chunk, settings))
        .collect()
}

/// Groups words into readable lines, breaking early on audible pauses.
fn chunk_words<'a>(words: &'a [Word], settings: CaptionSettings) -> Vec<Vec<&'a Word>> {
    let max = settings.max_words.max(1);
    let min = settings.min_words.clamp(1, max);

    let mut chunks: Vec<Vec<&Word>> = Vec::new();
    let mut current: Vec<&Word> = Vec::new();

    for word in words {
        // A long silence ends the line, provided it is already readable.
        if !current.is_empty() && word.gap_before >= CAPTION_BREAK_SEC && current.len() >= min {
            chunks.push(std::mem::take(&mut current));
        }
        current.push(word);
        if current.len() >= max {
            chunks.push(std::mem::take(&mut current));
        }
    }

    // A short tail joins the previous line rather than flashing on its own.
    if !current.is_empty() {
        match chunks.last_mut() {
            Some(last) if current.len() < min && last.len() + current.len() <= max + 1 => {
                last.extend(current);
            }
            _ => chunks.push(current),
        }
    }
    chunks
}

fn caption_for(chunk: &[&Word], settings: CaptionSettings) -> ActionCommand {
    let start = chunk.first().map_or(0.0, |word| word.start);
    let end = chunk
        .last()
        .map_or(start + MIN_CAPTION_SEC, |word| word.end)
        .max(start + MIN_CAPTION_SEC);

    let emphasised = chunk.iter().any(|word| is_emphasis(word));
    let text = chunk
        .iter()
        .map(|word| word.text.trim())
        .collect::<Vec<_>>()
        .join(" ");

    ActionCommand::AddText {
        start_sec: start,
        end_sec: end,
        text,
        animation: if emphasised {
            settings.emphasis_animation
        } else {
            settings.base_animation
        },
        style: if emphasised {
            TextStyle::Highlight
        } else {
            TextStyle::Default
        },
    }
}

/// Whether a word carries enough weight to animate.
///
/// Confidence is a gate, not a reason: Whisper being sure of "the" says nothing.
/// The word also has to be one that lands — an emphasis word, a number, or
/// something the speaker shouted.
pub fn is_emphasis(word: &Word) -> bool {
    if word.probability < EMPHASIS_MIN_PROBABILITY {
        return false;
    }

    let cleaned: String = word
        .text
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect();
    if cleaned.is_empty() {
        return false;
    }

    let shouted = cleaned.len() > 2 && cleaned.chars().all(|c| c.is_uppercase() || c.is_numeric());
    let numeric = cleaned.chars().any(|c| c.is_numeric());
    let listed = EMPHASIS_WORDS.contains(&cleaned.to_ascii_lowercase().as_str());

    shouted || numeric || listed
}

/// One placed effect, before it becomes a command.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Cue {
    at: f32,
    file_id: &'static str,
    volume_db: f32,
}

/// Places sound effects against motion, speech and the blueprint's own cuts.
pub fn generate_sfx(
    transcript: &TranscriptOutput,
    visual: Option<&VisualTimeline>,
    planned: &[ActionCommand],
    settings: SfxSettings,
) -> Vec<ActionCommand> {
    let mut cues: Vec<Cue> = Vec::new();

    // Impacts the audience sees but does not hear anyone talk over.
    if let Some(visual) = visual {
        let threshold = spike_threshold(visual);
        for spike in &visual.spikes {
            if spike.intensity < threshold {
                continue;
            }
            if speech_near(transcript, spike.timestamp_sec, settings.speech_guard_sec) {
                continue;
            }
            cues.push(Cue {
                at: spike.timestamp_sec,
                file_id: SFX_BOOM,
                volume_db: settings.boom_db,
            });
        }
    }

    // Transitions the plan already introduced.
    for action in planned {
        match action {
            ActionCommand::SplitClip { time_sec, .. } => cues.push(Cue {
                at: *time_sec,
                file_id: SFX_WHOOSH,
                volume_db: settings.whoosh_db,
            }),
            // A B-roll marker gets a riser that resolves on the insert.
            ActionCommand::AddMarker { time_sec, label, .. } if label.contains("B-roll") => {
                cues.push(Cue {
                    at: (time_sec - settings.riser_lead_sec).max(0.0),
                    file_id: SFX_RISER,
                    volume_db: settings.riser_db,
                });
            }
            _ => {}
        }
    }

    thin(&mut cues, settings.min_spacing_sec);
    cues.into_iter()
        .map(|cue| ActionCommand::AddAudio {
            start_sec: cue.at,
            file_id: cue.file_id.to_string(),
            volume_db: cue.volume_db,
        })
        .collect()
}

/// Captions and effects together, captions first so the plan reads in layers.
pub fn enhance(
    transcript: &TranscriptOutput,
    visual: Option<&VisualTimeline>,
    planned: &[ActionCommand],
    captions: CaptionSettings,
    sfx: SfxSettings,
) -> Vec<ActionCommand> {
    let mut actions = generate_captions(transcript, captions);
    actions.extend(generate_sfx(transcript, visual, planned, sfx));
    actions
}

/// Only spikes well above the clip's own energy earn an effect; on busy footage
/// a fixed threshold would fire constantly.
fn spike_threshold(visual: &VisualTimeline) -> f32 {
    (visual.avg_intensity * 2.0).max(f32::EPSILON)
}

fn speech_near(transcript: &TranscriptOutput, at: f32, guard: f32) -> bool {
    transcript
        .transcript
        .words
        .iter()
        .any(|word| word.start - guard <= at && at <= word.end + guard)
}

/// Keeps the first cue in any cluster and drops the rest.
fn thin(cues: &mut Vec<Cue>, min_spacing: f32) {
    cues.sort_by(|a, b| a.at.total_cmp(&b.at));

    let mut last: Option<f32> = None;
    cues.retain(|cue| match last {
        Some(previous) if cue.at - previous < min_spacing => false,
        _ => {
            last = Some(cue.at);
            true
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_tooling::audio_analysis::models::{
        AnalysisConfig, Media, Meta, Transcript, TranscriptStats,
    };
    use crate::ai_tooling::visual_analysis::models::MotionSpike;

    fn word(text: &str, start: f32, gap: f32, probability: f32) -> Word {
        Word {
            text: text.into(),
            start,
            end: start + 0.3,
            probability,
            gap_before: gap,
            mean_dbfs: -18.0,
        }
    }

    fn transcript(words: Vec<Word>) -> TranscriptOutput {
        let duration = words.last().map_or(0.0, |word| word.end);
        let stats = TranscriptStats::from_words(&words, &AnalysisConfig::default().pacing);

        TranscriptOutput::new(
            Media {
                path: "a.mp4".into(),
                filename: "a.mp4".into(),
                duration_sec: duration,
                sample_rate: 16_000,
                channels: 1,
            },
            Meta {
                generated_at: "2026-01-01T00:00:00Z".into(),
                generator: "test".into(),
                generator_version: "0".into(),
                language: None,
                analysis_duration_sec: 0.0,
            },
            AnalysisConfig::default(),
            Transcript { text: String::new(), segments: Vec::new(), words, stats },
        )
    }

    /// Nine evenly spoken words, no pauses.
    fn steady() -> TranscriptOutput {
        transcript(
            (0..9)
                .map(|index| word(&format!("w{index}"), index as f32 * 0.5, 0.1, 0.9))
                .collect(),
        )
    }

    fn captions(output: &[ActionCommand]) -> Vec<(String, f32, f32, TextAnimation, TextStyle)> {
        output
            .iter()
            .filter_map(|action| match action {
                ActionCommand::AddText { text, start_sec, end_sec, animation, style } => {
                    Some((text.clone(), *start_sec, *end_sec, *animation, *style))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn captions_hold_three_to_five_words_and_follow_the_speech() {
        let lines = captions(&generate_captions(&steady(), CaptionSettings::default()));

        assert!(!lines.is_empty());
        for (text, start, end, _, _) in &lines {
            let count = text.split_whitespace().count();
            assert!((MIN_WORDS..=MAX_WORDS + 1).contains(&count), "{text}");
            assert!(end > start, "a caption must be on screen");
        }
        // Lines run in order and never overlap their neighbours' starts.
        assert!(lines.windows(2).all(|pair| pair[0].1 < pair[1].1));
    }

    #[test]
    fn a_pause_breaks_the_line_early() {
        let words = vec![
            word("one", 0.0, 0.0, 0.9),
            word("two", 0.5, 0.1, 0.9),
            word("three", 1.0, 0.1, 0.9),
            // A full second of silence: the next line starts here.
            word("four", 2.5, 1.2, 0.9),
            word("five", 3.0, 0.1, 0.9),
            word("six", 3.5, 0.1, 0.9),
        ];
        let lines = captions(&generate_captions(&transcript(words), CaptionSettings::default()));

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].0, "one two three");
        assert_eq!(lines[1].0, "four five six");
    }

    #[test]
    fn an_emphasis_word_animates_the_line_it_lands_in() {
        let words = vec![
            word("this", 0.0, 0.0, 0.9),
            word("is", 0.5, 0.1, 0.9),
            word("insane", 1.0, 0.1, 0.9),
        ];
        let lines = captions(&generate_captions(&transcript(words), CaptionSettings::default()));

        assert_eq!(lines[0].3, TextAnimation::Pop);
        assert_eq!(lines[0].4, TextStyle::Highlight);
    }

    #[test]
    fn an_ordinary_line_is_not_animated_for_emphasis() {
        let words = vec![
            word("and", 0.0, 0.0, 0.99),
            word("then", 0.5, 0.1, 0.99),
            word("the", 1.0, 0.1, 0.99),
        ];
        let lines = captions(&generate_captions(&transcript(words), CaptionSettings::default()));

        // High confidence alone is not a reason to shout.
        assert_eq!(lines[0].3, TextAnimation::SlideUp);
        assert_eq!(lines[0].4, TextStyle::Default);
    }

    #[test]
    fn emphasis_requires_confidence_as_well_as_weight() {
        assert!(is_emphasis(&word("insane", 0.0, 0.0, 0.9)));
        assert!(!is_emphasis(&word("insane", 0.0, 0.0, 0.3)), "unsure: leave it alone");
        assert!(is_emphasis(&word("$5,000", 0.0, 0.0, 0.9)), "numbers land");
        assert!(is_emphasis(&word("STOP", 0.0, 0.0, 0.9)), "shouted");
        assert!(!is_emphasis(&word("the", 0.0, 0.0, 0.99)));
        assert!(!is_emphasis(&word("...", 0.0, 0.0, 0.99)), "punctuation is not a word");
    }

    #[test]
    fn a_boom_lands_on_a_motion_spike_with_no_one_talking() {
        // Speech stops at ~1.2s; the spike is at 5s.
        let visual = VisualTimeline {
            spikes: vec![MotionSpike { timestamp_sec: 5.0, intensity: 80.0 }],
            avg_intensity: 10.0,
            sampled_frames: 100,
            duration_sec: 8.0,
        };

        let sfx = generate_sfx(&steady(), Some(&visual), &[], SfxSettings::default());
        assert_eq!(sfx.len(), 1);
        assert!(matches!(
            &sfx[0],
            ActionCommand::AddAudio { file_id, start_sec, volume_db }
                if file_id == SFX_BOOM && *start_sec == 5.0 && *volume_db < 0.0
        ));
    }

    #[test]
    fn a_spike_under_speech_is_left_alone() {
        // 0.5s is mid-sentence in `steady()`.
        let visual = VisualTimeline {
            spikes: vec![MotionSpike { timestamp_sec: 0.5, intensity: 80.0 }],
            avg_intensity: 10.0,
            sampled_frames: 100,
            duration_sec: 8.0,
        };

        let sfx = generate_sfx(&steady(), Some(&visual), &[], SfxSettings::default());
        assert!(sfx.is_empty(), "a boom would bury the line");
    }

    #[test]
    fn a_weak_spike_on_busy_footage_earns_nothing() {
        let visual = VisualTimeline {
            spikes: vec![MotionSpike { timestamp_sec: 6.0, intensity: 12.0 }],
            avg_intensity: 10.0,
            sampled_frames: 100,
            duration_sec: 8.0,
        };

        assert!(generate_sfx(&steady(), Some(&visual), &[], SfxSettings::default()).is_empty());
    }

    #[test]
    fn cuts_get_a_whoosh_and_broll_marks_get_a_riser_that_leads_them() {
        let planned = vec![
            ActionCommand::SplitClip { clip_id: "c1".into(), time_sec: 10.0 },
            ActionCommand::AddMarker {
                time_sec: 20.0,
                color: "purple".into(),
                label: "B-roll".into(),
            },
        ];

        let sfx = generate_sfx(&steady(), None, &planned, SfxSettings::default());
        let placed: Vec<(String, f32)> = sfx
            .iter()
            .filter_map(|action| match action {
                ActionCommand::AddAudio { file_id, start_sec, .. } => {
                    Some((file_id.clone(), *start_sec))
                }
                _ => None,
            })
            .collect();

        assert_eq!(placed[0], (SFX_WHOOSH.to_string(), 10.0));
        // The riser starts before the insert so it resolves on it.
        assert_eq!(placed[1], (SFX_RISER.to_string(), 19.2));
    }

    #[test]
    fn a_cluster_of_effects_is_thinned_to_one() {
        let planned: Vec<ActionCommand> = (0..5)
            .map(|index| ActionCommand::SplitClip {
                clip_id: "c1".into(),
                time_sec: 10.0 + index as f32 * 0.1,
            })
            .collect();

        let sfx = generate_sfx(&steady(), None, &planned, SfxSettings::default());
        assert_eq!(sfx.len(), 1, "five whooshes in half a second is noise");
    }

    #[test]
    fn enhance_returns_captions_before_effects() {
        let planned = vec![ActionCommand::SplitClip { clip_id: "c1".into(), time_sec: 10.0 }];
        let actions = enhance(
            &steady(),
            None,
            &planned,
            CaptionSettings::default(),
            SfxSettings::default(),
        );

        let first_audio = actions
            .iter()
            .position(|action| matches!(action, ActionCommand::AddAudio { .. }))
            .expect("an effect");
        let last_text = actions
            .iter()
            .rposition(|action| matches!(action, ActionCommand::AddText { .. }))
            .expect("a caption");

        assert!(last_text < first_audio, "captions layer first");
    }

    #[test]
    fn an_empty_transcript_produces_no_captions() {
        assert!(generate_captions(&transcript(Vec::new()), CaptionSettings::default()).is_empty());
    }
}
