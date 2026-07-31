//! Frame-domain adapter over `audio_engine::analysis`. The action engine thinks
//! in timeline frames; the audio engine thinks in seconds and samples.

use crate::action_engine::edits::FrameRange;
use crate::action_engine::types::{ClipSnapshot, EngineError};
use crate::audio_engine::analysis::{
    decode_mono_pcm, encode_wav_pcm16, silent_runs, ANALYSIS_SAMPLE_RATE,
    DEFAULT_MIN_SILENCE_SECONDS, DEFAULT_SILENCE_THRESHOLD_DB,
};

/// Clip audio as a Whisper-ready WAV.
pub fn clip_wav(clip: &ClipSnapshot) -> Result<Vec<u8>, EngineError> {
    let samples = clip_samples(clip)?;
    Ok(encode_wav_pcm16(&samples, ANALYSIS_SAMPLE_RATE))
}

/// Silence inside the clip, as absolute timeline frame ranges.
pub fn silence_ranges(clip: &ClipSnapshot) -> Result<Vec<FrameRange>, EngineError> {
    let samples = clip_samples(clip)?;
    let fps = clip.fps();
    let min_run = (fps * DEFAULT_MIN_SILENCE_SECONDS).round() as u64;

    // One slice per video frame, so the ranges land on frame boundaries.
    let runs = silent_runs(
        &samples,
        ANALYSIS_SAMPLE_RATE,
        fps,
        clip.duration_frames,
        DEFAULT_SILENCE_THRESHOLD_DB,
        min_run,
    );

    Ok(runs
        .into_iter()
        .map(|(start, end)| FrameRange::new(clip.start_frame + start, clip.start_frame + end))
        .collect())
}

fn clip_samples(clip: &ClipSnapshot) -> Result<Vec<f32>, EngineError> {
    let (start, duration) = clip.source_window();
    let samples = decode_mono_pcm(&clip.source_path, start, duration).map_err(EngineError::Media)?;
    if samples.is_empty() {
        return Err(EngineError::Media("clip has no audio stream".into()));
    }
    Ok(samples)
}
