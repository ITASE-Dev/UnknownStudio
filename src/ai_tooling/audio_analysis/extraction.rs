//! FFmpeg extraction and PCM reading.
//!
//! Whisper wants 16 kHz mono f32; the energy maths wants the same buffer, so
//! the file is decoded once and both read from it.

use crate::ai_tooling::audio_analysis::AudioAnalysisError;
use std::path::{Path, PathBuf};
use tokio::process::Command;

/// Whisper's native rate. Resampling anywhere else would cost accuracy for
/// nothing.
pub const TARGET_SAMPLE_RATE: u32 = 16_000;
pub const TARGET_CHANNELS: u16 = 1;

/// Canonical 44-byte RIFF header length for PCM.
const WAV_HEADER_LEN: usize = 44;

/// Decoded audio plus the shape it was decoded to.
#[derive(Debug, Clone)]
pub struct PcmBuffer {
    /// Normalised to -1.0..=1.0.
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
}

impl PcmBuffer {
    pub fn duration_sec(&self) -> f32 {
        if self.sample_rate == 0 {
            return 0.0;
        }
        self.samples.len() as f32 / self.sample_rate as f32
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
}

/// Extracts the audio to a temp WAV and reads it back as normalised samples.
/// The temp file is removed before returning.
pub async fn extract_pcm(media_path: &Path) -> Result<PcmBuffer, AudioAnalysisError> {
    if !tokio::fs::try_exists(media_path).await.unwrap_or(false) {
        return Err(AudioAnalysisError::MediaNotFound(media_path.to_path_buf()));
    }

    let wav_path = temp_wav_path();
    extract_wav(media_path, &wav_path).await?;

    let bytes = tokio::fs::read(&wav_path).await?;
    // Best effort: a leftover temp file must not fail an otherwise good run.
    let _ = tokio::fs::remove_file(&wav_path).await;

    Ok(PcmBuffer {
        samples: decode_s16le(&bytes),
        sample_rate: TARGET_SAMPLE_RATE,
        channels: TARGET_CHANNELS,
    })
}

/// Runs FFmpeg. stdout/stderr are captured, not inherited, so nothing reaches
/// the console; the tail of stderr is kept for the error message.
pub async fn extract_wav(media_path: &Path, wav_path: &Path) -> Result<(), AudioAnalysisError> {
    let output = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-y",
            "-i",
            &media_path.to_string_lossy(),
            "-vn",
            "-ac",
            &TARGET_CHANNELS.to_string(),
            "-ar",
            &TARGET_SAMPLE_RATE.to_string(),
            "-acodec",
            "pcm_s16le",
            "-f",
            "wav",
            &wav_path.to_string_lossy(),
        ])
        .output()
        .await
        .map_err(|err| AudioAnalysisError::Ffmpeg(format!("could not run ffmpeg: {err}")))?;

    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        let reason = detail.lines().last().unwrap_or("extraction failed").trim();
        return Err(AudioAnalysisError::Ffmpeg(reason.to_string()));
    }
    Ok(())
}

/// Signed 16-bit little-endian PCM to normalised f32, skipping the RIFF header.
///
/// Division by 32768 (not 32767) keeps the mapping symmetric: full-scale
/// negative becomes exactly -1.0 and nothing can exceed the range.
pub fn decode_s16le(bytes: &[u8]) -> Vec<f32> {
    let body = if bytes.starts_with(b"RIFF") && bytes.len() > WAV_HEADER_LEN {
        &bytes[data_offset(bytes)..]
    } else {
        bytes
    };

    body.chunks_exact(2)
        .map(|pair| i16::from_le_bytes([pair[0], pair[1]]) as f32 / 32_768.0)
        .collect()
}

