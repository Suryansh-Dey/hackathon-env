//! Vendor ranking (PLAN.md §12.4). Weights come from AppConfig; hard filters
//! (eligibility, hazardous authorization, suspension) are applied before scoring.

use kabados_domain::ids::UserId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct RankWeights {
    pub price: f64,
    pub coverage: f64,
    pub distance: f64,
    pub reliability: f64,
    pub rating: f64,
    pub response: f64,
    /// Bonus for vendors with few completed deals.
    pub exploration_bonus: f64,
    /// Vendors with fewer deals than this get the exploration bonus.
    pub exploration_max_deals: u32,
}

impl Default for RankWeights {
    fn default() -> Self {
        Self {
            price: 0.40,
            coverage: 0.20,
            distance: 0.10,
            reliability: 0.15,
            rating: 0.10,
            response: 0.05,
            exploration_bonus: 0.05,
            exploration_max_deals: 5,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct Candidate {
    pub vendor_id: UserId,
    /// Expected payout midpoint, paise.
    pub payout_mid: i64,
    pub coverage_permille: u32,
    pub distance_m: i64,
    pub max_radius_m: i64,
    /// Smoothed 0..1 signals.
    pub reliability: f64,
    pub rating: f64,
    pub response: f64,
    pub completed_deals: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct Ranked {
    pub vendor_id: UserId,
    pub score: f64,
    pub explored: bool,
}

/// Scores and sorts candidates (descending), ties broken by vendor ID for determinism.
pub fn rank(candidates: &[Candidate], w: &RankWeights) -> Vec<Ranked> {
    let max_payout = candidates.iter().map(|c| c.payout_mid).max().unwrap_or(0).max(1) as f64;
    let mut out: Vec<Ranked> = candidates
        .iter()
        .map(|c| {
            let dist = if c.max_radius_m > 0 { 1.0 - (c.distance_m as f64 / c.max_radius_m as f64).clamp(0.0, 1.0) } else { 0.0 };
            let explored = c.completed_deals < w.exploration_max_deals;
            let score = w.price * (c.payout_mid.max(0) as f64 / max_payout)
                + w.coverage * (c.coverage_permille as f64 / 1000.0)
                + w.distance * dist
                + w.reliability * c.reliability.clamp(0.0, 1.0)
                + w.rating * c.rating.clamp(0.0, 1.0)
                + w.response * c.response.clamp(0.0, 1.0)
                + if explored { w.exploration_bonus } else { 0.0 };
            Ranked { vendor_id: c.vendor_id.clone(), score, explored }
        })
        .collect();
    out.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.vendor_id.cmp(&b.vendor_id)));
    out
}

/// Caps the share of exploration slots in the top `k` (PLAN.md §12.4 "capped share").
pub fn top_k_with_exploration_cap(ranked: &[Ranked], k: usize, max_explored: usize) -> Vec<Ranked> {
    let mut out = Vec::with_capacity(k);
    let mut explored = 0;
    for r in ranked {
        if out.len() == k {
            break;
        }
        if r.explored {
            if explored == max_explored {
                continue;
            }
            explored += 1;
        }
        out.push(r.clone());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(payout: i64, deals: u32) -> Candidate {
        Candidate {
            vendor_id: UserId::new(),
            payout_mid: payout,
            coverage_permille: 1000,
            distance_m: 1000,
            max_radius_m: 8000,
            reliability: 0.8,
            rating: 0.8,
            response: 0.5,
            completed_deals: deals,
        }
    }

    #[test]
    fn better_price_ranks_higher() {
        let a = cand(100_000, 50);
        let b = cand(80_000, 50);
        let r = rank(&[b.clone(), a.clone()], &RankWeights::default());
        assert_eq!(r[0].vendor_id, a.vendor_id);
    }

    #[test]
    fn exploration_is_capped() {
        let cs: Vec<Candidate> = (0..6).map(|i| cand(100_000 - i, 0)).collect();
        let r = rank(&cs, &RankWeights::default());
        assert_eq!(top_k_with_exploration_cap(&r, 5, 2).len(), 2);
    }
}
