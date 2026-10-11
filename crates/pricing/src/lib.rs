//! `kabados-pricing`: the single implementation of quoting (PLAN.md §12.3).
//!
//! The same [`quote`] function computes listing suggestions, offer pre-fill,
//! re-grades at pickup and final settlement (weighed weight used as both low
//! and high), so what a seller is shown, what a vendor offers and what is paid
//! can never drift apart. Pure and deterministic: no I/O, no clocks, no floats
//! in money paths.

pub mod rank;
pub mod stats;
pub mod suggest;

use chrono::{DateTime, Utc};
use kabados_domain::geo::LatLng;
use kabados_domain::ids::ItemId;
use kabados_domain::ladder::LadderLevel;
use kabados_domain::listing::{AttrValue, PlausibleWeight, Weekday, WeightSource};
use kabados_domain::money::div_round;
use kabados_domain::offer::{LotAdjustment, Rate};
use kabados_domain::ruleset::{Modifier, RuleLine, RuleSet, RuleSetStatus};
use kabados_domain::taxonomy::{Taxonomy, Unit, glob_matches};
use kabados_domain::{Grams, Paise};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Bumped whenever quoting semantics change; stored on every quote, offer and deal.
pub const ENGINE_VERSION: &str = "pricing-1.0.0";

/// Road distance ≈ great-circle × this factor when no routed distance is cached.
pub const ROAD_FACTOR_PERMILLE: i64 = 1300;

