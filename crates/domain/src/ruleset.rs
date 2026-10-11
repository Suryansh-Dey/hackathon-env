//! Vendor rate card / rule set schema (PLAN.md §12.2).
//!
//! Money in paise and weights in grams (integers). Published versions are
//! immutable; deals keep the version they were priced with.

use crate::DomainError;
use crate::geo::{self, LatLng};
use crate::ids::*;
use crate::ladder::LadderLevel;
use crate::listing::{AttrValue, Weekday};
use crate::taxonomy::{self, Taxonomy, Unit};
use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Upper bound on any single price (₹10 lakh) to catch typos.
pub const MAX_PRICE_PAISE: i64 = 10_00_000_00;
pub const MAX_MODIFIER_PCT: u32 = 50;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ServiceArea {
    Radius { center: LatLng, radius_km: f64 },
    Polygon { points: Vec<LatLng> },
}

impl ServiceArea {
    pub fn contains(&self, p: &LatLng) -> bool {
        match self {
            ServiceArea::Radius { center, radius_km } => center.distance_m(p) <= radius_km * 1000.0,
            ServiceArea::Polygon { points } => geo::point_in_polygon(p, points),
        }
    }

    /// Resolution-8 cells covered (for the coverage index, PLAN.md §12.5).
    pub fn cells_r8(&self) -> Result<Vec<String>, DomainError> {
        match self {
            ServiceArea::Radius { center, radius_km } => geo::cells_within_radius(center, radius_km * 1000.0),
            ServiceArea::Polygon { points } => geo::cells_within_polygon(points),
        }
    }

