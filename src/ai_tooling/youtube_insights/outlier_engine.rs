//! Robust outlier detection over view counts.
//!
//! A channel's hits are exactly the values a mean-and-standard-deviation test
//! would let distort the baseline: one 50x video drags the mean up and inflates
//! the spread, so the next 10x video scores as ordinary. Everything here is
//! built on the median instead, which those same hits cannot move.

use crate::ai_tooling::ingestion::outliers::trimmed_average;
use crate::ai_tooling::youtube_insights::models::{
    ChannelMetrics, OutlierAnalysis, OutlierMethod, ViralScore,
};

/// Iglewicz–Hoaglin: MAD scaled by this estimates the standard deviation of a
/// normal distribution, so the score reads on the familiar z scale.
const MAD_TO_SIGMA: f64 = 0.674_5;

/// Same idea for mean absolute deviation, used when MAD collapses to zero.
const MEAN_AD_TO_SIGMA: f64 = 1.253_314;

/// Iglewicz–Hoaglin's recommended cut-off for the modified z-score.
pub const DEFAULT_Z_THRESHOLD: f64 = 3.5;

/// A video must also clear this multiple of the baseline. The z-score alone
/// flags anything unusual — on a flat channel that includes a merely decent
/// upload, which is not a golden video.
pub const DEFAULT_MULTIPLIER_THRESHOLD: f64 = 2.0;

#[derive(Debug, Clone, Copy)]
pub struct OutlierSettings {
    pub z_threshold: f64,
    pub multiplier_threshold: f64,
    /// Fraction trimmed from each tail before averaging for the baseline.
    pub trim_percentile: f64,
}

impl Default for OutlierSettings {
    fn default() -> Self {
        Self {
            z_threshold: DEFAULT_Z_THRESHOLD,
            multiplier_threshold: DEFAULT_MULTIPLIER_THRESHOLD,
            trim_percentile: 0.05,
        }
    }
}

/// Median of a slice. Even counts take the midpoint of the two central values.
pub fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);

    let middle = sorted.len() / 2;
    if sorted.len() % 2 == 0 {
        (sorted[middle - 1] + sorted[middle]) / 2.0
    } else {
        sorted[middle]
    }
}

/// Median absolute deviation: the median of each value's distance from the
/// median. Unlike the standard deviation it does not grow when an outlier does.
pub fn median_absolute_deviation(values: &[f64], center: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let deviations: Vec<f64> = values.iter().map(|value| (value - center).abs()).collect();
    median(&deviations)
}

pub fn mean_absolute_deviation(values: &[f64], center: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().map(|value| (value - center).abs()).sum::<f64>() / values.len() as f64
}

/// The spread scores are divided by, and how it was obtained.
///
/// Falls back down a ladder: MAD, then mean absolute deviation for
/// distributions with a dominant repeated value, then nothing at all when every
/// upload performed identically.
pub fn robust_deviation(values: &[f64], center: f64) -> (f64, OutlierMethod) {
    let mad = median_absolute_deviation(values, center);
    if mad > 0.0 {
        return (mad / MAD_TO_SIGMA, OutlierMethod::ModifiedZScore);
    }

    let mean_ad = mean_absolute_deviation(values, center);
    if mean_ad > 0.0 {
        return (
            mean_ad * MEAN_AD_TO_SIGMA,
            OutlierMethod::MeanAbsoluteDeviation,
        );
    }

    (0.0, OutlierMethod::BaselineMultiple)
}

/// Share of `values` strictly below `value`, 0.0..=1.0.
pub fn percentile_of(values: &[f64], value: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let below = values.iter().filter(|other| **other < value).count();
    below as f64 / values.len() as f64
}

/// Scores every upload against its channel's own baseline.
pub fn analyze(metrics: &ChannelMetrics, settings: OutlierSettings) -> OutlierAnalysis {
    let views: Vec<f64> = metrics
        .videos
        .iter()
        .map(|video| video.view_count as f64)
        .collect();

    let center = median(&views);
    let baseline = {
        let counts: Vec<u64> = metrics.videos.iter().map(|v| v.view_count).collect();
        let trimmed = trimmed_average(&counts, settings.trim_percentile);
        // The trimmed mean is the friendlier baseline, but a degenerate sample
        // can leave it at zero; the median then has to carry it.
        if trimmed > 0.0 { trimmed } else { center }
    };
    let (deviation, method) = robust_deviation(&views, center);

    let mut scores: Vec<ViralScore> = metrics
        .videos
        .iter()
        .map(|video| {
            let value = video.view_count as f64;
            let modified_z = if deviation > 0.0 {
                (value - center) / deviation
            } else {
                0.0
            };
            let multiplier = if baseline > 0.0 { value / baseline } else { 0.0 };

            ViralScore {
                video_id: video.video_id.clone(),
                title: video.title.clone(),
                view_count: video.view_count,
                baseline_views: baseline,
                multiplier,
                modified_z,
                percentile: percentile_of(&views, value),
                method,
                is_outlier: is_outlier(modified_z, multiplier, method, &settings),
            }
        })
        .collect();

    // Most viral first: the caller almost always wants the top of this list.
    scores.sort_by(|a, b| b.multiplier.total_cmp(&a.multiplier));

    OutlierAnalysis {
        channel_id: metrics.channel_id.clone(),
        baseline_views: baseline,
        median_views: center,
        deviation,
        method,
        sample_size: metrics.videos.len(),
        reliable: metrics.is_reliable(),
        scores,
    }
}

