//! Offline audio analysis: decode to mono PCM, measure loudness, pack WAV.
//! Used by transcription, silence detection and waveform drawing.

use ffmpeg_next as ffmpeg;

/// 16 kHz mono — the standard input rate for speech models, and small enough
/// to upload cheaply.
pub const ANALYSIS_SAMPLE_RATE: u32 = 16_000;

/// Below this dBFS a slice counts as silence.
pub const DEFAULT_SILENCE_THRESHOLD_DB: f64 = -40.0;

/// Runs shorter than half a second are natural speech pauses, not dead air.
pub const DEFAULT_MIN_SILENCE_SECONDS: f64 = 0.5;

/// Decodes `[start, start + duration)` of a file to mono f32 PCM at
/// `ANALYSIS_SAMPLE_RATE`. One decode path for every analysis consumer.
pub fn decode_mono_pcm(
    source_path: &str,
    start_seconds: f64,
    duration_seconds: f64,
) -> Result<Vec<f32>, String> {
    ffmpeg::init().map_err(|err| err.to_string())?;

    let mut ictx = ffmpeg::format::input(source_path).map_err(|err| err.to_string())?;
    let stream = ictx
        .streams()
        .best(ffmpeg::media::Type::Audio)
        .ok_or_else(|| "source has no audio stream".to_string())?;
    let stream_index = stream.index();
    let time_base = f64::from(stream.time_base());

    let context = ffmpeg::codec::context::Context::from_parameters(stream.parameters())
        .map_err(|err| err.to_string())?;
    let mut decoder = context.decoder().audio().map_err(|err| err.to_string())?;

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

/// Deterministic silence detection over already-decoded samples — arithmetic,
/// not model guesswork.
///
/// ```text
/// samples_per_slice = sample_rate / slices_per_second
/// rms(i)            = sqrt(mean(sample^2 in window i))
/// dbfs(i)           = 20 * log10(max(rms, 1e-9))   // floor avoids -inf
/// silent(i)         = dbfs(i) < threshold_db
/// ```
/// Returns `[start, end)` slice-index runs at least `min_run` long. Slices past
/// the end of the samples count as silent — the audio simply stopped.
pub fn silent_runs(
    samples: &[f32],
    sample_rate: u32,
    slices_per_second: f64,
    slice_count: u64,
    threshold_db: f64,
    min_run: u64,
) -> Vec<(u64, u64)> {
    let samples_per_slice = (sample_rate as f64 / slices_per_second.max(1.0)).max(1.0);
    let mut runs: Vec<(u64, u64)> = Vec::new();
    let mut run_start: Option<u64> = None;

    for index in 0..slice_count {
        let start = (index as f64 * samples_per_slice) as usize;
        let end = (((index + 1) as f64) * samples_per_slice) as usize;
        let end = end.min(samples.len());

        let silent = if start >= end {
            true
        } else {
            dbfs(&samples[start..end]) < threshold_db
        };

        match (silent, run_start) {
            (true, None) => run_start = Some(index),
            (false, Some(from)) => {
                push_run(&mut runs, from, index, min_run);
                run_start = None;
            }
            _ => {}
        }
    }

    if let Some(from) = run_start {
        push_run(&mut runs, from, slice_count, min_run);
    }
    runs
}

fn dbfs(window: &[f32]) -> f64 {
    let mean_square =
        window.iter().map(|s| (*s as f64) * (*s as f64)).sum::<f64>() / window.len() as f64;
    20.0 * mean_square.sqrt().max(1e-9).log10()
}

fn push_run(runs: &mut Vec<(u64, u64)>, from: u64, to: u64, min_run: u64) {
    if to.saturating_sub(from) >= min_run {
        runs.push((from, to));
    }
}

/// Minimal RIFF/WAVE (16-bit mono PCM). Writing the 44-byte header by hand
/// avoids pulling in an FFmpeg muxer.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(len: usize, amplitude: f32) -> Vec<f32> {
        (0..len)
            .map(|i| amplitude * ((i as f32) * 0.1).sin())
            .collect()
    }

    #[test]
    fn finds_the_quiet_stretch_between_two_loud_ones() {
        // 3 slices loud, 4 silent, 3 loud at 1 slice = 100 samples.
        let mut samples = tone(300, 0.8);
        samples.extend(std::iter::repeat(0.0).take(400));
        samples.extend(tone(300, 0.8));

        let runs = silent_runs(&samples, 100, 1.0, 10, DEFAULT_SILENCE_THRESHOLD_DB, 2);
        assert_eq!(runs, vec![(3, 7)]);
    }

    #[test]
    fn ignores_runs_shorter_than_the_minimum() {
        let mut samples = tone(300, 0.8);
        samples.extend(std::iter::repeat(0.0).take(100));
        samples.extend(tone(600, 0.8));

        let runs = silent_runs(&samples, 100, 1.0, 10, DEFAULT_SILENCE_THRESHOLD_DB, 2);
        assert!(runs.is_empty());
    }

    #[test]
    fn missing_tail_counts_as_silence() {
        let samples = tone(300, 0.8);
        let runs = silent_runs(&samples, 100, 1.0, 8, DEFAULT_SILENCE_THRESHOLD_DB, 2);
        assert_eq!(runs, vec![(3, 8)]);
    }

    #[test]
    fn wav_header_describes_the_payload() {
        let wav = encode_wav_pcm16(&[0.0, 1.0, -1.0], 16_000);
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(wav.len(), 44 + 6);
        assert_eq!(u32::from_le_bytes(wav[40..44].try_into().unwrap()), 6);
    }
}
