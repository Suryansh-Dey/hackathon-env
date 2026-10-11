//! Matching fast path (PLAN.md §12.7): coverage index → rules engine →
//! rank → cache. Shared by the price suggestion, vendor feed, offer pre-fill
//! and the matching worker (which adds the agent slow path).

use crate::error::ApiResult;
use crate::state::AppState;
use chrono::Utc;
use kabados_db::Record;
use kabados_db::repo::ListingBundle;
use kabados_domain::Paise;
use kabados_domain::geo::{self, LatLng};
use kabados_domain::ids::{ItemId, UserId};
use kabados_domain::listing::{Item, Listing, ListingVersion};
use kabados_domain::offer::Rate;
use kabados_domain::ruleset::RuleSet;
use kabados_domain::user::VendorProfile;
use kabados_pricing::rank::{Candidate, RankWeights, rank};
use kabados_pricing::{PickupContext, Quote, QuoteItem, QuoteMode, QuoteRequest, VendorContext, quote};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// Default ring size for the H3 r8 search (PLAN.md §12.7 `ring(h3_r8, k)`).
pub const DEFAULT_RING: u32 = 6;
pub const MAX_RING: u32 = 14;
pub const CACHE_TTL_HOURS: i64 = 6;

pub fn quote_items(items: &[Item]) -> Vec<QuoteItem> {
    items
        .iter()
        .map(|i| QuoteItem {
            item_id: i.id.clone(),
            category_id: i.category_id.clone(),
            grade: i.grade.clone(),
            attributes: i.attributes.clone(),
            quantity: i.quantity,
            est_weight: i.est_weight_g.unwrap_or_default(),
            plausible: i.plausible_weight,
            ladder: i.ladder,
            hazardous: i.hazard.is_hazardous,
            weighed: None,
        })
        .collect()
}

/// Pickup context at the listing's r8 cell centre (vendors see only the cell).
pub fn pickup_ctx(l: &Listing, v: &ListingVersion) -> PickupContext {
    let location = geo::cell_center(&l.approx.h3_r8).unwrap_or(l.location);
    PickupContext { location, floor: v.pickup.floor, lift_available: v.pickup.lift_available, days: v.pickup.days.clone() }
}

pub fn vendor_ctx(v: &VendorProfile) -> VendorContext {
    VendorContext {
        active: v.kyc_approved() && !v.suspended,
        hazardous_authorized: v.hazardous_auth_valid(Utc::now()),
        yard: v.yard_location,
        capacity_left: None,
    }
}

