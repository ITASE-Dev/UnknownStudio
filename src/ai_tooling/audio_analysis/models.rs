//! The analysis document, exactly as it is written to disk.
//!
//! Every field is explicit: this file is the schema contract, so a reader
//! elsewhere can rely on the shape without consulting the writer.

use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: &str = "1.0";

/// Root document.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TranscriptOutput {
    pub schema_version: String,
    pub media: Media,
    pub meta: Meta,
    pub config: AnalysisConfig,
    pub transcript: Transcript,
}

impl TranscriptOutput {
    pub fn new(media: Media, meta: Meta, config: AnalysisConfig, transcript: Transcript) -> Self {
        Self {
            schema_version: SCHEMA_VERSION.to_string(),
            media,
            meta,
            config,
            transcript,
        }
    }
}

/// The file that was analysed, and the PCM it was decoded to.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Media {
    pub path: String,
    pub filename: String,
    pub duration_sec: f32,
    /// Rate of the analysed PCM, not necessarily the source's.
    pub sample_rate: u32,
    pub channels: u16,
}

/// How and when this document was produced.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Meta {
    /// RFC 3339, UTC.
    pub generated_at: String,
    pub generator: String,
    pub generator_version: String,
    /// Detected or forced language, when known.
    pub language: Option<String>,
    /// Wall-clock cost of the run.
    pub analysis_duration_sec: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AnalysisConfig {
    pub transcription: TranscriptionConfig,
    pub pacing: PacingConfig,
    pub energy: EnergyConfig,
}

