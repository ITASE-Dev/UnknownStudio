//! Orchestration: extract, transcribe, measure, assemble the document.

use crate::ai_tooling::audio_analysis::dsp;
use crate::ai_tooling::audio_analysis::extraction::{extract_pcm, PcmBuffer};
use crate::ai_tooling::audio_analysis::models::{
    AnalysisConfig, Media, Meta, Segment, Transcript, TranscriptOutput, TranscriptStats, Word,
};
use crate::ai_tooling::audio_analysis::AudioAnalysisError;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const GENERATOR: &str = "unknown_studio.audio_analysis";

/// One transcribed word before any measurement is attached.
#[derive(Debug, Clone, PartialEq)]
pub struct RawWord {
    pub text: String,
    pub start: f32,
    pub end: f32,
    pub probability: f32,
}

/// Transcript as the recogniser produced it.
#[derive(Debug, Clone, Default)]
pub struct RawTranscription {
    pub words: Vec<RawWord>,
    pub segments: Vec<Segment>,
    pub language: Option<String>,
}

/// Transcribes a file and measures each word against its own audio.
pub async fn analyze_audio(
    media_path: &Path,
    config: AnalysisConfig,
) -> Result<TranscriptOutput, AudioAnalysisError> {
    let started = Instant::now();
    let pcm = extract_pcm(media_path).await?;

    if pcm.is_empty() {
        return Err(AudioAnalysisError::NoAudio(media_path.to_path_buf()));
    }

    // Whisper is CPU-bound and blocking; it must not sit on an async worker.
    let samples = pcm.samples.clone();
    let transcription_config = config.transcription.clone();
    let raw = tokio::task::spawn_blocking(move || transcribe(&samples, &transcription_config))
        .await
        .map_err(|err| AudioAnalysisError::Transcription(format!("task failed: {err}")))??;

    Ok(assemble(media_path, &pcm, raw, config, started.elapsed().as_secs_f32()))
}

/// Attaches `gap_before` and `mean_dbfs` to each word and builds the document.
///
/// Split out from the async path so the measurement logic can be tested with a
/// synthetic transcript and buffer, with no model or ffmpeg involved.
pub fn assemble(
    media_path: &Path,
    pcm: &PcmBuffer,
    raw: RawTranscription,
    config: AnalysisConfig,
    analysis_duration_sec: f32,
) -> TranscriptOutput {
    let mut words = Vec::with_capacity(raw.words.len());
    let mut previous_end: Option<f32> = None;

    for word in raw.words {
        words.push(Word {
            gap_before: dsp::gap_before(previous_end, word.start),
            mean_dbfs: dsp::mean_dbfs_for_span(
                &pcm.samples,
                pcm.sample_rate,
                word.start,
                word.end,
            ),
            text: word.text,
            start: word.start,
            end: word.end,
            probability: word.probability,
        });
        previous_end = Some(word.end);
    }

    let text = if raw.segments.is_empty() {
        words
            .iter()
            .map(|word| word.text.trim())
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        raw.segments
            .iter()
            .map(|segment| segment.text.trim())
            .collect::<Vec<_>>()
            .join(" ")
    };

    let stats = TranscriptStats::from_words(&words, &config.pacing);

    TranscriptOutput::new(
        media(media_path, pcm),
        Meta {
            generated_at: rfc3339_now(),
            generator: GENERATOR.to_string(),
            generator_version: env!("CARGO_PKG_VERSION").to_string(),
            language: raw.language.or_else(|| config.transcription.language.clone()),
            analysis_duration_sec,
        },
        config,
        Transcript {
            text,
            segments: raw.segments,
            words,
            stats,
        },
    )
}

fn media(media_path: &Path, pcm: &PcmBuffer) -> Media {
    Media {
        path: media_path.to_string_lossy().into_owned(),
        filename: media_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        duration_sec: pcm.duration_sec(),
        sample_rate: pcm.sample_rate,
        channels: pcm.channels,
    }
}

/// Writes the document beside the media, as `<name>.audio.json`.
pub async fn write_report(
    output: &TranscriptOutput,
    directory: &Path,
) -> Result<PathBuf, AudioAnalysisError> {
    tokio::fs::create_dir_all(directory).await?;

    let stem = Path::new(&output.media.filename)
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "audio".to_string());
    let path = directory.join(format!("{stem}.audio.json"));

    let json = serde_json::to_vec_pretty(output)?;
    tokio::fs::write(&path, &json).await?;
    Ok(path)
}