/// Offset of the `data` chunk payload. FFmpeg writes the canonical 44-byte
/// header, but a `LIST` chunk before `data` would shift it.
fn data_offset(bytes: &[u8]) -> usize {
    let mut cursor = 12; // past "RIFF"<size>"WAVE"

    while cursor + 8 <= bytes.len() {
        let id = &bytes[cursor..cursor + 4];
        let size = u32::from_le_bytes([
            bytes[cursor + 4],
            bytes[cursor + 5],
            bytes[cursor + 6],
            bytes[cursor + 7],
        ]) as usize;

        if id == b"data" {
            return cursor + 8;
        }
        // Chunks are word-aligned.
        cursor += 8 + size + (size & 1);
    }
    WAV_HEADER_LEN.min(bytes.len())
}

fn temp_wav_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "unknown_studio_audio_{}.wav",
        uuid::Uuid::new_v4().simple()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Canonical 44-byte header followed by `samples`.
    fn wav(samples: &[i16]) -> Vec<u8> {
        let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut bytes = Vec::with_capacity(WAV_HEADER_LEN + data.len());

        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&((36 + data.len()) as u32).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
        bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
        bytes.extend_from_slice(&TARGET_SAMPLE_RATE.to_le_bytes());
        bytes.extend_from_slice(&(TARGET_SAMPLE_RATE * 2).to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&data);
        bytes
    }

    #[test]
    fn samples_are_normalised_symmetrically() {
        let decoded = decode_s16le(&wav(&[0, i16::MAX, i16::MIN, -16_384]));

        assert_eq!(decoded.len(), 4);
        assert_eq!(decoded[0], 0.0);
        assert!((decoded[1] - 1.0).abs() < 1e-4);
        assert_eq!(decoded[2], -1.0, "full-scale negative is exactly -1.0");
        assert_eq!(decoded[3], -0.5);
        assert!(decoded.iter().all(|s| (-1.0..=1.0).contains(s)));
    }

    #[test]
    fn the_riff_header_is_skipped_not_decoded_as_audio() {
        let decoded = decode_s16le(&wav(&[1000; 8]));

        assert_eq!(decoded.len(), 8, "header bytes are not samples");
        assert!(decoded.iter().all(|s| (*s - 1000.0 / 32_768.0).abs() < 1e-6));
    }

    #[test]
    fn a_list_chunk_before_data_does_not_shift_the_samples() {
        // FFmpeg sometimes writes a LIST/INFO chunk between fmt and data.
        let mut bytes = wav(&[]);
        let mut with_list = bytes.drain(..36).collect::<Vec<u8>>();
        with_list.extend_from_slice(b"LIST");
        with_list.extend_from_slice(&4u32.to_le_bytes());
        with_list.extend_from_slice(b"INFO");
        with_list.extend_from_slice(b"data");
        with_list.extend_from_slice(&4u32.to_le_bytes());
        with_list.extend_from_slice(&i16::MAX.to_le_bytes());
        with_list.extend_from_slice(&i16::MIN.to_le_bytes());

        let decoded = decode_s16le(&with_list);
        assert_eq!(decoded.len(), 2);
        assert!((decoded[0] - 1.0).abs() < 1e-4);
        assert_eq!(decoded[1], -1.0);
    }

    #[test]
    fn raw_pcm_without_a_header_decodes_too() {
        let raw: Vec<u8> = [0i16, 16_384].iter().flat_map(|s| s.to_le_bytes()).collect();
        assert_eq!(decode_s16le(&raw), vec![0.0, 0.5]);

        // A trailing odd byte is dropped rather than misread.
        assert_eq!(decode_s16le(&[0x00, 0x40, 0x00]).len(), 1);
    }

    #[test]
    fn duration_follows_the_sample_count() {
        let buffer = PcmBuffer {
            samples: vec![0.0; TARGET_SAMPLE_RATE as usize * 3],
            sample_rate: TARGET_SAMPLE_RATE,
            channels: TARGET_CHANNELS,
        };
        assert_eq!(buffer.duration_sec(), 3.0);
        assert!(!buffer.is_empty());
    }

    #[tokio::test]
    async fn a_missing_file_is_reported_before_ffmpeg_runs() {
        let result = extract_pcm(Path::new("no-such-media.mp4")).await;
        assert!(matches!(result, Err(AudioAnalysisError::MediaNotFound(_))));
    }
}
