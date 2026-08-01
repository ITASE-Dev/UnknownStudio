//! Level maths over normalised PCM. Pure functions, no I/O.

/// Written instead of `-inf` for a silent slice: JSON has no infinity, and a
/// finite floor keeps averages and comparisons well-behaved.
pub const FLOOR_DBFS: f32 = -100.0;

/// Root mean square of a slice. Zero for an empty one.
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_squares: f64 = samples.iter().map(|s| (*s as f64) * (*s as f64)).sum();
    (sum_squares / samples.len() as f64).sqrt() as f32
}

/// Amplitude to dBFS: `20 * log10(amplitude)`. Digital silence is `-inf`, which
/// is mathematically right and useless downstream — see [`dbfs_floored`].
pub fn dbfs(amplitude: f32) -> f32 {
    if amplitude <= 0.0 {
        return f32::NEG_INFINITY;
    }
    20.0 * amplitude.log10()
}

/// Same, clamped to [`FLOOR_DBFS`] so the value stays finite and serializable.
pub fn dbfs_floored(amplitude: f32) -> f32 {
    dbfs(amplitude).max(FLOOR_DBFS)
}

/// Mean level of a slice, floored.
pub fn mean_dbfs(samples: &[f32]) -> f32 {
    dbfs_floored(rms(samples))
}

/// Samples covering `[start_sec, end_sec)`, clamped to the buffer.
///
/// Whisper timings can run slightly past the audio on the final word, so the
/// range is clamped rather than treated as an error.
pub fn slice_for_span(samples: &[f32], sample_rate: u32, start_sec: f32, end_sec: f32) -> &[f32] {
    if samples.is_empty() || sample_rate == 0 || end_sec <= start_sec {
        return &[];
    }

    let rate = sample_rate as f32;
    let start = ((start_sec.max(0.0) * rate) as usize).min(samples.len());
    let end = ((end_sec.max(0.0) * rate).ceil() as usize).min(samples.len());

    if end <= start {
        return &[];
    }
    &samples[start..end]
}

/// Mean level across one time span of a buffer.
pub fn mean_dbfs_for_span(
    samples: &[f32],
    sample_rate: u32,
    start_sec: f32,
    end_sec: f32,
) -> f32 {
    mean_dbfs(slice_for_span(samples, sample_rate, start_sec, end_sec))
}

/// Silence before a word: the space since the previous one ended. Never
/// negative — overlapping timings mean "no gap", not "negative gap".
pub fn gap_before(previous_end_sec: Option<f32>, start_sec: f32) -> f32 {
    match previous_end_sec {
        Some(previous) => (start_sec - previous).max(0.0),
        None => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Full-scale square wave: RMS 1.0, so 0 dBFS.
    fn full_scale(len: usize) -> Vec<f32> {
        (0..len)
            .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
            .collect()
    }

    #[test]
    fn full_scale_is_zero_dbfs_and_half_scale_is_about_minus_six() {
        assert!((rms(&full_scale(64)) - 1.0).abs() < 1e-6);
        assert!(dbfs(1.0).abs() < 1e-6);

        let half: Vec<f32> = full_scale(64).iter().map(|s| s * 0.5).collect();
        assert!((dbfs(rms(&half)) + 6.02).abs() < 0.01);
    }

    #[test]
    fn digital_silence_is_negative_infinity_but_never_serialized_as_such() {
        let silence = vec![0.0f32; 100];

        assert!(dbfs(rms(&silence)).is_infinite());
        assert_eq!(mean_dbfs(&silence), FLOOR_DBFS);
        assert!(mean_dbfs(&silence).is_finite(), "JSON cannot hold -inf");
    }

    #[test]
    fn an_empty_slice_reads_as_silence_rather_than_panicking() {
        assert_eq!(rms(&[]), 0.0);
        assert_eq!(mean_dbfs(&[]), FLOOR_DBFS);
    }

    #[test]
    fn a_span_selects_the_samples_it_covers() {
        // 1 second at 10 Hz: sample n is second n/10.
        let samples: Vec<f32> = (0..10).map(|i| i as f32 / 10.0).collect();

        let middle = slice_for_span(&samples, 10, 0.3, 0.6);
        assert_eq!(middle.len(), 3);
        assert!((middle[0] - 0.3).abs() < 1e-6);
    }

    #[test]
    fn spans_past_the_end_are_clamped_not_rejected() {
        let samples = full_scale(100);

        // Whisper's last word often runs a little past the audio.
        let overrun = slice_for_span(&samples, 100, 0.5, 5.0);
        assert_eq!(overrun.len(), 50);

        // Entirely past the end: nothing, and that reads as silence.
        assert!(slice_for_span(&samples, 100, 9.0, 10.0).is_empty());
        assert_eq!(mean_dbfs_for_span(&samples, 100, 9.0, 10.0), FLOOR_DBFS);

        // Backwards or zero-length spans select nothing.
        assert!(slice_for_span(&samples, 100, 0.5, 0.5).is_empty());
        assert!(slice_for_span(&samples, 100, 0.9, 0.2).is_empty());
    }

    #[test]
    fn a_loud_word_measures_above_a_quiet_one() {
        let mut samples = vec![0.0f32; 200];
        // Second 1.0..1.5 is loud, the rest is near-silent.
        for (index, sample) in samples.iter_mut().enumerate() {
            *sample = if (100..150).contains(&index) {
                if index % 2 == 0 { 0.8 } else { -0.8 }
            } else {
                0.001
            };
        }

        let loud = mean_dbfs_for_span(&samples, 100, 1.0, 1.5);
        let quiet = mean_dbfs_for_span(&samples, 100, 0.0, 0.5);
        assert!(loud > quiet + 40.0, "loud {loud} vs quiet {quiet}");
        assert!((loud + 1.94).abs() < 0.1, "0.8 amplitude ≈ -1.94 dBFS");
    }

    #[test]
    fn the_first_word_has_no_gap_and_overlaps_do_not_go_negative() {
        assert_eq!(gap_before(None, 4.0), 0.0);
        assert!((gap_before(Some(1.0), 1.4) - 0.4).abs() < 1e-6);
        assert_eq!(gap_before(Some(2.0), 1.9), 0.0, "overlap is not a gap");
    }
}