#[cfg(feature = "audio-analysis")]
fn transcribe(
    samples: &[f32],
    config: &crate::ai_tooling::audio_analysis::models::TranscriptionConfig,
) -> Result<RawTranscription, AudioAnalysisError> {
    use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

    let failed = |message: String| AudioAnalysisError::Transcription(message);

    let context = WhisperContext::new_with_params(
        &config.model_path,
        WhisperContextParameters::default(),
    )
    .map_err(|err| failed(format!("cannot load model {}: {err}", config.model_path)))?;
    let mut state = context
        .create_state()
        .map_err(|err| failed(err.to_string()))?;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_n_threads(config.threads.max(1) as i32);
    params.set_translate(config.translate);
    params.set_language(config.language.as_deref());
    // Word-level timing: token timestamps, split on word boundaries.
    params.set_token_timestamps(config.word_timestamps);
    params.set_split_on_word(config.word_timestamps);
    // The C++ side prints to stdout unless every channel is silenced.
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);

    state
        .full(params, samples)
        .map_err(|err| failed(err.to_string()))?;

    let segment_count = state.full_n_segments().map_err(|err| failed(err.to_string()))?;
    let mut transcription = RawTranscription {
        language: config.language.clone(),
        ..RawTranscription::default()
    };

    for index in 0..segment_count {
        let text = state
            .full_get_segment_text(index)
            .map_err(|err| failed(err.to_string()))?;
        let start = state
            .full_get_segment_t0(index)
            .map_err(|err| failed(err.to_string()))?;
        let end = state
            .full_get_segment_t1(index)
            .map_err(|err| failed(err.to_string()))?;

        transcription.segments.push(Segment {
            id: index as u32,
            start: centiseconds(start),
            end: centiseconds(end),
            text: text.trim().to_string(),
        });

        let token_count = state
            .full_n_tokens(index)
            .map_err(|err| failed(err.to_string()))?;
        for token in 0..token_count {
            let raw = state
                .full_get_token_text(index, token)
                .map_err(|err| failed(err.to_string()))?;
            // Special tokens ([_BEG_], timestamps) carry no speech.
            if raw.starts_with("[_") || raw.trim().is_empty() {
                continue;
            }

            let data = state
                .full_get_token_data(index, token)
                .map_err(|err| failed(err.to_string()))?;
            transcription.words.push(RawWord {
                text: raw.trim().to_string(),
                start: centiseconds(data.t0),
                end: centiseconds(data.t1),
                probability: data.p,
            });
        }
    }

    Ok(transcription)
}

/// Whisper reports times in centiseconds.
#[cfg(feature = "audio-analysis")]
fn centiseconds(value: i64) -> f32 {
    value as f32 / 100.0
}

/// Without the `audio-analysis` feature the module still compiles and callers
/// still type-check; transcription reports that it is unavailable.
#[cfg(not(feature = "audio-analysis"))]
fn transcribe(
    _samples: &[f32],
    _config: &crate::ai_tooling::audio_analysis::models::TranscriptionConfig,
) -> Result<RawTranscription, AudioAnalysisError> {
    Err(AudioAnalysisError::Transcription(
        "transcription needs the 'audio-analysis' feature and a whisper build".into(),
    ))
}

