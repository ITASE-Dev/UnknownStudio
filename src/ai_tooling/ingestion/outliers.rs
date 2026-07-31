//! Statistical baseline: which uploads beat their channel's own average.

/// Mean after dropping the top and bottom `percentile` of the distribution, so
/// one viral hit cannot raise the bar it is meant to be measured against.
pub fn trimmed_average(values: &[u64], percentile: f64) -> f64 {
    let mut sorted: Vec<u64> = values.to_vec();
    if sorted.is_empty() {
        return 0.0;
    }
    sorted.sort_unstable();

    let cut = (sorted.len() as f64 * percentile.clamp(0.0, 0.49)) as usize;
    let window = if sorted.len() > cut * 2 {
        &sorted[cut..sorted.len() - cut]
    } else {
        &sorted[..]
    };

    window.iter().sum::<u64>() as f64 / window.len() as f64
}

/// Threshold a view count must beat to count as an outlier. Zero average means
/// no baseline, and therefore no outliers.
pub fn outlier_threshold(average: f64, multiplier: f64) -> f64 {
    if average <= 0.0 {
        f64::INFINITY
    } else {
        average * multiplier
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trimming_drops_both_tails() {
        let values: Vec<u64> = (1..=100).collect();
        // 5% off each end leaves 6..=95.
        assert_eq!(trimmed_average(&values, 0.05), 50.5);
        assert_eq!(trimmed_average(&values, 0.0), 50.5);
    }

    #[test]
    fn one_viral_hit_does_not_move_the_baseline_much() {
        let mut values: Vec<u64> = vec![100; 40];
        values.push(1_000_000);
        assert_eq!(trimmed_average(&values, 0.05), 100.0);
    }

    #[test]
    fn tiny_samples_keep_every_value() {
        assert_eq!(trimmed_average(&[10, 20, 30], 0.05), 20.0);
        assert_eq!(trimmed_average(&[], 0.05), 0.0);
    }

    #[test]
    fn no_baseline_means_no_outliers() {
        assert!(outlier_threshold(0.0, 3.0).is_infinite());
        assert_eq!(outlier_threshold(1_000.0, 3.0), 3_000.0);
    }
}