/// One item of a listing as seen by the engine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct QuoteItem {
    pub item_id: ItemId,
    pub category_id: String,
    pub grade: Option<String>,
    #[serde(default)]
    pub attributes: BTreeMap<String, AttrValue>,
    pub quantity: u32,
    /// Seller's estimate for the whole line.
    pub est_weight: Grams,
    /// Plausible range: per piece for PIECE categories, whole line for KG.
    pub plausible: Option<PlausibleWeight>,
    pub ladder: LadderLevel,
    pub hazardous: bool,
    /// Scale reading (settlement and re-grades).
    pub weighed: Option<Grams>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct PickupContext {
    /// Point used for service-area checks (the listing's cell centre).
    pub location: LatLng,
    pub floor: i32,
    pub lift_available: bool,
    /// Seller's preferred days; empty = any.
    pub days: Vec<Weekday>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct VendorContext {
    pub active: bool,
    pub hazardous_authorized: bool,
    pub yard: LatLng,
    /// Remaining daily capacity in grams, if the vendor set one.
    pub capacity_left: Option<Grams>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum QuoteMode {
    /// Pre-deal quote from seller estimates.
    Estimate,
    /// Final amount from scale readings; agreed eligibility is not re-checked.
    Settlement,
}

pub struct QuoteRequest<'a> {
    pub items: &'a [QuoteItem],
    pub pickup: &'a PickupContext,
    pub vendor: &'a VendorContext,
    pub rules: &'a RuleSet,
    pub taxonomy: &'a Taxonomy,
    /// Routed distance yard → pickup (Amazon Location Routes, cached per cell pair).
    pub road_distance_m: Option<i64>,
    /// Agreed per-item rates (offer edits); take precedence over rule lines.
    pub overrides: &'a BTreeMap<ItemId, Rate>,
    pub mode: QuoteMode,
    pub now: DateTime<Utc>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Code {
    VendorInactive,
    RulesNotPublished,
    RulesNotEffectiveYet,
    OutsideServiceArea,
    ScheduleMismatch,
    CapacityExceeded,
    HazardousNotAuthorized,
    NoCoveredItems,
    PartialLotNotAccepted,
    BelowMinOrderValue,
    MayFallBelowMinOrderValue,
    BelowMinWeight,
    ExcludedCategory,
    NoRuleForItem,
    LineBelowMinWeight,
    AgreedRateApplied,
    VerifiedBonusApplied,
    UnverifiedDiscountApplied,
    BulkTierApplied,
    DistanceDeductionApplied,
    NoLiftSurchargeApplied,
    ClampedToZero,
    WeightClampedToReference,
    NotWeighed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct QuoteLine {
    pub item_id: ItemId,
    pub category_id: String,
    pub grade: Option<String>,
    pub rate: Rate,
    pub quantity: u32,
    pub weight_low: Grams,
    pub weight_high: Grams,
    pub base_low: Paise,
    pub base_high: Paise,
    /// Net verification modifier (+bonus / −discount), percent.
    pub modifier_pct: i64,
    pub value_low: Paise,
    pub value_high: Paise,
    /// Index of the matched rule line; `None` when an agreed rate was used.
    pub rule_line: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct ItemNote {
    pub item_id: ItemId,
    pub code: Code,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct Quote {
    pub eligible: bool,
    pub covered_item_ids: Vec<ItemId>,
    pub uncovered_item_ids: Vec<ItemId>,
    /// Covered items / all items, in per-mille (integer for determinism).
    pub coverage_permille: u32,
    pub quote_low: Paise,
    pub quote_high: Paise,
    pub covered_weight_low: Grams,
    pub covered_weight_high: Grams,
    pub lines: Vec<QuoteLine>,
    pub adjustments: Vec<LotAdjustment>,
    pub explanation_codes: Vec<Code>,
    pub item_notes: Vec<ItemNote>,
    pub distance_m: i64,
    pub engine_version: String,
    pub rule_set_version: u32,
}

impl Quote {
    pub fn coverage_ratio(&self) -> f64 {
        self.coverage_permille as f64 / 1000.0
    }

    pub fn mid(&self) -> Paise {
        Paise((self.quote_low.0 + self.quote_high.0) / 2)
    }
}

/// Value of one line: `rate × weight` (KG) or `price × qty` (PIECE).
pub fn line_value(rate: Rate, quantity: u32, weight: Grams) -> Paise {
    match rate {
        Rate::Kg { paise_per_kg } => Paise::per_kg(paise_per_kg, weight).unwrap_or(Paise(i64::MAX / 4)),
        Rate::Piece { paise_per_piece } => Paise::per_piece(paise_per_piece, quantity).unwrap_or(Paise(i64::MAX / 4)),
    }
}

/// `base` adjusted by `pct` percent, rounded half away from zero.
pub fn apply_pct(base: Paise, pct: i64) -> Paise {
    base + base.pct(pct)
}

fn rate_of(line: &RuleLine, unit: Unit) -> Option<Rate> {
    match unit {
        Unit::Kg => line.price_per_kg_paise.map(|p| Rate::Kg { paise_per_kg: Paise(p) }),
        Unit::Piece => line.price_per_piece_paise.map(|p| Rate::Piece { paise_per_piece: Paise(p) }),
    }
}

/// Most specific applicable rule line; ties broken by line order (PLAN.md §12.3 step 2).
pub fn match_line<'r>(rules: &'r RuleSet, tax: &Taxonomy, item: &QuoteItem, point_weight: Grams) -> Result<(usize, &'r RuleLine), Code> {
    if rules.is_excluded(&item.category_id) {
        return Err(Code::ExcludedCategory);
    }
    let mut below_min = false;
    let mut best: Option<(usize, &RuleLine)> = None;
    for (i, l) in rules.lines.iter().enumerate() {
        if l.category_id != item.category_id {
            continue;
        }
        if let Some(g) = &l.grade
            && item.grade.as_ref() != Some(g)
        {
            continue;
        }
        if !l.conditions.iter().all(|(k, v)| item.attributes.get(k) == Some(v)) {
            continue;
        }
        if l.effective_unit(tax).and_then(|u| rate_of(l, u)).is_none() {
            continue;
        }
        if let Some(min) = l.min_weight_g
            && point_weight.0 < min
        {
            below_min = true;
            continue;
        }
        if best.is_none_or(|(_, b)| l.specificity() > b.specificity()) {
            best = Some((i, l));
        }
    }
    best.ok_or(if below_min { Code::LineBelowMinWeight } else { Code::NoRuleForItem })
}

/// Weight range for an item (PLAN.md §12.3 step 3): estimate ± tolerance,
/// clamped to the reference-table range when there is one.
pub fn weight_range(item: &QuoteItem, tax: &Taxonomy, mode: QuoteMode) -> Option<(Grams, Grams, bool)> {
    if mode == QuoteMode::Settlement {
        return item.weighed.map(|w| (w, w, false));
    }
    if let Some(w) = item.weighed {
        return Some((w, w, false));
    }
    let cat = tax.get(&item.category_id)?;
    let tol = cat.weight_tolerance_pct as i64;
    let est = item.est_weight.0.max(0);
    let (mut lo, mut hi) = (Grams(est).pct(100 - tol), Grams(est).pct(100 + tol));
    let mut clamped = false;
    if let Some(PlausibleWeight { range, source: WeightSource::ReferenceTable }) = item.plausible {
        let (mut pmin, mut pmax) = (range.min, range.max);
        if cat.unit == Unit::Piece {
            let q = item.quantity.max(1) as i64;
            pmin = Grams(pmin.0 * q);
            pmax = Grams(pmax.0 * q);
        }
        let (nlo, nhi) = (lo.clamp(pmin, pmax), hi.clamp(pmin, pmax));
        clamped = nlo != lo || nhi != hi;
        lo = nlo;
        hi = nhi;
    }
    Some((lo, hi, clamped))
}

fn verification_pct(rules: &RuleSet, ladder: LadderLevel) -> (i64, Vec<Code>) {
    let mut bonus = 0i64;
    let mut discount = 0i64;
    for m in &rules.modifiers {
        match m {
            Modifier::VerifiedBonusPct { min_ladder, value } if ladder >= *min_ladder => bonus = bonus.max(*value as i64),
            Modifier::UnverifiedDiscountPct { max_ladder, value } if ladder <= *max_ladder => discount = discount.max(*value as i64),
            _ => {}
        }
    }
    let mut codes = Vec::new();
    if bonus > 0 {
        codes.push(Code::VerifiedBonusApplied);
    }
    if discount > 0 {
        codes.push(Code::UnverifiedDiscountApplied);
    }
    (bonus - discount, codes)
}

fn distance_m(req: &QuoteRequest<'_>) -> i64 {
    req.road_distance_m.unwrap_or_else(|| {
        let crow = req.vendor.yard.distance_m(&req.pickup.location).round() as i64;
        div_round(crow as i128 * ROAD_FACTOR_PERMILLE as i128, 1000) as i64
    })
}

/// Computes a quote. Never panics; ineligibility is reported, not raised.
pub fn quote(req: &QuoteRequest<'_>) -> Quote {
    let rules = req.rules;
    let tax = req.taxonomy;
    let estimate = req.mode == QuoteMode::Estimate;
    let mut codes: Vec<Code> = Vec::new();
    let mut ineligible: Vec<Code> = Vec::new();

    // 1. Eligibility (agreed deals are not re-checked at settlement).
    if estimate {
        if !req.vendor.active {
            ineligible.push(Code::VendorInactive);
        }
        if rules.status != RuleSetStatus::Published {
            ineligible.push(Code::RulesNotPublished);
        }
        if req.now < rules.effective_from.with_timezone(&Utc) {
            ineligible.push(Code::RulesNotEffectiveYet);
        }
        if !rules.service_area.contains(&req.pickup.location) {
            ineligible.push(Code::OutsideServiceArea);
        }
        if !req.pickup.days.is_empty() && !req.pickup.days.iter().any(|d| rules.schedule.days.contains(d)) {
            ineligible.push(Code::ScheduleMismatch);
        }
        if req.items.iter().any(|i| i.hazardous) && !req.vendor.hazardous_authorized {
            ineligible.push(Code::HazardousNotAuthorized);
        }
    }

    // 2–5. Line matching, weight ranges, line values, verification modifiers.
    let mut lines: Vec<QuoteLine> = Vec::new();
    let mut covered = Vec::new();
    let mut uncovered = Vec::new();
    let mut notes = Vec::new();
    for item in req.items {
        let Some((w_lo, w_hi, clamped)) = weight_range(item, tax, req.mode) else {
            uncovered.push(item.item_id.clone());
            let code = if tax.get(&item.category_id).is_none() { Code::NoRuleForItem } else { Code::NotWeighed };
            notes.push(ItemNote { item_id: item.item_id.clone(), code });
            continue;
        };
        if clamped {
            notes.push(ItemNote { item_id: item.item_id.clone(), code: Code::WeightClampedToReference });
        }
        let point = item.weighed.unwrap_or(item.est_weight);
        let (rate, rule_line) = match req.overrides.get(&item.item_id) {
            Some(r) => {
                notes.push(ItemNote { item_id: item.item_id.clone(), code: Code::AgreedRateApplied });
                (*r, None)
            }
            None => match match_line(rules, tax, item, point) {
                Ok((i, l)) => {
                    let unit = l.effective_unit(tax).unwrap_or(Unit::Kg);
                    match rate_of(l, unit) {
                        Some(r) => (r, Some(i)),
                        None => {
                            uncovered.push(item.item_id.clone());
                            notes.push(ItemNote { item_id: item.item_id.clone(), code: Code::NoRuleForItem });
                            continue;
                        }
                    }
                }
                Err(code) => {
                    uncovered.push(item.item_id.clone());
                    notes.push(ItemNote { item_id: item.item_id.clone(), code });
                    continue;
                }
            },
        };
        let base_low = line_value(rate, item.quantity, w_lo);
        let base_high = line_value(rate, item.quantity, w_hi);
        let (pct, vcodes) = verification_pct(rules, item.ladder);
        for c in vcodes {
            if !codes.contains(&c) {
                codes.push(c);
            }
        }
        lines.push(QuoteLine {
            item_id: item.item_id.clone(),
            category_id: item.category_id.clone(),
            grade: item.grade.clone(),
            rate,
            quantity: item.quantity,
            weight_low: w_lo,
            weight_high: w_hi,
            base_low,
            base_high,
            modifier_pct: pct,
            value_low: apply_pct(base_low, pct),
            value_high: apply_pct(base_high, pct),
            rule_line,
        });
        covered.push(item.item_id.clone());
    }

    if lines.is_empty() {
        ineligible.push(Code::NoCoveredItems);
    } else if !uncovered.is_empty() && !rules.accept_partial_lots && estimate {
        ineligible.push(Code::PartialLotNotAccepted);
    }

    let covered_weight_low: Grams = lines.iter().map(|l| l.weight_low).sum();
    let covered_weight_high: Grams = lines.iter().map(|l| l.weight_high).sum();
    if estimate
        && let Some(cap) = req.vendor.capacity_left
        && covered_weight_low > cap
    {
        ineligible.push(Code::CapacityExceeded);
    }

    // 6. Lot modifiers.
    let mut adjustments = Vec::new();
    let sum_low: Paise = lines.iter().map(|l| l.value_low).sum();
    let sum_high: Paise = lines.iter().map(|l| l.value_high).sum();
    let mut total_low = sum_low;
    let mut total_high = sum_high;

    // Bulk tiers: best qualifying tier per glob.
    let mut globs: Vec<&str> = rules
        .modifiers
        .iter()
        .filter_map(|m| match m {
            Modifier::BulkTier { category_glob, .. } => Some(category_glob.as_str()),
            _ => None,
        })
        .collect();
    globs.sort_unstable();
    globs.dedup();
    for glob in globs {
        let matching: Vec<&QuoteLine> = lines.iter().filter(|l| glob_matches(glob, &l.category_id)).collect();
        let w_lo: i64 = matching.iter().map(|l| l.weight_low.0).sum();
        let w_hi: i64 = matching.iter().map(|l| l.weight_high.0).sum();
        let v_lo: Paise = matching.iter().map(|l| l.value_low).sum();
        let v_hi: Paise = matching.iter().map(|l| l.value_high).sum();
        let pct_for = |w: i64| -> i64 {
            rules
                .modifiers
                .iter()
                .filter_map(|m| match m {
                    Modifier::BulkTier { category_glob, threshold_g, bonus_pct } if category_glob == glob && w >= *threshold_g => {
                        Some(*bonus_pct as i64)
                    }
                    _ => None,
                })
                .max()
                .unwrap_or(0)
        };
        let (p_lo, p_hi) = (pct_for(w_lo), pct_for(w_hi));
        if p_lo > 0 || p_hi > 0 {
            let (a_lo, a_hi) = (v_lo.pct(p_lo), v_hi.pct(p_hi));
            total_low += a_lo;
            total_high += a_hi;
            adjustments.push(LotAdjustment { code: format!("BULK_TIER:{glob}"), amount_low: a_lo, amount_high: a_hi });
            if !codes.contains(&Code::BulkTierApplied) {
                codes.push(Code::BulkTierApplied);
            }
        }
    }

    let dist = distance_m(req);
    if !lines.is_empty() {
        for m in &rules.modifiers {
            match m {
                Modifier::DistanceDeduction { free_km, per_km_paise } => {
                    let extra_m = (dist - *free_km as i64 * 1000).max(0);
                    let d = Paise(div_round(*per_km_paise as i128 * extra_m as i128, 1000) as i64);
                    if d.0 > 0 {
                        total_low -= d;
                        total_high -= d;
                        adjustments.push(LotAdjustment { code: "DISTANCE_DEDUCTION".into(), amount_low: -d, amount_high: -d });
                        codes.push(Code::DistanceDeductionApplied);
                    }
                }
                Modifier::NoLiftSurchargePerFloor { value } if !req.pickup.lift_available && req.pickup.floor > 0 => {
                    let d = Paise(value * req.pickup.floor as i64);
                    if d.0 > 0 {
                        total_low -= d;
                        total_high -= d;
                        adjustments.push(LotAdjustment { code: "NO_LIFT_SURCHARGE".into(), amount_low: -d, amount_high: -d });
                        codes.push(Code::NoLiftSurchargeApplied);
                    }
                }
                _ => {}
            }
        }
    }
    if total_low.0 < 0 || total_high.0 < 0 {
        codes.push(Code::ClampedToZero);
    }
    let total_low = total_low.max0();
    let total_high = total_high.max0();

    // 7. Minimums.
    if let Some(min) = rules.minimums.order_value_paise {
        if total_high.0 < min {
            if estimate {
                ineligible.push(Code::BelowMinOrderValue);
            } else {
                codes.push(Code::BelowMinOrderValue);
            }
        } else if total_low.0 < min {
            codes.push(Code::MayFallBelowMinOrderValue);
        }
    }
    if let Some(min) = rules.minimums.weight_g
        && covered_weight_high.0 < min
    {
        if estimate {
            ineligible.push(Code::BelowMinWeight);
        } else {
            codes.push(Code::BelowMinWeight);
        }
    }

    let n = req.items.len().max(1) as u64;
    let coverage_permille = (covered.len() as u64 * 1000 / n) as u32;
    let eligible = ineligible.is_empty();
    let mut explanation_codes = ineligible;
    explanation_codes.extend(codes);

    Quote {
        eligible,
        covered_item_ids: covered,
        uncovered_item_ids: uncovered,
        coverage_permille,
        quote_low: total_low,
        quote_high: total_high,
        covered_weight_low,
        covered_weight_high,
        lines,
        adjustments,
        explanation_codes,
        item_notes: notes,
        distance_m: dist,
        engine_version: ENGINE_VERSION.to_string(),
        rule_set_version: rules.version,
    }
}
