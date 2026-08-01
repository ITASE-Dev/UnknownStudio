//! Motion analysis results and the spike detector that produces them.
//!
//! The detector is pure arithmetic over intensity samples — no OpenCV — so the
//! thresholding rules can be tested without a video file or a C++ toolchain.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// A sudden burst of motion: a hand hitting a table, a cut, a whip pan.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MotionSpike {
    pub timestamp_sec: f32,
    /// Mean absolute pixel change against the previous sampled frame, 0..=255.
    pub intensity: f32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VisualTimeline {
    pub spikes: Vec<MotionSpike>,
    /// Mean intensity across every sampled frame — the clip's baseline energy.
    pub avg_intensity: f32,
    /// Frames actually measured, after any sampling stride.
    #[serde(default)]
    pub sampled_frames: u64,
    #[serde(default)]
    pub duration_sec: f32,
}

impl VisualTimeline {
    /// Spikes per minute — a rough proxy for how frenetic the footage is.
    pub fn spike_rate_per_minute(&self) -> f32 {
        if self.duration_sec <= 0.0 {
            return 0.0;
        }
        self.spikes.len() as f32 / (self.duration_sec / 60.0)
    }

    /// The loudest moment, when there is one.
    pub fn strongest(&self) -> Option<MotionSpike> {
        self.spikes
            .iter()
            .copied()
            .max_by(|a, b| a.intensity.total_cmp(&b.intensity))
    }
}

/// Tuning for spike detection.
#[derive(Debug, Clone, Copy)]
pub struct SpikeSettings {
    /// Samples the rolling average is taken over.
    pub window: usize,
    /// A frame spikes when it exceeds `rolling_average * multiplier`.
    pub multiplier: f32,
    /// Absolute floor, so a static shot's sensor noise cannot spike.
    pub noise_floor: f32,
    /// Minimum gap between spikes; one impact must not report five times.
    pub min_gap_sec: f32,
}

impl Default for SpikeSettings {
    fn default() -> Self {
        Self {
            window: 30,
            multiplier: 2.5,
            noise_floor: 1.5,
            min_gap_sec: 0.35,
        }
    }
}

/// Folds a stream of per-frame intensities into a [`VisualTimeline`].
///
/// A frame is a spike when it stands well above the *recent* average rather
/// than the whole clip's: a busy sequence should not report every frame, and a
/// still one should still catch a single hit.
pub struct SpikeDetector {
    settings: SpikeSettings,
    recent: VecDeque<f32>,
    spikes: Vec<MotionSpike>,
    total: f64,
    count: u64,
    last_spike_sec: Option<f32>,
    last_timestamp: f32,
}

impl SpikeDetector {
    pub fn new(settings: SpikeSettings) -> Self {
        Self {
            recent: VecDeque::with_capacity(settings.window.max(1)),
            settings,
            spikes: Vec::new(),
            total: 0.0,
            count: 0,
            last_spike_sec: None,
            last_timestamp: 0.0,
        }
    }

    /// Feeds one measured frame. Returns the spike it produced, if any.
    pub fn push(&mut self, timestamp_sec: f32, intensity: f32) -> Option<MotionSpike> {
        self.total += intensity as f64;
        self.count += 1;
        self.last_timestamp = self.last_timestamp.max(timestamp_sec);

        // Compare against the window *before* this frame joins it, or a spike
        // would partly raise the bar it has to clear.
        let spike = self.is_spike(timestamp_sec, intensity).then(|| MotionSpike {
            timestamp_sec,
            intensity,
        });

        if let Some(spike) = spike {
            self.spikes.push(spike);
            self.last_spike_sec = Some(timestamp_sec);
        }

        self.recent.push_back(intensity);
        if self.recent.len() > self.settings.window.max(1) {
            self.recent.pop_front();
        }
        spike
    }

    fn is_spike(&self, timestamp_sec: f32, intensity: f32) -> bool {
        // Half a window is enough history to judge against.
        let minimum_history = (self.settings.window / 2).max(2);
        if self.recent.len() < minimum_history || intensity < self.settings.noise_floor {
            return false;
        }
        if let Some(last) = self.last_spike_sec {
            if timestamp_sec - last < self.settings.min_gap_sec {
                return false;
            }
        }

        let average = self.rolling_average();
        average > 0.0 && intensity > average * self.settings.multiplier
    }

    fn rolling_average(&self) -> f32 {
        if self.recent.is_empty() {
            return 0.0;
        }
        self.recent.iter().sum::<f32>() / self.recent.len() as f32
    }

