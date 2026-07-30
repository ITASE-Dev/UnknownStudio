//! Audio extraction, WAV packing and deterministic silence detection.
//!
//! Silence is measured, never guessed: the LLM has no say in where cuts land.

use crate::action_engine::edits::FrameRange;
use crate::action_engine::types::{ClipSnapshot, EngineError};
use ffmpeg_next as ffmpeg;

/// 16 kHz mono — the standard input rate for speech models and small enough to
/// upload cheaply.
pub const ANALYSIS_SAMPLE_RATE: u32 = 16_000;

/// Below this dBFS a frame counts as silence.
pub const DEFAULT_SILENCE_THRESHOLD_DB: f64 = -40.0;

/// Runs shorter than half a second are natural speech pauses, not dead air.
pub const DEFAULT_MIN_SILENCE_SECONDS: f64 = 0.5;

/// Decodes `[start, start + duration)` of a source file to mono f32 PCM at
/// `ANALYSIS_SAMPLE_RATE`. Single decode path for both transcription and
/// silence analysis.
pub fn decode_mono_pcm(
    source_path: &str,
    start_seconds: f64,
    duration_seconds: f64,
) -> Result<Vec<f32>, EngineError> {
    let media = |m: String| EngineError::Media(m);

    ffmpeg::init().map_err(|err| media(err.to_string()))?;

    let mut ictx = ffmpeg::format::input(source_path).map_err(|err| media(err.to_string()))?;
    let stream = ictx
        .streams()
        .best(ffmpeg::media::Type::Audio)
        .ok_or_else(|| media("source has no audio stream".into()))?;
    let stream_index = stream.index();
    let time_base = f64::from(stream.time_base());

    let context = ffmpeg::codec::context::Context::from_parameters(stream.parameters())
        .map_err(|err| media(err.to_string()))?;
    let mut decoder = context.decoder().audio().map_err(|err| media(err.to_string()))?;

    // Sample-accurate seeking is unnecessary: a few ms of drift affects neither
    // transcription nor threshold analysis.
    if start_seconds > 0.0 && time_base > 0.0 {
        let target = (start_seconds / time_base) as i64;
        let _ = ictx.seek(target, ..target);
        decoder.flush();
    }

    let target_layout = ffmpeg::util::channel_layout::ChannelLayout::MONO;
    let target_format = ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Packed);
    let mut resampler: Option<ffmpeg::software::resampling::Context> = None;

    let wanted = ((start_seconds + duration_seconds) * ANALYSIS_SAMPLE_RATE as f64) as usize;
    let mut samples: Vec<f32> = Vec::with_capacity(wanted);
    let mut decoded = ffmpeg::frame::Audio::empty();

    'outer: for (stream, packet) in ictx.packets() {
        if stream.index() != stream_index || decoder.send_packet(&packet).is_err() {
            continue;
        }

        while decoder.receive_frame(&mut decoded).is_ok() {
            if resampler.is_none() {
                resampler = ffmpeg::software::resampling::Context::get(
                    decoded.format(),
                    decoded.channel_layout(),
                    decoded.rate(),
                    target_format,
                    target_layout,
                    ANALYSIS_SAMPLE_RATE,
                )
                .ok();
            }
            let Some(resampler) = resampler.as_mut() else {
                break 'outer;
            };

            let mut output = ffmpeg::frame::Audio::empty();
            if resampler.run(&decoded, &mut output).is_err() {
                continue;
            }
            append_mono_f32(&mut samples, &output);

            if samples.len() >= wanted {
                break 'outer;
            }
        }
    }

    // Sequential reading may have covered [0, start_seconds) too; clip to the
    // requested window.
    let skip = ((start_seconds * ANALYSIS_SAMPLE_RATE as f64) as usize).min(samples.len());
    let end = wanted.min(samples.len());
    if skip >= end {
        return Ok(Vec::new());
    }

    samples.truncate(end);
    samples.drain(..skip);
    Ok(samples)
}

