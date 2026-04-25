//! Heuristic progress percentages for single-package and bulk upgrades.

use std::time::Instant;

/// What: Heuristic progress for single-package upgrades.
///
/// Inputs:
/// - `elapsed_ms`: Milliseconds since the upgrade started.
///
/// Output:
/// - Percentage in the range `8..=95` that monotonically rises with `elapsed_ms`.
///
/// Details:
/// - We do not receive granular progress updates from package managers, so this returns a
///   monotonic estimate that moves quickly at first and then gradually slows, capped at 95% until
///   the worker reports completion.
//
// The cast to `u16` is safe: the computed percent is bounded to 8..=95 in every branch.
#[allow(clippy::cast_possible_truncation)]
pub const fn single_upgrade_percent(elapsed_ms: u64) -> u16 {
    if elapsed_ms <= 10_000 {
        let pct = 8_u64 + ((elapsed_ms * 72_u64) / 10_000_u64);
        return pct as u16;
    }
    if elapsed_ms <= 45_000 {
        let tail_ms = elapsed_ms - 10_000_u64;
        let pct = 80_u64 + ((tail_ms * 15_u64) / 35_000_u64);
        return pct as u16;
    }
    95
}

/// What: Computes the bulk-upgrade gauge percent.
///
/// Inputs:
/// - `total`: Total number of packages scheduled for the bulk upgrade.
/// - `done`: Number of packages already finished.
/// - `current_started_at`: When the currently-running package upgrade started, if any.
///
/// Output:
/// - Percentage in `0..=100`. Done steps count fully; the in-progress step ramps up to 95%.
///
/// Details:
/// - Returns `0` when `total == 0` to avoid a divide-by-zero.
pub fn multi_upgrade_percent(
    total: usize,
    done: usize,
    current_started_at: Option<Instant>,
) -> u16 {
    if total == 0 {
        return 0;
    }
    let elapsed_ms = current_started_at.map_or(0_u128, |t| t.elapsed().as_millis());
    let sub_progress_per_mille =
        usize::try_from(((elapsed_ms * 1000) / 7000).min(950)).unwrap_or(950);
    let units_per_mille = done.saturating_mul(1000) + sub_progress_per_mille;
    let pct_usize = (units_per_mille.saturating_mul(100)) / (total.saturating_mul(1000));
    u16::try_from(pct_usize).unwrap_or(100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_upgrade_progress_is_monotonic_and_capped() {
        let checkpoints = [0_u64, 1_000, 5_000, 10_000, 20_000, 45_000, 120_000];
        let mut prev = 0_u16;
        for ms in checkpoints {
            let pct = single_upgrade_percent(ms);
            assert!(pct >= prev, "progress regressed at {ms}ms");
            prev = pct;
        }
        assert_eq!(single_upgrade_percent(0), 8);
        assert_eq!(single_upgrade_percent(10_000), 80);
        assert_eq!(single_upgrade_percent(45_000), 95);
        assert_eq!(single_upgrade_percent(120_000), 95);
    }

    #[test]
    fn multi_upgrade_progress_handles_zero_total() {
        assert_eq!(multi_upgrade_percent(0, 0, None), 0);
    }

    #[test]
    fn multi_upgrade_progress_full_when_all_done() {
        assert_eq!(multi_upgrade_percent(3, 3, None), 100);
    }
}
