//! Peak extraction for timeline waveforms, off the UI thread.

use crate::audio_engine::analysis::{decode_mono_pcm, ANALYSIS_SAMPLE_RATE};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;

/// Peaks per second of source. Enough detail for a timeline lane without
/// storing anything close to the sample data.
pub const PEAKS_PER_SECOND: f64 = 40.0;

/// Cap per file, so a two-hour podcast can't balloon the cache.
const MAX_PEAKS: usize = 40_000;

/// Analysed at most this many seconds of a file.
const MAX_ANALYSIS_SECONDS: f64 = MAX_PEAKS as f64 / PEAKS_PER_SECOND;

pub struct Waveform {
    pub path: PathBuf,
    /// Absolute peak per bucket, 0.0..=1.0.
    pub peaks: Arc<Vec<f32>>,
}

/// Request/poll pair mirroring the video preview engine.
pub struct WaveformService {
    requests: Sender<PathBuf>,
    ready: Receiver<Waveform>,
}

impl Default for WaveformService {
    fn default() -> Self {
        Self::new()
    }
}

impl WaveformService {
    pub fn new() -> Self {
        let (requests, request_rx) = mpsc::channel::<PathBuf>();
        let (ready_tx, ready) = mpsc::channel::<Waveform>();

        thread::spawn(move || {
            while let Ok(path) = request_rx.recv() {
                let Ok(samples) = decode_mono_pcm(&path.to_string_lossy(), 0.0, MAX_ANALYSIS_SECONDS)
                else {
                    continue;
                };
                if samples.is_empty() {
                    continue;
                }
                let peaks = Arc::new(peaks_from_samples(&samples, ANALYSIS_SAMPLE_RATE));
                if ready_tx.send(Waveform { path, peaks }).is_err() {
                    return;
                }
            }
        });

        Self { requests, ready }
    }

    pub fn request(&self, path: PathBuf) {
        let _ = self.requests.send(path);
    }

    pub fn poll(&self) -> impl Iterator<Item = Waveform> + '_ {
        std::iter::from_fn(move || self.ready.try_recv().ok())
    }
}

/// Absolute peak per bucket — peaks, not averages, so transients stay visible.
pub fn peaks_from_samples(samples: &[f32], sample_rate: u32) -> Vec<f32> {
    let per_bucket = (sample_rate as f64 / PEAKS_PER_SECOND).max(1.0) as usize;
    samples
        .chunks(per_bucket)
        .take(MAX_PEAKS)
        .map(|chunk| {
            chunk
                .iter()
                .fold(0.0f32, |peak, s| peak.max(s.abs()))
                .min(1.0)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_hold_the_loudest_sample() {
        let samples = [0.1, 0.9, 0.2, 0.0, 0.3, 0.4];
        // 3 samples per bucket at this rate.
        let peaks = peaks_from_samples(&samples, (PEAKS_PER_SECOND * 3.0) as u32);
        assert_eq!(peaks, vec![0.9, 0.4]);
    }

    #[test]
    fn peaks_are_absolute_and_clamped() {
        let peaks = peaks_from_samples(&[-0.8, 0.2, -2.0], (PEAKS_PER_SECOND * 3.0) as u32);
        assert_eq!(peaks, vec![1.0]);
    }
}