fn append_mono_f32(buffer: &mut Vec<f32>, frame: &ffmpeg::frame::Audio) {
    let count = frame.samples();
    if count == 0 {
        return;
    }
    let bytes = frame.data(0);
    let usable = count.min(bytes.len() / 4);
    buffer.extend(
        bytes[..usable * 4]
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])),
    );
}

/// Deterministic silence detection.
///
/// ```text
/// samples_per_frame  = ANALYSIS_SAMPLE_RATE / fps
/// rms(frame_i)       = sqrt(mean(sample^2 in window_i))
/// dbfs(frame_i)      = 20 * log10(max(rms, 1e-9))   // floor avoids -inf
/// is_silent(frame_i) = dbfs(frame_i) < threshold_db
/// ```
/// Returns absolute timeline frame ranges.
pub fn analyze_silence(
    clip: &ClipSnapshot,
    threshold_db: f64,
    min_duration_frames: u64,
) -> Result<Vec<FrameRange>, EngineError> {
    let (start_seconds, duration_seconds) = clip.source_window();
    let samples = decode_mono_pcm(&clip.source_path, start_seconds, duration_seconds)?;
    if samples.is_empty() {
        return Err(EngineError::Media("no audio samples in clip range".into()));
    }

    let samples_per_frame = (ANALYSIS_SAMPLE_RATE as f64 / clip.fps()).max(1.0);
    let mut ranges: Vec<FrameRange> = Vec::new();
    let mut run_start: Option<u64> = None;

    for frame_idx in 0..clip.duration_frames {
        let window_start = (frame_idx as f64 * samples_per_frame) as usize;
        let window_end = ((((frame_idx + 1) as f64) * samples_per_frame) as usize).min(samples.len());

        // Audio ended before this frame (short file) — treat as silent.
        let is_silent = if window_start >= window_end {
            true
        } else {
            let window = &samples[window_start..window_end];
            let mean_square =
                window.iter().map(|s| (*s as f64) * (*s as f64)).sum::<f64>() / window.len() as f64;
            20.0 * mean_square.sqrt().max(1e-9).log10() < threshold_db
        };

        match (is_silent, run_start) {
            (true, None) => run_start = Some(frame_idx),
            (false, Some(start)) => {
                push_run(&mut ranges, clip, start, frame_idx, min_duration_frames);
                run_start = None;
            }
            _ => {}
        }
    }

    if let Some(start) = run_start {
        push_run(&mut ranges, clip, start, clip.duration_frames, min_duration_frames);
    }

    Ok(ranges)
}

fn push_run(
    ranges: &mut Vec<FrameRange>,
    clip: &ClipSnapshot,
    local_start: u64,
    local_end: u64,
    min_duration_frames: u64,
) {
    if local_end.saturating_sub(local_start) >= min_duration_frames {
        ranges.push(FrameRange::new(
            clip.start_frame + local_start,
            clip.start_frame + local_end,
        ));
    }
}

/// Minimal RIFF/WAVE (16-bit mono PCM) container. Writing the header by hand
/// avoids pulling in an FFmpeg muxer for 44 bytes.
pub fn encode_wav_pcm16(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let data_len = samples.len() * 2;
    let mut buf = Vec::with_capacity(44 + data_len);

    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    buf.extend_from_slice(b"WAVE");

    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
    buf.extend_from_slice(&1u16.to_le_bytes()); // mono
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    buf.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    buf.extend_from_slice(&2u16.to_le_bytes()); // block align
    buf.extend_from_slice(&16u16.to_le_bytes()); // bits per sample

    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&(data_len as u32).to_le_bytes());
    for sample in samples {
        let pcm = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        buf.extend_from_slice(&pcm.to_le_bytes());
    }

    buf
}

/// Extracts a clip's audio window as a Whisper-ready WAV.
pub fn clip_wav(clip: &ClipSnapshot) -> Result<Vec<u8>, EngineError> {
    let (start, duration) = clip.source_window();
    let samples = decode_mono_pcm(&clip.source_path, start, duration)?;
    if samples.is_empty() {
        return Err(EngineError::Media("clip has no audio stream".into()));
    }
    Ok(encode_wav_pcm16(&samples, ANALYSIS_SAMPLE_RATE))
}