/// Runs the pricing engine for one vendor's published rule set.
#[allow(clippy::too_many_arguments)]
pub fn quote_with(
    tax: &kabados_domain::taxonomy::Taxonomy,
    items: &[QuoteItem],
    pickup: &PickupContext,
    vendor: &VendorProfile,
    rules: &RuleSet,
    road_distance_m: Option<i64>,
    overrides: &BTreeMap<ItemId, Rate>,
    mode: QuoteMode,
) -> Quote {
    quote(&QuoteRequest {
        items,
        pickup,
        vendor: &vendor_ctx(vendor),
        rules,
        taxonomy: tax,
        road_distance_m,
        overrides,
        mode,
        now: Utc::now(),
    })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct MatchedVendor {
    pub vendor_id: UserId,
    pub vendor_name: String,
    pub score: f64,
    pub rating: f64,
    pub distance_m: i64,
    pub rule_set_version: u32,
    pub quote: Quote,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct FastPathResult {
    pub listing_id: kabados_domain::ids::ListingId,
    pub ver: u32,
    pub ring: u32,
    pub cache_key: String,
    pub from_cache: bool,
    /// Eligible vendors, best first.
    pub matches: Vec<MatchedVendor>,
    /// Vendors that cover some item but are ineligible (with reasons), for the agent.
    pub near_misses: Vec<MatchedVendor>,
}

impl FastPathResult {
    /// Fast path is good enough when enough eligible vendors cover the whole lot (PLAN.md §11.6).
    pub fn good_enough(&self, min_matches: usize) -> bool {
        self.matches.iter().filter(|m| m.quote.coverage_permille == 1000).count() >= min_matches
    }
}

/// Candidate vendors from the coverage index for the cells × categories.
pub async fn candidates(st: &AppState, cells: &[String], categories: &BTreeSet<String>) -> ApiResult<BTreeMap<UserId, u32>> {
    let mut out: BTreeMap<UserId, u32> = BTreeMap::new();
    let mut futs = Vec::new();
    for c in cells {
        for cat in categories {
            futs.push(st.db.coverage(c, cat));
        }
    }
    for r in futures_join(futs).await {
        for e in r? {
            out.insert(e.vendor_id, e.rule_set_version);
        }
    }
    Ok(out)
}

/// Runs futures 32 at a time: one Query per (cell, category) pair (PLAN.md §12.5).
async fn futures_join<F: std::future::Future>(futs: Vec<F>) -> Vec<F::Output> {
    let mut out = Vec::with_capacity(futs.len());
    let mut it = futs.into_iter().peekable();
    while it.peek().is_some() {
        let batch: Vec<F> = it.by_ref().take(32).collect();
        out.extend(futures::future::join_all(batch).await);
    }
    out
}

pub fn cache_key(listing: &Listing, ver: u32, cells: &[String], versions: &BTreeMap<UserId, u32>) -> String {
    let mut h = Sha256::new();
    h.update(listing.id.as_str());
    h.update(ver.to_le_bytes());
    for c in cells {
        h.update(c.as_bytes());
    }
    for (v, rv) in versions {
        h.update(v.as_str());
        h.update(rv.to_le_bytes());
    }
    hex::encode(&h.finalize()[..16])
}

/// The fast path: ring → coverage → quotes → rank → cache (6 h).
pub async fn fast_path(st: &AppState, b: &ListingBundle, ring: u32) -> ApiResult<FastPathResult> {
    let l = &b.listing.data;
    let ver = b.version.data.ver;
    let items = b.item_data();
    let cats: BTreeSet<String> = items.iter().map(|i| i.category_id.clone()).collect();
    let cells = geo::grid_disk(&l.approx.h3_r8, ring)?;
    let versions = candidates(st, &cells, &cats).await?;
    let key = cache_key(l, ver, &cells, &versions);
    if let Some(v) = st.db.match_cache_get(&key).await?
        && let Ok(mut r) = serde_json::from_value::<FastPathResult>(v)
    {
        r.from_cache = true;
        return Ok(r);
    }
    let tax = st.taxonomy().await?;
    let qitems = quote_items(&items);
    let pickup = pickup_ctx(l, &b.version.data);
    let mut loaded: Vec<(Record<VendorProfile>, RuleSet)> = Vec::new();
    for (vendor_id, rv) in &versions {
        if *vendor_id == l.seller_id {
            continue;
        }
        let (Some(v), Some(rs)) = (st.db.vendor(vendor_id).await?, st.db.rule_set_version(vendor_id, *rv).await?) else { continue };
        loaded.push((v, rs.data));
    }
    let yards: Vec<LatLng> = loaded.iter().map(|(v, _)| v.data.yard_location).collect();
    let legs = if yards.is_empty() { vec![] } else { st.ix.routes.matrix(&yards, &[pickup.location]).await.unwrap_or_default() };
    let mut matched = Vec::new();
    let mut near = Vec::new();
    for (i, (v, rs)) in loaded.iter().enumerate() {
        let road = legs.get(i).and_then(|r| r.first()).copied().flatten().map(|l| l.distance_m);
        let q = quote_with(&tax, &qitems, &pickup, &v.data, rs, road, &BTreeMap::new(), QuoteMode::Estimate);
        let mv = MatchedVendor {
            vendor_id: v.data.user_id.clone(),
            vendor_name: v.data.business_name.clone(),
            score: 0.0,
            rating: v.data.rating.stars(),
            distance_m: q.distance_m,
            rule_set_version: rs.version,
            quote: q,
        };
        if mv.quote.eligible {
            matched.push(mv)
        } else if !mv.quote.covered_item_ids.is_empty() {
            near.push(mv)
        }
    }
    let cands: Vec<Candidate> = matched
        .iter()
        .map(|m| {
            let v = &loaded.iter().find(|(v, _)| v.data.user_id == m.vendor_id).expect("loaded").0.data;
            Candidate {
                vendor_id: m.vendor_id.clone(),
                payout_mid: m.quote.mid().0,
                coverage_permille: m.quote.coverage_permille,
                distance_m: m.distance_m,
                max_radius_m: (loaded
                    .iter()
                    .find(|(x, _)| x.data.user_id == m.vendor_id)
                    .map(|(_, r)| r.service_area.max_radius_km())
                    .unwrap_or(10.0)
                    * 1000.0) as i64,
                reliability: v.reliability.smoothed(),
                rating: v.rating.smoothed(),
                response: v.response_minutes_p50.map(|m| (1.0 - m as f64 / 240.0).clamp(0.0, 1.0)).unwrap_or(0.5),
                completed_deals: v.rating.count,
            }
        })
        .collect();
    let ranked = rank(&cands, &RankWeights::default());
    let mut ordered = Vec::with_capacity(matched.len());
    for r in ranked {
        if let Some(pos) = matched.iter().position(|m| m.vendor_id == r.vendor_id) {
            let mut m = matched.swap_remove(pos);
            m.score = r.score;
            ordered.push(m);
        }
    }
    let result = FastPathResult {
        listing_id: l.id.clone(),
        ver,
        ring,
        cache_key: key.clone(),
        from_cache: false,
        matches: ordered,
        near_misses: near,
    };
    st.db
        .match_cache_put(&key, &serde_json::to_value(&result).map_err(anyhow::Error::from)?, chrono::Duration::hours(CACHE_TTL_HOURS))
        .await?;
    Ok(result)
}

/// Seller-facing suggestion from the top-10 eligible quotes (PLAN.md §12.6).
pub async fn price_suggestion(st: &AppState, b: &ListingBundle) -> ApiResult<kabados_pricing::suggest::Suggestion> {
    let fp = fast_path(st, b, DEFAULT_RING).await?;
    let mids: Vec<Paise> = fp.matches.iter().take(10).map(|m| m.quote.mid()).collect();
    let ri = rate_index_value(st, b).await?;
    Ok(kabados_pricing::suggest::suggest(&mids, ri))
}

/// Values the lot at published Rate Index medians (KG items only).
async fn rate_index_value(st: &AppState, b: &ListingBundle) -> ApiResult<Option<Paise>> {
    let entries = st.db.rate_index(&b.listing.data.approx.city, 200).await?;
    if entries.is_empty() {
        return Ok(None);
    }
    let mut total = Paise(0);
    let mut any = false;
    for i in &b.items {
        let Some(w) = i.data.est_weight_g else { continue };
        if let Some(e) = entries
            .iter()
            .find(|e| e.category_id == i.data.category_id && (i.data.grade.as_deref().unwrap_or("any") == e.grade || e.grade == "any"))
        {
            total += Paise::per_kg(e.median_paise_per_kg, w).unwrap_or_default();
            any = true;
        }
    }
    Ok(any.then_some(total))
}
