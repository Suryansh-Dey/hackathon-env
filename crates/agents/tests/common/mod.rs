#![allow(dead_code)]

use kabados_agents::matching::*;
use kabados_agents::synthetic;
use kabados_agents::{ImageInput, VerifyInput};
use kabados_domain::Grams;
use kabados_domain::ids::{ImageId, ItemId, ListingId};
use kabados_domain::ladder::LadderLevel;
use kabados_domain::listing::{Hazard, Item, ItemSource};
use kabados_domain::taxonomy::Unit;
use std::collections::BTreeMap;

pub fn png(bytes: Vec<u8>) -> ImageInput {
    ImageInput::new(ImageId::new(), "image/png", bytes)
}

pub fn item(cat: &str, kg: i64) -> Item {
    Item {
        id: ItemId::new(),
        category_id: cat.into(),
        grade: None,
        attributes: BTreeMap::new(),
        quantity: 1,
        unit: Unit::Kg,
        est_weight_g: Some(Grams::from_kg(kg)),
        plausible_weight: None,
        hazard: Hazard { is_hazardous: false, class: None, handling: None },
        evidence: vec![],
        source: ItemSource::Seller,
        ai_category_id: None,
        ai_confidence: None,
        ladder: LadderLevel::L0Claimed,
        note: None,
    }
}

pub fn verify_input(images: Vec<ImageInput>, description: &str) -> VerifyInput {
    VerifyInput {
        items: vec![item("metal.copper", 12)],
        images,
        description: Some(description.into()),
        asking_price: None,
        claim_reason: Some("branded copper winding, unused".into()),
        prior_rounds: 0,
        label_text: BTreeMap::new(),
    }
}

pub fn busy_photo() -> ImageInput {
    png(synthetic::render_busy(320, 240, 7))
}

#[allow(clippy::too_many_arguments)] // test fixture builder
fn vendor(
    id: &str,
    eligible: bool,
    low: i64,
    high: i64,
    coverage: u32,
    near: bool,
    cats: &[&str],
    hazardous: bool,
    covered: &[&str],
) -> (CandidateVendor, VendorQuote, VendorSignal, bool) {
    (
        CandidateVendor {
            vendor_id: id.into(),
            rule_set_version: 3,
            categories: cats.iter().map(|c| c.to_string()).collect(),
            hazardous_authorized: hazardous,
        },
        VendorQuote {
            vendor_id: id.into(),
            eligible,
            quote_low_paise: low,
            quote_high_paise: high,
            coverage_permille: coverage,
            covered_item_ids: covered.iter().map(|c| c.to_string()).collect(),
            explanation_codes: if eligible { vec![] } else { vec!["HAZARDOUS_NOT_AUTHORIZED".into()] },
        },
        VendorSignal { vendor_id: id.into(), reliability: 0.9, rating: 0.8, response_minutes: Some(20), current_load: 1 },
        near,
    )
}

/// A mixed lot (AC + newspaper) with authorized and unauthorized vendors.
pub fn fake_tools() -> (FakeMatchingTools, ListingId) {
    let listing = ListingId::new();
    let snapshot = ListingSnapshot {
        listing_id: listing.to_string(),
        ver: 1,
        items: vec![
            SnapshotItem {
                item_id: "itm_ac".into(),
                category_id: "ewaste.ceew.split_ac_outdoor".into(),
                grade: None,
                quantity: 1,
                est_weight_g: 35_000,
                ladder: "L1_PHOTO_CONSISTENT".into(),
                hazardous: true,
            },
            SnapshotItem {
                item_id: "itm_np".into(),
                category_id: "paper.newspaper".into(),
                grade: None,
                quantity: 1,
                est_weight_g: 20_000,
                ladder: "L0_CLAIMED".into(),
                hazardous: false,
            },
        ],
        h3_r8: "883da1140bfffff".into(),
        contains_hazardous: true,
        floor: 2,
        lift_available: false,
        preferred_days: vec!["SAT".into()],
    };
    let tools = FakeMatchingTools {
        snapshot: Some(snapshot),
        vendors: vec![
            vendor(
                "usr_recycler",
                true,
                300_000,
                360_000,
                1000,
                true,
                &["ewaste.ceew.split_ac_outdoor", "paper.newspaper"],
                true,
                &["itm_ac", "itm_np"],
            ),
            vendor("usr_kabadi", false, 25_000, 30_000, 500, true, &["paper.newspaper"], false, &["itm_np"]),
            vendor("usr_dismantler", true, 280_000, 340_000, 500, false, &["ewaste.ceew.split_ac_outdoor"], true, &["itm_ac"]),
        ],
        split: Some(SplitSuggestion {
            groups: vec![
                SplitGroup {
                    item_ids: vec!["itm_ac".into()],
                    vendor_ids: vec!["usr_dismantler".into()],
                    reason: "AC to an authorized dismantler".into(),
                },
                SplitGroup {
                    item_ids: vec!["itm_np".into()],
                    vendor_ids: vec!["usr_kabadi".into()],
                    reason: "Newspaper to a local kabadiwala".into(),
                },
            ],
        }),
        proof_unlocks: vec!["Add a photo of the newspaper bundle on a scale.".into()],
    };
    (tools, listing)
}