    pub fn finish(self) -> VisualTimeline {
        let avg_intensity = if self.count == 0 {
            0.0
        } else {
            (self.total / self.count as f64) as f32
        };

        VisualTimeline {
            spikes: self.spikes,
            avg_intensity,
            sampled_frames: self.count,
            duration_sec: self.last_timestamp,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feeds `calm` quiet frames, then one loud one, at 30 fps.
    fn detect(settings: SpikeSettings, calm: usize, loud: f32) -> VisualTimeline {
        let mut detector = SpikeDetector::new(settings);
        for index in 0..calm {
            detector.push(index as f32 / 30.0, 4.0);
        }
        detector.push(calm as f32 / 30.0, loud);
        detector.finish()
    }

    #[test]
    fn a_burst_above_the_rolling_average_is_a_spike() {
        let timeline = detect(SpikeSettings::default(), 40, 60.0);

        assert_eq!(timeline.spikes.len(), 1);
        assert_eq!(timeline.spikes[0].intensity, 60.0);
        assert!((timeline.spikes[0].timestamp_sec - 40.0 / 30.0).abs() < 1e-6);
    }

    #[test]
    fn steady_motion_never_spikes() {
        let mut detector = SpikeDetector::new(SpikeSettings::default());
        // Busy but even footage: every frame is high, none stands out.
        for index in 0..120 {
            detector.push(index as f32 / 30.0, 45.0);
        }

        let timeline = detector.finish();
        assert!(timeline.spikes.is_empty());
        assert_eq!(timeline.avg_intensity, 45.0);
        assert_eq!(timeline.sampled_frames, 120);
    }

    #[test]
    fn sensor_noise_on_a_locked_off_shot_is_ignored() {
        // Near-black differences: relatively large jumps, absolutely tiny.
        let timeline = detect(SpikeSettings::default(), 40, 1.0);
        assert!(timeline.spikes.is_empty(), "below the noise floor");
    }

    #[test]
    fn one_impact_reports_once() {
        let mut detector = SpikeDetector::new(SpikeSettings::default());
        for index in 0..40 {
            detector.push(index as f32 / 30.0, 4.0);
        }
        // An impact rings out over several frames at 30 fps.
        for index in 40..46 {
            detector.push(index as f32 / 30.0, 70.0);
        }

        let timeline = detector.finish();
        assert_eq!(timeline.spikes.len(), 1, "the refractory gap holds");
    }

    #[test]
    fn a_second_impact_after_the_gap_is_reported() {
        let mut detector = SpikeDetector::new(SpikeSettings::default());
        for index in 0..40 {
            detector.push(index as f32 / 30.0, 4.0);
        }
        detector.push(40.0 / 30.0, 70.0);
        for index in 41..80 {
            detector.push(index as f32 / 30.0, 4.0);
        }
        detector.push(80.0 / 30.0, 70.0);

        let timeline = detector.finish();
        assert_eq!(timeline.spikes.len(), 2);
        assert!(timeline.strongest().expect("strongest").intensity == 70.0);
    }

    #[test]
    fn the_opening_frames_cannot_spike_without_history() {
        let mut detector = SpikeDetector::new(SpikeSettings::default());
        // The very first frame has nothing to be compared against.
        assert!(detector.push(0.0, 200.0).is_none());
        assert!(detector.push(0.03, 200.0).is_none());
    }

    #[test]
    fn settings_move_the_bar() {
        // A 1.2x multiplier catches what the default 2.5x ignores.
        let sensitive = SpikeSettings {
            multiplier: 1.2,
            ..SpikeSettings::default()
        };
        assert_eq!(detect(sensitive, 40, 8.0).spikes.len(), 1);
        assert!(detect(SpikeSettings::default(), 40, 8.0).spikes.is_empty());
    }

    #[test]
    fn an_empty_clip_reports_nothing_rather_than_dividing_by_zero() {
        let timeline = SpikeDetector::new(SpikeSettings::default()).finish();

        assert_eq!(timeline.avg_intensity, 0.0);
        assert_eq!(timeline.spike_rate_per_minute(), 0.0);
        assert!(timeline.strongest().is_none());
    }

    #[test]
    fn the_spike_rate_is_per_minute_of_footage() {
        let mut detector = SpikeDetector::new(SpikeSettings::default());
        for index in 0..40 {
            detector.push(index as f32 / 30.0, 4.0);
        }
        detector.push(40.0 / 30.0, 70.0);
        // Carry the clip out to 30 seconds of calm.
        for index in 41..900 {
            detector.push(index as f32 / 30.0, 4.0);
        }

        let timeline = detector.finish();
        assert_eq!(timeline.spikes.len(), 1);
        assert!((timeline.spike_rate_per_minute() - 2.0).abs() < 0.1);
    }
}