impl Default for AnalysisConfig {
    fn default() -> Self {
        Self {
            transcription: TranscriptionConfig::default(),
            pacing: PacingConfig::default(),
            energy: EnergyConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TranscriptionConfig {
    /// Path to the ggml/gguf weights.
    pub model_path: String,
    /// `None` lets Whisper detect the language.
    pub language: Option<String>,
    pub word_timestamps: bool,
    pub translate: bool,
    pub threads: u16,
}

impl Default for TranscriptionConfig {
    fn default() -> Self {
        Self {
            model_path: String::new(),
            language: None,
            word_timestamps: true,
            translate: false,
            // Leaves a core for the UI on a typical machine.
            threads: 4,
        }
    }
}

/// Thresholds that turn word timings into pacing judgements.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PacingConfig {
    /// A gap this long reads as a beat worth cutting.
    pub long_pause_sec: f32,
    /// Below this a gap is just articulation, not a pause.
    pub short_pause_sec: f32,
    /// Pace the edit aims for.
    pub target_wpm: f32,
}

impl Default for PacingConfig {
    fn default() -> Self {
        Self {
            long_pause_sec: 0.6,
            short_pause_sec: 0.15,
            target_wpm: 150.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct EnergyConfig {
    pub sample_rate: u32,
    /// At or below this a slice counts as silence.
    pub silence_dbfs: f32,
    /// Floor written instead of `-inf`, which JSON cannot represent.
    pub floor_dbfs: f32,
}

impl Default for EnergyConfig {
    fn default() -> Self {
        Self {
            sample_rate: crate::ai_tooling::audio_analysis::extraction::TARGET_SAMPLE_RATE,
            silence_dbfs: -45.0,
            floor_dbfs: crate::ai_tooling::audio_analysis::dsp::FLOOR_DBFS,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Transcript {
    /// Full text, segments joined.
    pub text: String,
    pub segments: Vec<Segment>,
    pub words: Vec<Word>,
    pub stats: TranscriptStats,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Segment {
    pub id: u32,
    pub start: f32,
    pub end: f32,
    pub text: String,
}

/// One word, with everything the editor needs to cut around it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Word {
    pub text: String,
    pub start: f32,
    pub end: f32,
    /// Whisper's confidence, 0.0..=1.0.
    pub probability: f32,
    /// Silence before this word: `start - previous.end`. Zero for the first.
    pub gap_before: f32,
    /// Mean level across the word's own audio, floored rather than `-inf`.
    pub mean_dbfs: f32,
}

impl Word {
    pub fn duration(&self) -> f32 {
        (self.end - self.start).max(0.0)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TranscriptStats {
    pub word_count: usize,
    /// Words per minute across the speaking time, not the whole clip.
    pub words_per_minute: f32,
    pub speaking_time_sec: f32,
    pub silence_time_sec: f32,
    /// Gaps at or above `PacingConfig::long_pause_sec`.
    pub long_pause_count: usize,
    pub mean_probability: f32,
    pub mean_dbfs: f32,
}

impl TranscriptStats {
    /// Derives the summary from the words themselves, so it can never disagree
    /// with the list it summarises.
    pub fn from_words(words: &[Word], pacing: &PacingConfig) -> Self {
        if words.is_empty() {
            return Self::default();
        }

        let speaking_time_sec: f32 = words.iter().map(Word::duration).sum();
        let silence_time_sec: f32 = words.iter().map(|word| word.gap_before.max(0.0)).sum();
        let long_pause_count = words
            .iter()
            .filter(|word| word.gap_before >= pacing.long_pause_sec)
            .count();

        let count = words.len() as f32;
        let words_per_minute = if speaking_time_sec > 0.0 {
            count / (speaking_time_sec / 60.0)
        } else {
            0.0
        };

        Self {
            word_count: words.len(),
            words_per_minute,
            speaking_time_sec,
            silence_time_sec,
            long_pause_count,
            mean_probability: words.iter().map(|word| word.probability).sum::<f32>() / count,
            mean_dbfs: words.iter().map(|word| word.mean_dbfs).sum::<f32>() / count,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(text: &str, start: f32, end: f32, gap: f32) -> Word {
        Word {
            text: text.into(),
            start,
            end,
            probability: 0.9,
            gap_before: gap,
            mean_dbfs: -20.0,
        }
    }

    #[test]
    fn the_document_serializes_in_snake_case_with_its_version() {
        let output = TranscriptOutput::new(
            Media {
                path: "/clips/a.mp4".into(),
                filename: "a.mp4".into(),
                duration_sec: 12.0,
                sample_rate: 16_000,
                channels: 1,
            },
            Meta {
                generated_at: "2026-01-01T00:00:00Z".into(),
                generator: "unknown_studio".into(),
                generator_version: "0.1.0".into(),
                language: Some("en".into()),
                analysis_duration_sec: 3.5,
            },
            AnalysisConfig::default(),
            Transcript::default(),
        );

        let json = serde_json::to_value(&output).expect("serialize");
        assert_eq!(json["schema_version"], "1.0");
        assert_eq!(json["media"]["duration_sec"], 12.0);
        assert_eq!(json["meta"]["generated_at"], "2026-01-01T00:00:00Z");
        assert_eq!(json["config"]["transcription"]["word_timestamps"], true);
        assert_eq!(json["config"]["pacing"]["target_wpm"], 150.0);
        assert!(json["config"]["energy"]["silence_dbfs"].is_number());
        assert!(json["transcript"]["words"].is_array());
    }

    #[test]
    fn a_word_round_trips_with_every_metric() {
        let word = word("hello", 1.0, 1.4, 0.25);
        let json = serde_json::to_value(&word).expect("serialize");

        assert_eq!(json["text"], "hello");
        assert_eq!(json["gap_before"], 0.25);
        assert_eq!(json["mean_dbfs"], -20.0);
        // f32 widens to f64 on the way out, so compare with a tolerance.
        let probability = json["probability"].as_f64().expect("number");
        assert!((probability - 0.9).abs() < 1e-6);

        let parsed: Word = serde_json::from_value(json).expect("deserialize");
        assert!((parsed.duration() - 0.4).abs() < 1e-6);
    }

    #[test]
    fn stats_are_derived_from_the_words_they_describe() {
        let words = vec![
            word("one", 0.0, 0.5, 0.0),
            word("two", 0.6, 1.1, 0.1),
            word("three", 2.0, 2.5, 0.9),
        ];

        let stats = TranscriptStats::from_words(&words, &PacingConfig::default());
        assert_eq!(stats.word_count, 3);
        assert!((stats.speaking_time_sec - 1.5).abs() < 1e-5);
        assert!((stats.silence_time_sec - 1.0).abs() < 1e-5);
        assert_eq!(stats.long_pause_count, 1, "only the 0.9s gap is a beat");
        assert!((stats.words_per_minute - 120.0).abs() < 0.1);
        assert!((stats.mean_probability - 0.9).abs() < 1e-5);
    }

    #[test]
    fn an_empty_transcript_reports_zeros_rather_than_nan() {
        let stats = TranscriptStats::from_words(&[], &PacingConfig::default());
        assert_eq!(stats.word_count, 0);
        assert_eq!(stats.words_per_minute, 0.0);
        assert!(stats.mean_dbfs.is_finite());
    }
}
