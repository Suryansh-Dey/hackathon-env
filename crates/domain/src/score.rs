//! Bayesian-smoothed scores (PLAN.md §12.4, §14).

use crate::user::ScoreStats;

/// Prior: `PRIOR_WEIGHT` virtual observations at `PRIOR_MEAN` per-mille.
pub const PRIOR_WEIGHT: u64 = 5;
pub const PRIOR_MEAN_PERMILLE: u64 = 800;

impl ScoreStats {
    pub fn record(&mut self, observation_permille: u32) {
        self.count = self.count.saturating_add(1);
        self.sum_permille = self.sum_permille.saturating_add(observation_permille.min(1000) as u64);
    }

    /// Smoothed score in `0.0..=1.0`; new users start near the prior.
    pub fn smoothed(&self) -> f64 {
        let num = self.sum_permille + PRIOR_WEIGHT * PRIOR_MEAN_PERMILLE;
        let den = (self.count as u64 + PRIOR_WEIGHT) * 1000;
        num as f64 / den as f64
    }

    /// Stars (1–5) mapped to per-mille.
    pub fn record_stars(&mut self, stars: u8) {
        let s = stars.clamp(1, 5) as u32;
        self.record((s - 1) * 250);
    }

    /// Display rating out of 5.
    pub fn stars(&self) -> f64 {
        1.0 + self.smoothed() * 4.0
    }
}

/// Seller honesty observation from estimate vs. weighed weight.
pub fn weight_accuracy_permille(estimate_g: i64, weighed_g: i64) -> u32 {
    if weighed_g <= 0 {
        return 0;
    }
    let err = (estimate_g - weighed_g).abs() as f64 / weighed_g as f64;
    ((1.0 - err.min(1.0)) * 1000.0).round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_users_are_not_penalised() {
        let s = ScoreStats::default();
        assert!((s.smoothed() - 0.8).abs() < 1e-9);
        let mut bad = ScoreStats::default();
        bad.record(0);
        assert!(bad.smoothed() > 0.6, "one bad observation should not sink a new user");
    }

    #[test]
    fn accuracy() {
        assert_eq!(weight_accuracy_permille(100, 100), 1000);
        assert_eq!(weight_accuracy_permille(90, 100), 900);
        assert_eq!(weight_accuracy_permille(500, 100), 0);
    }
}
