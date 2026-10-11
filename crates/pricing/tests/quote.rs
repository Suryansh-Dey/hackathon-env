//! Golden and property tests for the quote engine (PLAN.md §12.3 "Tests").

use chrono::{TimeZone, Utc};
use kabados_domain::geo::LatLng;
use kabados_domain::ids::ItemId;
use kabados_domain::ladder::LadderLevel;
use kabados_domain::listing::{PlausibleWeight, Weekday, WeightSource};
use kabados_domain::offer::Rate;
use kabados_domain::ruleset::{Modifier, RuleLine, RuleSet};
use kabados_domain::taxonomy::{Taxonomy, WeightRange};
use kabados_domain::{Grams, Paise};
use kabados_pricing::*;
use proptest::prelude::*;
use std::collections::BTreeMap;

fn plan_rules() -> RuleSet {
    serde_json::from_str(
        r#"{
          "rule_set_id": "rs_0192f5b8c1a77d3e9b1e0c2d4f6a8b0c",
          "vendor_id": "usr_0192f5b8c1a77d3e9b1e0c2d4f6a8b0c",
          "version": 7,
          "status": "PUBLISHED",
          "effective_from": "2026-10-12T09:00:00+05:30",
          "service_area": {"type": "RADIUS", "center": {"lat": 28.61, "lng": 77.21}, "radius_km": 8},
          "schedule": {"days": ["MON","TUE","WED","THU","FRI","SAT"], "slots": ["09:00-13:00"], "daily_capacity_kg": 800},
          "minimums": {"order_value_paise": 20000, "weight_g": 5000},
          "lines": [
            {"category_id": "metal.copper", "grade": "clean", "price_per_kg_paise": 65000},
            {"category_id": "metal.copper", "grade": "mixed", "price_per_kg_paise": 55000},
            {"category_id": "paper.newspaper", "price_per_kg_paise": 1400, "min_weight_g": 5000},
            {"category_id": "ewaste.ceew.split_ac_outdoor", "price_per_piece_paise": 250000, "conditions": {"compressor_present": true}}
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

fn item(cat: &str, grade: Option<&str>, kg: i64, ladder: LadderLevel) -> QuoteItem {
    QuoteItem {
        item_id: ItemId::new(),
        category_id: cat.into(),
        grade: grade.map(Into::into),
        attributes: BTreeMap::new(),
        quantity: 1,
        est_weight: Grams::from_kg(kg),
        plausible: None,
        ladder,
        hazardous: Taxonomy::seed().is_hazardous(cat),
        weighed: None,
    }
}

fn pickup(floor: i32, lift: bool) -> PickupContext {
    PickupContext { location: LatLng { lat: 28.62, lng: 77.22 }, floor, lift_available: lift, days: vec![Weekday::Sat] }
}

fn vendor(hazardous_authorized: bool) -> VendorContext {
    VendorContext { active: true, hazardous_authorized, yard: LatLng { lat: 28.61, lng: 77.21 }, capacity_left: None }
}

fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 13, 6, 0, 0).unwrap()
}

fn run(
    items: &[QuoteItem],
    p: &PickupContext,
    v: &VendorContext,
    rules: &RuleSet,
    overrides: &BTreeMap<ItemId, Rate>,
    mode: QuoteMode,
) -> Quote {
    quote(&QuoteRequest {
        items,
        pickup: p,
        vendor: v,
        rules,
        taxonomy: Taxonomy::seed(),
        road_distance_m: Some(5000),
        overrides,
        mode,
        now: now(),
    })
}

#[test]
fn golden_estimate_matches_hand_calculation() {
    let rules = plan_rules();
    let items =
        vec![item("metal.copper", Some("clean"), 10, LadderLevel::L2CloseUp), item("paper.newspaper", None, 60, LadderLevel::L0Claimed)];
    let q = run(&items, &pickup(2, false), &vendor(false), &rules, &BTreeMap::new(), QuoteMode::Estimate);
    assert!(q.eligible, "{:?}", q.explanation_codes);
    assert_eq!(q.coverage_permille, 1000);
    // copper: 8.5–11.5 kg × ₹650 = 5,52,500–7,47,500 paise, +3% → 5,69,075–7,69,925
    assert_eq!((q.lines[0].value_low, q.lines[0].value_high), (Paise(569_075), Paise(769_925)));
    // newspaper: 51–69 kg × ₹14 = 71,400–96,600, −5% → 67,830–91,770
    assert_eq!((q.lines[1].value_low, q.lines[1].value_high), (Paise(67_830), Paise(91_770)));
    // + bulk 5% (3,392 / 4,589) − distance 1,000 − two floors without lift 2,000
    assert_eq!((q.quote_low, q.quote_high), (Paise(637_297), Paise(863_284)));
    assert!(q.explanation_codes.contains(&Code::BulkTierApplied));
    assert!(q.explanation_codes.contains(&Code::DistanceDeductionApplied));
    assert!(q.explanation_codes.contains(&Code::NoLiftSurchargeApplied));
    assert_eq!(q.engine_version, ENGINE_VERSION);
    assert_eq!(q.rule_set_version, 7);
}

#[test]
fn golden_settlement_uses_weighed_weight_as_low_and_high() {
    let rules = plan_rules();
    let mut items =
        vec![item("metal.copper", Some("clean"), 10, LadderLevel::L2CloseUp), item("paper.newspaper", None, 60, LadderLevel::L0Claimed)];
    items[0].weighed = Some(Grams(10_200));
    items[1].weighed = Some(Grams(58_000));
    let overrides: BTreeMap<ItemId, Rate> = [
        (items[0].item_id.clone(), Rate::Kg { paise_per_kg: Paise(65_000) }),
        (items[1].item_id.clone(), Rate::Kg { paise_per_kg: Paise(1_400) }),
    ]
    .into();
    let q = run(&items, &pickup(2, false), &vendor(false), &rules, &overrides, QuoteMode::Settlement);
    assert_eq!(q.quote_low, q.quote_high);
    // 6,63,000 + 3% = 6,82,890; 81,200 − 5% = 77,140; bulk +3,857; −1,000 −2,000
    assert_eq!(q.quote_low, Paise(760_887));
}

#[test]
fn eligibility_rules() {
    let rules = plan_rules();
    let v = vendor(false);
    let far = PickupContext { location: LatLng { lat: 28.9, lng: 77.5 }, ..pickup(0, true) };
    let q =
        run(&[item("metal.copper", Some("clean"), 10, LadderLevel::L0Claimed)], &far, &v, &rules, &BTreeMap::new(), QuoteMode::Estimate);
    assert!(!q.eligible && q.explanation_codes.contains(&Code::OutsideServiceArea));

    let mut ac = item("ewaste.ceew.split_ac_outdoor", None, 35, LadderLevel::L1PhotoConsistent);
    ac.attributes.insert("compressor_present".into(), kabados_domain::listing::AttrValue::Bool(true));
    let q = run(&[ac.clone()], &pickup(0, true), &v, &rules, &BTreeMap::new(), QuoteMode::Estimate);
    assert!(q.explanation_codes.contains(&Code::HazardousNotAuthorized));
    let q = run(&[ac], &pickup(0, true), &vendor(true), &rules, &BTreeMap::new(), QuoteMode::Estimate);
    assert!(q.eligible, "{:?}", q.explanation_codes);
    assert_eq!(q.quote_low, Paise(250_000 - 1000));

    let q = run(
        &[item("glass.bottles", None, 20, LadderLevel::L0Claimed), item("metal.copper", Some("mixed"), 6, LadderLevel::L1PhotoConsistent)],
        &pickup(0, true),
        &v,
        &rules,
        &BTreeMap::new(),
        QuoteMode::Estimate,
    );
    assert!(q.eligible);
    assert_eq!(q.coverage_permille, 500);
    assert!(q.item_notes.iter().any(|n| n.code == Code::ExcludedCategory));

    let q = run(
        &[item("paper.newspaper", None, 2, LadderLevel::L0Claimed)],
        &pickup(0, true),
        &v,
        &rules,
        &BTreeMap::new(),
        QuoteMode::Estimate,
    );
    assert!(!q.eligible);
    assert!(q.item_notes.iter().any(|n| n.code == Code::LineBelowMinWeight));

    let mut sunday = pickup(0, true);
    sunday.days = vec![Weekday::Sun];
    let q =
        run(&[item("metal.copper", Some("clean"), 10, LadderLevel::L0Claimed)], &sunday, &v, &rules, &BTreeMap::new(), QuoteMode::Estimate);
    assert!(q.explanation_codes.contains(&Code::ScheduleMismatch));
}

#[test]
fn reference_table_clamps_piece_weights() {
    let rules = plan_rules();
    let mut ac = item("ewaste.ceew.split_ac_outdoor", None, 200, LadderLevel::L1PhotoConsistent);
    ac.plausible =
        Some(PlausibleWeight { range: WeightRange { min: Grams(25_000), max: Grams(50_000) }, source: WeightSource::ReferenceTable });
    let (lo, hi, clamped) = weight_range(&ac, Taxonomy::seed(), QuoteMode::Estimate).unwrap();
    assert!(clamped);
    assert_eq!((lo, hi), (Grams(50_000), Grams(50_000)));
    let _ = rules;
}

#[test]
fn most_specific_line_wins_and_ties_break_by_order() {
    let mut rules = plan_rules();
    rules.lines.insert(
        0,
        RuleLine {
            category_id: "metal.copper".into(),
            grade: None,
            unit: None,
            price_per_kg_paise: Some(40_000),
            price_per_piece_paise: None,
            min_weight_g: None,
            conditions: Default::default(),
        },
    );
    let it = item("metal.copper", Some("clean"), 10, LadderLevel::L1PhotoConsistent);
    let (i, l) = match_line(&rules, Taxonomy::seed(), &it, it.est_weight).unwrap();
    assert_eq!((i, l.price_per_kg_paise), (1, Some(65_000)));
    let ungraded = item("metal.copper", None, 10, LadderLevel::L1PhotoConsistent);
    assert_eq!(match_line(&rules, Taxonomy::seed(), &ungraded, ungraded.est_weight).unwrap().0, 0);
}

// ---------- property tests ----------

const KG_CATS: &[(&str, Option<&str>)] = &[
    ("metal.copper", Some("clean")),
    ("metal.copper", Some("mixed")),
    ("paper.newspaper", None),
    ("paper.cardboard", None),
    ("plastic.pet_bottle", None),
    ("metal.iron_steel", None),
];

fn ladder() -> impl Strategy<Value = LadderLevel> {
    prop_oneof![
        Just(LadderLevel::L0Claimed),
        Just(LadderLevel::L1PhotoConsistent),
        Just(LadderLevel::L2CloseUp),
        Just(LadderLevel::L3Identifier),
    ]
}

fn arb_rules() -> impl Strategy<Value = RuleSet> {
    (
        prop::collection::vec(1i64..2_000_000, KG_CATS.len()),
        0u32..=50,
        0u32..=50,
        0u32..=20,
        0i64..=10_000,
        1i64..200_000,
        0u32..=50,
        0i64..=50_000,
        any::<bool>(),
    )
        .prop_map(|(prices, bonus, discount, free_km, per_km, bulk_t, bulk_pct, floor, partial)| {
            let mut r = plan_rules();
            r.lines = KG_CATS
                .iter()
                .zip(prices)
                .map(|((c, g), p)| RuleLine {
                    category_id: (*c).into(),
                    grade: g.map(Into::into),
                    unit: None,
                    price_per_kg_paise: Some(p),
                    price_per_piece_paise: None,
                    min_weight_g: None,
                    conditions: Default::default(),
                })
                .collect();
            r.modifiers = vec![
                Modifier::VerifiedBonusPct { min_ladder: LadderLevel::L2CloseUp, value: bonus },
                Modifier::UnverifiedDiscountPct { max_ladder: LadderLevel::L0Claimed, value: discount },
                Modifier::DistanceDeduction { free_km, per_km_paise: per_km },
                Modifier::BulkTier { category_glob: "paper.*".into(), threshold_g: bulk_t, bonus_pct: bulk_pct },
                Modifier::NoLiftSurchargePerFloor { value: floor },
            ];
            r.minimums = Default::default();
            r.accept_partial_lots = partial;
            r
        })
}

fn arb_items() -> impl Strategy<Value = Vec<QuoteItem>> {
    prop::collection::vec((0..KG_CATS.len(), 0i64..500_000, ladder()), 1..6).prop_map(|v| {
        v.into_iter()
            .map(|(c, g, l)| {
                let mut it = item(KG_CATS[c].0, KG_CATS[c].1, 0, l);
                it.est_weight = Grams(g);
                it
            })
            .collect()
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn never_negative_and_low_le_high(rules in arb_rules(), items in arb_items(), floor in 0i32..10, lift: bool, dist in 0i64..30_000) {
        let p = pickup(floor, lift);
        let q = quote(&QuoteRequest { items: &items, pickup: &p, vendor: &vendor(false), rules: &rules, taxonomy: Taxonomy::seed(), road_distance_m: Some(dist), overrides: &BTreeMap::new(), mode: QuoteMode::Estimate, now: now() });
        prop_assert!(q.quote_low.0 >= 0);
        prop_assert!(q.quote_low <= q.quote_high);
        for l in &q.lines {
            prop_assert!(l.value_low <= l.value_high);
            prop_assert!(l.weight_low <= l.weight_high);
        }
    }

    #[test]
    fn monotonic_in_weight(rules in arb_rules(), items in arb_items(), idx in 0usize..6, extra in 0i64..200_000, floor in 0i32..5, lift: bool) {
        let p = pickup(floor, lift);
        let base = quote(&QuoteRequest { items: &items, pickup: &p, vendor: &vendor(false), rules: &rules, taxonomy: Taxonomy::seed(), road_distance_m: Some(4000), overrides: &BTreeMap::new(), mode: QuoteMode::Estimate, now: now() });
        let mut heavier = items.clone();
        let i = idx % heavier.len();
        heavier[i].est_weight = Grams(heavier[i].est_weight.0 + extra);
        let more = quote(&QuoteRequest { items: &heavier, pickup: &p, vendor: &vendor(false), rules: &rules, taxonomy: Taxonomy::seed(), road_distance_m: Some(4000), overrides: &BTreeMap::new(), mode: QuoteMode::Estimate, now: now() });
        prop_assert!(more.quote_low >= base.quote_low, "low {} -> {}", base.quote_low.0, more.quote_low.0);
        prop_assert!(more.quote_high >= base.quote_high, "high {} -> {}", base.quote_high.0, more.quote_high.0);
    }

    #[test]
    fn modifiers_are_bounded(rules in arb_rules(), items in arb_items()) {
        let p = pickup(0, true);
        let q = quote(&QuoteRequest { items: &items, pickup: &p, vendor: &vendor(false), rules: &rules, taxonomy: Taxonomy::seed(), road_distance_m: Some(0), overrides: &BTreeMap::new(), mode: QuoteMode::Estimate, now: now() });
        let base_high: i64 = q.lines.iter().map(|l| l.base_high.0).sum();
        let base_low: i64 = q.lines.iter().map(|l| l.base_low.0).sum();
        // Each modifier is capped at 50%: at most +50% verification, +50% bulk, −50% discount.
        let n = q.lines.len() as i64 + 2;
        prop_assert!(q.quote_high.0 <= base_high * 9 / 4 + n, "{} vs base {}", q.quote_high.0, base_high);
        prop_assert!(q.quote_low.0 >= base_low / 2 - n);
    }

    #[test]
    fn deterministic_and_serializable(rules in arb_rules(), items in arb_items()) {
        let p = pickup(1, false);
        let req = QuoteRequest { items: &items, pickup: &p, vendor: &vendor(false), rules: &rules, taxonomy: Taxonomy::seed(), road_distance_m: None, overrides: &BTreeMap::new(), mode: QuoteMode::Estimate, now: now() };
        let a = quote(&req);
        let b = quote(&req);
        prop_assert_eq!(&a, &b);
        let back: Quote = serde_json::from_value(serde_json::to_value(&a).unwrap()).unwrap();
        prop_assert_eq!(back, a);
    }

    #[test]
    fn settlement_is_point_valued(rules in arb_rules(), items in arb_items(), w in prop::collection::vec(1i64..300_000, 6)) {
        let mut items = items;
        for (it, g) in items.iter_mut().zip(w) { it.weighed = Some(Grams(g)); }
        let p = pickup(0, true);
        let q = quote(&QuoteRequest { items: &items, pickup: &p, vendor: &vendor(false), rules: &rules, taxonomy: Taxonomy::seed(), road_distance_m: Some(0), overrides: &BTreeMap::new(), mode: QuoteMode::Settlement, now: now() });
        prop_assert_eq!(q.quote_low, q.quote_high);
        for (l, it) in q.lines.iter().zip(items.iter()) {
            prop_assert_eq!(l.weight_low, it.weighed.unwrap());
        }
    }
}