fn rfc3339_now() -> String {
    // Formatted by hand rather than pulling in a date crate for one field.
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let (days, time) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    let (hour, minute, second) = (time / 3600, (time % 3600) / 60, time % 60);
    let (year, month, day) = civil_from_days(days);

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Days since the Unix epoch to a calendar date (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;

    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_tooling::audio_analysis::extraction::TARGET_SAMPLE_RATE;

    /// Two seconds where 0.0–0.5 and 1.0–1.5 are loud, the rest near-silent.
    fn buffer() -> PcmBuffer {
        let rate = TARGET_SAMPLE_RATE as usize;
        let samples = (0..rate * 2)
            .map(|index| {
                let second = index as f32 / rate as f32;
                let loud = (0.0..0.5).contains(&second) || (1.0..1.5).contains(&second);
                match (loud, index % 2 == 0) {
                    (true, true) => 0.5,
                    (true, false) => -0.5,
                    _ => 0.0,
                }
            })
            .collect();

        PcmBuffer {
            samples,
            sample_rate: TARGET_SAMPLE_RATE,
            channels: 1,
        }
    }

    fn raw() -> RawTranscription {
        RawTranscription {
            words: vec![
                RawWord { text: "hello".into(), start: 0.0, end: 0.5, probability: 0.95 },
                RawWord { text: "there".into(), start: 1.0, end: 1.5, probability: 0.80 },
                RawWord { text: "quiet".into(), start: 1.6, end: 1.9, probability: 0.70 },
            ],
            segments: vec![Segment {
                id: 0,
                start: 0.0,
                end: 1.9,
                text: "hello there quiet".into(),
            }],
            language: Some("en".into()),
        }
    }

    fn analyse() -> TranscriptOutput {
        assemble(
            Path::new("/clips/take_1.mp4"),
            &buffer(),
            raw(),
            AnalysisConfig::default(),
            1.25,
        )
    }

    #[test]
    fn gaps_are_measured_from_the_previous_word() {
        let words = analyse().transcript.words;

        assert_eq!(words[0].gap_before, 0.0, "the first word has no gap");
        assert!((words[1].gap_before - 0.5).abs() < 1e-5);
        assert!((words[2].gap_before - 0.1).abs() < 1e-5);
    }

    #[test]
    fn each_word_is_measured_against_its_own_audio() {
        let words = analyse().transcript.words;

        // Half-scale speech ≈ -6 dBFS.
        assert!((words[0].mean_dbfs + 6.02).abs() < 0.2, "{}", words[0].mean_dbfs);
        assert!((words[1].mean_dbfs + 6.02).abs() < 0.2);
        // The third word sits over silence: floored, and finite.
        assert_eq!(words[2].mean_dbfs, dsp::FLOOR_DBFS);
        assert!(words.iter().all(|word| word.mean_dbfs.is_finite()));
    }

    #[test]
    fn the_document_is_complete_and_serializable() {
        let output = analyse();

        assert_eq!(output.schema_version, "1.0");
        assert_eq!(output.media.filename, "take_1.mp4");
        assert_eq!(output.media.sample_rate, TARGET_SAMPLE_RATE);
        assert!((output.media.duration_sec - 2.0).abs() < 1e-3);
        assert_eq!(output.meta.language.as_deref(), Some("en"));
        assert_eq!(output.meta.analysis_duration_sec, 1.25);
        assert_eq!(output.transcript.text, "hello there quiet");
        assert_eq!(output.transcript.stats.word_count, 3);

        // Non-finite floats would make this fail outright.
        let json = serde_json::to_string(&output).expect("serializes");
        assert!(json.contains("\"gap_before\""));
        assert!(json.contains("\"mean_dbfs\""));
        assert!(!json.contains("inf"), "no infinities reach the file");
    }

    #[test]
    fn stats_summarise_the_words() {
        let stats = analyse().transcript.stats;

        assert!((stats.speaking_time_sec - 1.3).abs() < 1e-4);
        assert!((stats.silence_time_sec - 0.6).abs() < 1e-4);
        assert_eq!(stats.long_pause_count, 0, "0.5s is under the 0.6s threshold");
        assert!((stats.mean_probability - 0.8166).abs() < 0.01);
    }

    #[test]
    fn text_falls_back_to_the_words_when_there_are_no_segments() {
        let output = assemble(
            Path::new("a.wav"),
            &buffer(),
            RawTranscription {
                segments: Vec::new(),
                ..raw()
            },
            AnalysisConfig::default(),
            0.1,
        );
        assert_eq!(output.transcript.text, "hello there quiet");
    }

    #[test]
    fn timestamps_are_rfc3339_utc() {
        let stamp = rfc3339_now();

        assert_eq!(stamp.len(), 20, "{stamp}");
        assert!(stamp.ends_with('Z') && stamp.contains('T'));
        // A known epoch day, to prove the calendar maths rather than the clock.
        assert_eq!(civil_from_days(20_000), (2024, 10, 4));
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }

    #[tokio::test]
    async fn a_missing_file_never_reaches_the_model() {
        let result = analyze_audio(Path::new("no-such.mp4"), AnalysisConfig::default()).await;
        assert!(matches!(result, Err(AudioAnalysisError::MediaNotFound(_))));
    }
}
