//! Cost arithmetic for `bosun progress` (spec §9): the headline metric is cost per merged task.
//! Aggregation of values the command already fetched; no I/O.

use crate::time::Timestamp;

/// Start of a window of `days` days that ends at `now`.
pub fn window_start(now: Timestamp, days: u32) -> Timestamp {
    now.minus_secs(i64::from(days) * 86_400)
}

/// True when `at` lies in `[since, now]`.
pub fn in_window(at: Timestamp, since: Timestamp, now: Timestamp) -> bool {
    at >= since && at <= now
}

/// Round dollars to 6 decimals so sums of cents do not print as `0.30000000000000004`.
pub fn round_usd(usd: f64) -> f64 {
    (usd * 1e6).round() / 1e6
}

/// `total / count`, `None` when `count` is 0.
pub fn per_task(total: f64, count: usize) -> Option<f64> {
    (count > 0).then(|| round_usd(total / count as f64))
}

/// Mean of the task costs, `None` when there are none.
pub fn mean(costs: &[f64]) -> Option<f64> {
    per_task(costs.iter().sum(), costs.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_bounds_are_inclusive() {
        let now = Timestamp(1_000_000);
        let since = window_start(now, 7);
        assert_eq!(since, Timestamp(1_000_000 - 7 * 86_400));
        assert!(in_window(since, since, now) && in_window(now, since, now));
        assert!(!in_window(since.minus_secs(1), since, now));
        assert!(!in_window(now.plus_secs(1), since, now));
    }

    #[test]
    fn per_task_and_mean_are_none_without_tasks() {
        assert_eq!(per_task(5.0, 0), None);
        assert_eq!(mean(&[]), None);
        assert_eq!(per_task(9.0, 2), Some(4.5));
        assert_eq!(mean(&[1.0, 2.0, 3.0]), Some(2.0));
    }

    #[test]
    fn sums_of_cents_are_rounded() {
        assert_eq!(mean(&[0.1, 0.2]), Some(0.15));
        assert_eq!(round_usd(0.1 + 0.2), 0.3);
    }
}