    /// Radius used to normalise distance in ranking (PLAN.md §12.4).
    pub fn max_radius_km(&self) -> f64 {
        match self {
            ServiceArea::Radius { radius_km, .. } => *radius_km,
            ServiceArea::Polygon { points } => {
                let n = points.len().max(1) as f64;
                let c = LatLng { lat: points.iter().map(|p| p.lat).sum::<f64>() / n, lng: points.iter().map(|p| p.lng).sum::<f64>() / n };
                points.iter().map(|p| p.distance_m(&c)).fold(0.0, f64::max) / 1000.0
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct Schedule {
    pub days: Vec<Weekday>,
    /// `HH:MM-HH:MM` windows (IST).
    pub slots: Vec<String>,
    pub daily_capacity_kg: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct Minimums {
    pub order_value_paise: Option<i64>,
    pub weight_g: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct RuleLine {
    pub category_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grade: Option<String>,
    /// Defaults to the category's unit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<Unit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_per_kg_paise: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_per_piece_paise: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_weight_g: Option<i64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub conditions: BTreeMap<String, AttrValue>,
}

impl RuleLine {
    /// Specificity used for line matching: grade + conditions + weight threshold.
    pub fn specificity(&self) -> u32 {
        self.grade.is_some() as u32 * 4 + self.conditions.len() as u32 * 2 + self.min_weight_g.is_some() as u32
    }

    pub fn effective_unit(&self, tax: &Taxonomy) -> Option<Unit> {
        self.unit.or_else(|| tax.get(&self.category_id).map(|c| c.unit))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Modifier {
    /// +value% on items at or above `min_ladder`.
    VerifiedBonusPct { min_ladder: LadderLevel, value: u32 },
    /// −value% on items at or below `max_ladder`.
    UnverifiedDiscountPct { max_ladder: LadderLevel, value: u32 },
    /// ₹ per km beyond `free_km` (road distance from the yard).
    DistanceDeduction { free_km: u32, per_km_paise: i64 },
    /// +bonus% when the lot's weight in `category_glob` reaches `threshold_g`.
    BulkTier { category_glob: String, threshold_g: i64, bonus_pct: u32 },
    /// Deduction per floor when no lift is available.
    #[serde(rename = "NO_LIFT_SURCHARGE_PER_FLOOR_PAISE")]
    NoLiftSurchargePerFloor { value: i64 },
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RuleSetStatus {
    Draft,
    Published,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct RuleSet {
    pub rule_set_id: RuleSetId,
    pub vendor_id: UserId,
    pub version: u32,
    pub status: RuleSetStatus,
    #[schema(value_type = String, format = DateTime)]
    pub effective_from: DateTime<FixedOffset>,
    pub service_area: ServiceArea,
    pub schedule: Schedule,
    #[serde(default)]
    pub minimums: Minimums,
    pub lines: Vec<RuleLine>,
    #[serde(default)]
    pub modifiers: Vec<Modifier>,
    #[serde(default)]
    pub exclusions: Vec<String>,
    #[serde(default = "yes")]
    pub accept_partial_lots: bool,
    /// How the draft was produced (form, text instruction, bulk change).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct ValidationIssue {
    pub path: String,
    pub message: String,
}

/// Validates a time window `HH:MM-HH:MM` with start < end.
pub fn parse_window(w: &str) -> Option<(u32, u32)> {
    let (a, b) = w.split_once('-')?;
    let mins = |s: &str| -> Option<u32> {
        let (h, m) = s.split_once(':')?;
        let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
        (h < 24 && m < 60 && s.len() == 5).then_some(h * 60 + m)
    };
    let (x, y) = (mins(a)?, mins(b)?);
    (x < y).then_some((x, y))
}

impl RuleSet {
    /// Semantic validation (PLAN.md §12.2). Returns every issue found.
    pub fn validate(&self, tax: &Taxonomy, hazardous_authorized: bool) -> Vec<ValidationIssue> {
        let mut issues = Vec::new();
        let mut push = |path: String, message: String| issues.push(ValidationIssue { path, message });

        match &self.service_area {
            ServiceArea::Radius { center, radius_km } => {
                if center.validate().is_err() {
                    push("service_area.center".into(), "invalid coordinates".into());
                }
                if !(0.5..=100.0).contains(radius_km) {
                    push("service_area.radius_km".into(), "radius must be between 0.5 and 100 km".into());
                }
            }
            ServiceArea::Polygon { points } => {
                if points.len() < 3 || points.len() > 200 {
                    push("service_area.points".into(), "polygon needs 3 to 200 points".into());
                }
                if points.iter().any(|p| p.validate().is_err()) {
                    push("service_area.points".into(), "invalid coordinates".into());
                }
            }
        }
        if self.schedule.days.is_empty() {
            push("schedule.days".into(), "at least one working day is required".into());
        }
        for (i, s) in self.schedule.slots.iter().enumerate() {
            if parse_window(s).is_none() {
                push(format!("schedule.slots[{i}]"), format!("{s:?} is not a HH:MM-HH:MM window"));
            }
        }
        if let Some(v) = self.minimums.order_value_paise
            && !(0..=MAX_PRICE_PAISE).contains(&v)
        {
            push("minimums.order_value_paise".into(), "out of range".into());
        }
        if let Some(v) = self.minimums.weight_g
            && v < 0
        {
            push("minimums.weight_g".into(), "must not be negative".into());
        }
        if self.lines.is_empty() {
            push("lines".into(), "add at least one price line".into());
        }

        let mut seen: BTreeSet<(String, Option<String>, String, Option<i64>)> = BTreeSet::new();
        for (i, l) in self.lines.iter().enumerate() {
            let p = format!("lines[{i}]");
            let Some(cat) = tax.get(&l.category_id) else {
                push(format!("{p}.category_id"), format!("unknown category {:?}", l.category_id));
                continue;
            };
            if cat.hazardous && !hazardous_authorized {
                push(format!("{p}.category_id"), format!("{} is hazardous and needs a valid authorization", cat.id));
            }
            if let Some(g) = &l.grade
                && !cat.has_grade(g)
            {
                push(format!("{p}.grade"), format!("{:?} is not a grade of {}", g, cat.id));
            }
            let unit = l.unit.unwrap_or(cat.unit);
            if unit != cat.unit {
                push(format!("{p}.unit"), format!("{} is priced per {:?}", cat.id, cat.unit));
            }
            match (unit, l.price_per_kg_paise, l.price_per_piece_paise) {
                (Unit::Kg, Some(v), None) | (Unit::Piece, None, Some(v)) => {
                    if !(1..=MAX_PRICE_PAISE).contains(&v) {
                        push(format!("{p}.price"), "price must be between 1 paisa and ₹10,00,000".into());
                    }
                }
                (Unit::Kg, _, _) => push(format!("{p}.price_per_kg_paise"), "set price_per_kg_paise only".into()),
                (Unit::Piece, _, _) => push(format!("{p}.price_per_piece_paise"), "set price_per_piece_paise only".into()),
            }
            if l.min_weight_g.is_some_and(|w| w <= 0) {
                push(format!("{p}.min_weight_g"), "must be positive".into());
            }
            let key = (l.category_id.clone(), l.grade.clone(), serde_json::to_string(&l.conditions).unwrap_or_default(), l.min_weight_g);
            if !seen.insert(key) {
                push(p.clone(), "duplicates another line with equal specificity".into());
            }
            if self.exclusions.iter().any(|g| taxonomy::glob_matches(g, &l.category_id)) {
                push(p, "category is also excluded".into());
            }
        }

        for (i, m) in self.modifiers.iter().enumerate() {
            let p = format!("modifiers[{i}]");
            match m {
                Modifier::VerifiedBonusPct { value, .. } | Modifier::UnverifiedDiscountPct { value, .. } => {
                    if *value > MAX_MODIFIER_PCT {
                        push(p, format!("percentage must be at most {MAX_MODIFIER_PCT}"));
                    }
                }
                Modifier::BulkTier { category_glob, threshold_g, bonus_pct } => {
                    if !taxonomy::is_valid_glob(category_glob) {
                        push(format!("{p}.category_glob"), "invalid category glob".into());
                    }
                    if *threshold_g <= 0 {
                        push(format!("{p}.threshold_g"), "must be positive".into());
                    }
                    if *bonus_pct > MAX_MODIFIER_PCT {
                        push(p, format!("percentage must be at most {MAX_MODIFIER_PCT}"));
                    }
                }
                Modifier::DistanceDeduction { free_km, per_km_paise } => {
                    if *per_km_paise < 0 || *per_km_paise > 100_00 || *free_km > 100 {
                        push(p, "distance deduction out of range".into());
                    }
                }
                Modifier::NoLiftSurchargePerFloor { value } => {
                    if !(0..=500_00).contains(value) {
                        push(p, "floor surcharge out of range".into());
                    }
                }
            }
        }
        for (i, g) in self.exclusions.iter().enumerate() {
            if !taxonomy::is_valid_glob(g) {
                push(format!("exclusions[{i}]"), format!("{g:?} is not a valid category glob"));
            }
        }
        issues
    }

    pub fn is_excluded(&self, category_id: &str) -> bool {
        self.exclusions.iter().any(|g| taxonomy::glob_matches(g, category_id))
    }

    /// Categories this rule set can price (for the coverage index).
    pub fn covered_categories(&self) -> BTreeSet<String> {
        self.lines.iter().filter(|l| !self.is_excluded(&l.category_id)).map(|l| l.category_id.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn sample() -> RuleSet {
        serde_json::from_str(
            r#"{
              "rule_set_id": "rs_0192f5b8c1a77d3e9b1e0c2d4f6a8b0c",
              "vendor_id": "usr_0192f5b8c1a77d3e9b1e0c2d4f6a8b0c",
              "version": 7,
              "status": "PUBLISHED",
              "effective_from": "2026-10-12T09:00:00+05:30",
              "service_area": {"type": "RADIUS", "center": {"lat": 28.61, "lng": 77.21}, "radius_km": 8},
              "schedule": {"days": ["MON","TUE","WED","THU","FRI","SAT"], "slots": ["09:00-13:00","14:00-19:00"], "daily_capacity_kg": 800},
              "minimums": {"order_value_paise": 20000, "weight_g": 5000},
              "lines": [
                {"category_id": "metal.copper", "grade": "clean", "price_per_kg_paise": 65000},
                {"category_id": "metal.copper", "grade": "mixed", "price_per_kg_paise": 55000},
                {"category_id": "paper.newspaper", "price_per_kg_paise": 1400, "min_weight_g": 5000}
              ],
              "modifiers": [
                {"type": "VERIFIED_BONUS_PCT", "min_ladder": "L2_CLOSE_UP", "value": 3},
                {"type": "UNVERIFIED_DISCOUNT_PCT", "max_ladder": "L0_CLAIMED", "value": 5},
                {"type": "DISTANCE_DEDUCTION", "free_km": 3, "per_km_paise": 500},
                {"type": "BULK_TIER", "category_glob": "paper.*", "threshold_g": 50000, "bonus_pct": 5},
                {"type": "NO_LIFT_SURCHARGE_PER_FLOOR_PAISE", "value": 1000}
              ],
              "exclusions": ["glass.*"],
              "accept_partial_lots": true
            }"#,
        )
        .unwrap()
    }

    #[test]
    fn plan_example_parses_and_validates() {
        let rs = sample();
        let issues = rs.validate(Taxonomy::seed(), false);
        assert!(issues.is_empty(), "{issues:?}");
        assert!(rs.is_excluded("glass.bottles"));
        assert!(rs.covered_categories().contains("metal.copper"));
        assert!(rs.service_area.contains(&LatLng { lat: 28.62, lng: 77.22 }));
        let round_trip: RuleSet = serde_json::from_value(serde_json::to_value(&rs).unwrap()).unwrap();
        assert_eq!(round_trip, rs);
    }

    #[test]
    fn catches_semantic_errors() {
        let mut rs = sample();
        rs.lines.push(RuleLine {
            category_id: "battery.lead_acid".into(),
            grade: None,
            unit: None,
            price_per_kg_paise: Some(9000),
            price_per_piece_paise: None,
            min_weight_g: None,
            conditions: Default::default(),
        });
        rs.lines.push(rs.lines[0].clone());
        rs.lines.push(RuleLine {
            category_id: "glass.bottles".into(),
            grade: Some("blue".into()),
            unit: None,
            price_per_kg_paise: None,
            price_per_piece_paise: Some(5),
            min_weight_g: None,
            conditions: Default::default(),
        });
        rs.schedule.slots.push("19:00-09:00".into());
        let issues = rs.validate(Taxonomy::seed(), false);
        let text = format!("{issues:?}");
        assert!(text.contains("hazardous"), "{text}");
        assert!(text.contains("duplicates"), "{text}");
        assert!(text.contains("not a grade"), "{text}");
        assert!(text.contains("price_per_kg_paise only"), "{text}");
        assert!(text.contains("excluded"), "{text}");
        assert!(text.contains("HH:MM"), "{text}");
        assert!(rs.validate(Taxonomy::seed(), true).iter().all(|i| !i.message.contains("hazardous")));
    }

    #[test]
    fn windows() {
        assert_eq!(parse_window("09:00-13:00"), Some((540, 780)));
        assert_eq!(parse_window("9:00-13:00"), None);
        assert_eq!(parse_window("13:00-09:00"), None);
    }
}