/// Both tests must pass: unusual *and* substantially bigger. Either alone
/// produces false positives — a flat channel makes noise look significant, and
/// a volatile one makes a 2x upload unremarkable.
fn is_outlier(
    modified_z: f64,
    multiplier: f64,
    method: OutlierMethod,
    settings: &OutlierSettings,
) -> bool {
    if multiplier < settings.multiplier_threshold {
        return false;
    }
    match method {
        // With no spread to measure, the multiple is the only evidence there is.
        OutlierMethod::BaselineMultiple => true,
        _ => modified_z >= settings.z_threshold,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_tooling::youtube_insights::models::VideoMetrics;

    fn channel(views: &[u64]) -> ChannelMetrics {
        ChannelMetrics {
            channel_id: "UC1".into(),
            title: Some("Test".into()),
            videos: views
                .iter()
                .enumerate()
                .map(|(index, count)| VideoMetrics {
                    video_id: format!("v{index}"),
                    title: format!("Video {index}"),
                    published_at: None,
                    view_count: *count,
                    duration_seconds: 600,
                })
                .collect(),
            baseline_views: 0.0,
            median_views: 0.0,
            sampled_at: 0,
        }
    }

    /// 24 ordinary uploads around 10k, plus one 20x hit.
    fn typical() -> ChannelMetrics {
        let mut views: Vec<u64> = (0..24).map(|i| 9_000 + i * 100).collect();
        views.push(200_000);
        channel(&views)
    }

    #[test]
    fn the_median_and_mad_survive_the_outlier_they_are_measuring() {
        let views: Vec<f64> = typical()
            .videos
            .iter()
            .map(|v| v.view_count as f64)
            .collect();

        let center = median(&views);
        // A 200k hit among 10k uploads must not move the centre.
        assert!((9_000.0..=11_000.0).contains(&center), "median {center}");

        let (deviation, method) = robust_deviation(&views, center);
        assert_eq!(method, OutlierMethod::ModifiedZScore);
        // The mean would have been dragged past 17k by that single video.
        let mean = views.iter().sum::<f64>() / views.len() as f64;
        assert!(mean > center * 1.5, "the mean is the fragile statistic");
        assert!(deviation > 0.0);
    }

    #[test]
    fn the_hit_is_flagged_and_the_ordinary_uploads_are_not() {
        let analysis = analyze(&typical(), OutlierSettings::default());

        let golden: Vec<&ViralScore> = analysis.golden().collect();
        assert_eq!(golden.len(), 1, "exactly one golden video");
        assert_eq!(golden[0].view_count, 200_000);
        assert!(golden[0].multiplier > 15.0);
        assert!(golden[0].modified_z > DEFAULT_Z_THRESHOLD);
        assert!(golden[0].percentile > 0.9);
        assert!(analysis.reliable, "25 uploads is a usable sample");
    }

    #[test]
    fn scores_come_back_most_viral_first() {
        let analysis = analyze(&channel(&[1_000, 50_000, 5_000, 2_000]), OutlierSettings::default());

        let order: Vec<u64> = analysis.scores.iter().map(|s| s.view_count).collect();
        assert_eq!(order, vec![50_000, 5_000, 2_000, 1_000]);
        assert_eq!(analysis.best().expect("best").view_count, 50_000);
    }

    #[test]
    fn a_flat_channel_reports_no_outliers() {
        // Every upload identical: no spread, and nothing beats the baseline.
        let analysis = analyze(&channel(&[10_000; 25]), OutlierSettings::default());

        assert_eq!(analysis.method, OutlierMethod::BaselineMultiple);
        assert_eq!(analysis.deviation, 0.0);
        assert_eq!(analysis.golden().count(), 0, "a 1.0x video is not golden");
    }

    #[test]
    fn a_flat_channel_with_one_hit_still_finds_it() {
        let mut views = vec![10_000u64; 24];
        views.push(90_000);
        let analysis = analyze(&channel(&views), OutlierSettings::default());

        // MAD is zero here, so the fallback ladder has to carry the decision.
        assert_ne!(analysis.method, OutlierMethod::ModifiedZScore);
        let golden: Vec<&ViralScore> = analysis.golden().collect();
        assert_eq!(golden.len(), 1);
        assert_eq!(golden[0].view_count, 90_000);
    }

    #[test]
    fn a_merely_good_video_is_not_golden() {
        // 1.5x the baseline on a volatile channel: unusual, but not a hit.
        let mut views: Vec<u64> = (0..24).map(|i| 5_000 + i * 500).collect();
        views.push(15_000);
        let analysis = analyze(&channel(&views), OutlierSettings::default());

        assert!(
            analysis.golden().all(|score| score.multiplier >= DEFAULT_MULTIPLIER_THRESHOLD),
            "the multiplier gate holds"
        );
    }

    #[test]
    fn a_short_sample_is_scored_but_marked_unreliable() {
        let analysis = analyze(&channel(&[1_000, 2_000, 90_000]), OutlierSettings::default());

        assert!(!analysis.reliable, "3 uploads prove nothing");
        assert_eq!(analysis.scores.len(), 3, "still scored, for display");
    }

    #[test]
    fn an_empty_channel_does_not_divide_by_zero() {
        let analysis = analyze(&channel(&[]), OutlierSettings::default());

        assert_eq!(analysis.baseline_views, 0.0);
        assert_eq!(analysis.deviation, 0.0);
        assert!(analysis.scores.is_empty());
        assert!(analysis.best().is_none());
    }

    #[test]
    fn percentiles_measure_against_the_whole_sample() {
        let values = vec![1.0, 2.0, 3.0, 4.0];
        assert_eq!(percentile_of(&values, 4.0), 0.75);
        assert_eq!(percentile_of(&values, 1.0), 0.0);
        assert_eq!(percentile_of(&[], 1.0), 0.0);
    }

    #[test]
    fn medians_handle_even_and_odd_counts() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&[4.0, 1.0, 3.0, 2.0]), 2.5);
        assert_eq!(median(&[]), 0.0);
    }
}
