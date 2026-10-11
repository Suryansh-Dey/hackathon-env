//! Vendor rate cards, feed and matches (PLAN.md §7.2, §7.3).

use super::{now, parse, split_action};
use crate::access::*;
use crate::ai::{RulesDraft, RulesRequest};
use crate::auth::Me;
use crate::error::{ApiError, ApiResult};
use crate::matching::{pickup_ctx, quote_items, quote_with};
use crate::state::AppState;
use crate::views::ListingSummary;
use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, FixedOffset};
use kabados_authz::{Action, Resource};
use kabados_db::{Query as DbQuery, Table, keys};
use kabados_domain::Paise;
use kabados_domain::geo;
use kabados_domain::ids::*;
use kabados_domain::ladder::LadderLevel;
use kabados_domain::ruleset::*;
use kabados_domain::taxonomy::glob_matches;
use kabados_pricing::{Quote, QuoteMode};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use utoipa::ToSchema;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/vendor/rule-sets", get(get_rule_sets).post(save_draft))
        .route("/v1/vendor/rule-sets:draft-from-text", post(draft_from_text))
        .route("/v1/vendor/rule-sets:bulk-change", post(bulk_change))
        .route("/v1/vendor/rule-sets/{id}/versions/{seg}", post(version_action))
        .route("/v1/vendor/feed", get(feed))
        .route("/v1/vendor/matches", get(matches))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RuleSetsView {
    pub draft: Option<RuleSet>,
    pub draft_issues: Vec<ValidationIssue>,
    pub published: Vec<RuleSet>,
}

#[utoipa::path(get, path = "/v1/vendor/rule-sets", tag = "vendor", responses((status = 200, body = RuleSetsView)))]
pub async fn get_rule_sets(State(st): State<AppState>, me: Me) -> ApiResult<Json<RuleSetsView>> {
    let v = vendor_of(&st, &me).await?;
    let tax = st.taxonomy().await?;
    let draft = st.db.rule_draft(me.id()).await?.map(|r| r.data);
    let issues = draft.as_ref().map(|d| d.validate(&tax, v.data.hazardous_auth_valid(now()))).unwrap_or_default();
    let published = st.db.rule_set_history(me.id(), 10).await?.into_iter().map(|r| r.data).collect();
    Ok(Json(RuleSetsView { draft, draft_issues: issues, published }))
}

/// Rule set body as edited in the form (IDs, version and status are server-set).
#[derive(Debug, Deserialize, ToSchema)]
pub struct RuleSetInput {
    #[schema(value_type = Option<String>, format = DateTime)]
    pub effective_from: Option<DateTime<FixedOffset>>,
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
}

fn yes() -> bool {
    true
}

async fn next_version(st: &AppState, vendor: &UserId) -> ApiResult<(RuleSetId, u32)> {
    Ok(match st.db.latest_rule_set(vendor).await? {
        Some(r) => (r.data.rule_set_id, r.data.version + 1),
        None => (RuleSetId::new(), 1),
    })
}

fn ist_now() -> DateTime<FixedOffset> {
    now().with_timezone(&FixedOffset::east_opt(5 * 3600 + 1800).expect("IST offset"))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DraftView {
    pub draft: RuleSet,
    pub issues: Vec<ValidationIssue>,
}

/// Saves the form editor's draft; validation issues are returned, not blocking.
#[utoipa::path(post, path = "/v1/vendor/rule-sets", tag = "vendor", request_body = RuleSetInput, responses((status = 200, body = DraftView)))]
pub async fn save_draft(State(st): State<AppState>, me: Me, Json(b): Json<RuleSetInput>) -> ApiResult<Json<DraftView>> {
    let v = vendor_of(&st, &me).await?;
    let (rule_set_id, version) = next_version(&st, me.id()).await?;
    let draft = RuleSet {
        rule_set_id,
        vendor_id: me.id().clone(),
        version,
        status: RuleSetStatus::Draft,
        effective_from: b.effective_from.unwrap_or_else(ist_now),
        service_area: b.service_area,
        schedule: b.schedule,
        minimums: b.minimums,
        lines: b.lines,
        modifiers: b.modifiers,
        exclusions: b.exclusions,
        accept_partial_lots: b.accept_partial_lots,
        source: Some("form".into()),
    };
    let issues = draft.validate(&*st.taxonomy().await?, v.data.hazardous_auth_valid(now()));
    st.db.put_rule_draft(me.id(), draft.clone()).await?;
    Ok(Json(DraftView { draft, issues }))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DraftFromText {
    /// e.g. "copper 650, brass 400, paper 14 above 5 kilo, no glass"
    pub instruction: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct BulkChange {
    /// Category glob, e.g. `metal.*`.
    pub category_glob: String,
    /// Percentage change, e.g. 3 or -5.
    pub percent: i64,
    #[schema(value_type = Option<String>, format = DateTime)]
    pub effective_from: Option<DateTime<FixedOffset>>,
}

async fn draft_base(st: &AppState, me: &Me) -> ApiResult<(Option<RuleSet>, RuleSetId, u32)> {
    let current = match st.db.rule_draft(me.id()).await? {
        Some(d) => Some(d.data),
        None => st.db.latest_rule_set(me.id()).await?.map(|r| r.data),
    };
    let (rid, version) = next_version(st, me.id()).await?;
    Ok((current, rid, version))
}

/// Drafts rules from a typed instruction with the rules-authoring agent
/// (PLAN.md §11.7). Nothing publishes until the vendor taps Publish.
#[utoipa::path(post, path = "/v1/vendor/rule-sets:draft-from-text", tag = "vendor", request_body = DraftFromText, responses((status = 200, body = RulesDraft)))]
pub async fn draft_from_text(State(st): State<AppState>, me: Me, Json(b): Json<DraftFromText>) -> ApiResult<Json<RulesDraft>> {
    let v = vendor_of(&st, &me).await?;
    let tax = st.taxonomy().await?;
    let (current, rid, version) = draft_base(&st, &me).await?;
    let text = b.instruction.trim();
    if text.is_empty() || text.chars().count() > 1000 {
        return Err(ApiError::bad("type the price changes (up to 1,000 characters)"));
    }
    if !st.db.rate_limit("rules-ai", me.id().as_str(), 3600, 30).await? {
        return Err(ApiError::RateLimited);
    }
    let ri = st.db.rate_index(&v.data.city, 200).await?.into_iter().map(|e| (e.category_id, e.median_paise_per_kg.0)).collect();
    let mut d = st
        .ai
        .draft_rules(RulesRequest {
            instruction: text.to_owned(),
            current,
            vendor_id: me.id().clone(),
            hazardous_authorized: v.data.hazardous_auth_valid(now()),
            rate_index_paise_per_kg: ri,
            taxonomy: tax.clone(),
            yard: Some(v.data.yard_location),
        })
        .await
        .map_err(|e| ApiError::Unavailable(e.to_string()))?;
    d.draft.rule_set_id = rid;
    d.draft.version = version;
    d.draft.vendor_id = me.id().clone();
    d.draft.status = RuleSetStatus::Draft;
    d.draft.source = Some("text".into());
    d.issues = d.draft.validate(&tax, v.data.hazardous_auth_valid(now()));
    st.db.put_rule_draft(me.id(), d.draft.clone()).await?;
    Ok(Json(d))
}

/// Bulk change, e.g. "all metals +3% from tomorrow 9am" (PLAN.md §7.2).
#[utoipa::path(post, path = "/v1/vendor/rule-sets:bulk-change", tag = "vendor", request_body = BulkChange, responses((status = 200, body = DraftView)))]
pub async fn bulk_change(State(st): State<AppState>, me: Me, Json(b): Json<BulkChange>) -> ApiResult<Json<DraftView>> {
    let v = vendor_of(&st, &me).await?;
    let tax = st.taxonomy().await?;
    let (current, rid, version) = draft_base(&st, &me).await?;
    if !kabados_domain::taxonomy::is_valid_glob(&b.category_glob) || !(-50..=50).contains(&b.percent) {
        return Err(ApiError::bad("use a category like metal.* and a change between −50% and +50%"));
    }
    let mut draft = current.ok_or_else(|| ApiError::bad("create a rate card first"))?;
    let mut changed = 0;
    for l in draft.lines.iter_mut().filter(|l| glob_matches(&b.category_glob, &l.category_id)) {
        for p in [&mut l.price_per_kg_paise, &mut l.price_per_piece_paise].into_iter().flatten() {
            *p = (Paise(*p) + Paise(*p).pct(b.percent)).0.max(1);
        }
        changed += 1;
    }
    if changed == 0 {
        return Err(ApiError::bad("no lines match that category"));
    }
    draft.rule_set_id = rid;
    draft.version = version;
    draft.status = RuleSetStatus::Draft;
    draft.effective_from = b.effective_from.unwrap_or_else(ist_now);
    draft.source = Some(format!("bulk:{}:{:+}%", b.category_glob, b.percent));
    let issues = draft.validate(&tax, v.data.hazardous_auth_valid(now()));
    st.db.put_rule_draft(me.id(), draft.clone()).await?;
    Ok(Json(DraftView { draft, issues }))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SimulationRow {
    pub listing: ListingSummary,
    pub current: Option<Quote>,
    pub draft: Quote,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Simulation {
    pub rows: Vec<SimulationRow>,
    pub eligible_now: usize,
    pub eligible_with_draft: usize,
}

/// `POST /v1/vendor/rule-sets/{id}/versions/{v}:simulate | :publish`.
#[utoipa::path(post, path = "/v1/vendor/rule-sets/{id}/versions/{v}:publish", tag = "vendor", responses((status = 200, body = RuleSet)))]
pub async fn version_action(
    State(st): State<AppState>,
    me: Me,
    Path((id, seg)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let rid: RuleSetId = parse(&id, "rule set id")?;
    let (ver, action) = split_action(&seg);
    let ver: u32 = parse(ver, "version")?;
    let mut v = vendor_of(&st, &me).await?;
    let draft = st.db.rule_draft(me.id()).await?.ok_or(ApiError::NotFound("draft"))?.data;
    if draft.rule_set_id != rid || draft.version != ver {
        return Err(ApiError::conflict("that draft version is no longer current"));
    }
    let tax = st.taxonomy().await?;
    match action {
        Some("simulate") => {
            let current = st.db.latest_rule_set(me.id()).await?.map(|r| r.data);
            let cells = geo::grid_disk(&geo::cell_parent(&v.data.yard_h3_r8, geo::FEED_RES)?, 2)?;
            let mut rows = Vec::new();
            for c in cells {
                let (ls, _) = st.db.feed(&c, None, 20).await?;
                for l in ls {
                    if rows.len() >= 20 {
                        break;
                    }
                    let Some(b) = st.db.listing_bundle(&l.data.id, None).await? else { continue };
                    let items = quote_items(&b.item_data());
                    let pickup = pickup_ctx(&b.listing.data, &b.version.data);
                    let mut d = draft.clone();
                    d.status = RuleSetStatus::Published;
                    let dq = quote_with(&tax, &items, &pickup, &v.data, &d, None, &BTreeMap::new(), QuoteMode::Estimate);
                    let cq = current
                        .as_ref()
                        .map(|c| quote_with(&tax, &items, &pickup, &v.data, c, None, &BTreeMap::new(), QuoteMode::Estimate));
                    rows.push(SimulationRow { listing: ListingSummary::from(&b.listing.data), current: cq, draft: dq });
                }
            }
            let sim = Simulation {
                eligible_now: rows.iter().filter(|r| r.current.as_ref().is_some_and(|q| q.eligible)).count(),
                eligible_with_draft: rows.iter().filter(|r| r.draft.eligible).count(),
                rows,
            };
            Ok(Json(serde_json::to_value(sim).map_err(anyhow::Error::from)?))
        }
        Some("publish") => {
            let issues = draft.validate(&tax, v.data.hazardous_auth_valid(now()));
            if !issues.is_empty() {
                return Err(ApiError::Invalid(issues));
            }
            let rec = st.db.publish_rule_set(&mut v, draft).await?;
            Ok(Json(serde_json::to_value(rec.data).map_err(anyhow::Error::from)?))
        }
        _ => Err(ApiError::NotFound("action")),
    }
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct FeedQuery {
    /// Comma-separated category globs.
    pub categories: Option<String>,
    pub min_value: Option<i64>,
    pub ladder: Option<LadderLevel>,
    pub pooled: Option<bool>,
    pub hazardous: Option<bool>,
    /// Ring of r7 cells around the yard (0–3).
    pub ring: Option<u32>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FeedItem {
    pub listing: ListingSummary,
    pub quote: Option<Quote>,
    pub distance_m: f64,
}

/// Browse all published listings near the yard (approximate locations only).
#[utoipa::path(get, path = "/v1/vendor/feed", tag = "vendor", params(FeedQuery), responses((status = 200, body = Vec<FeedItem>)))]
pub async fn feed(State(st): State<AppState>, me: Me, Query(q): Query<FeedQuery>) -> ApiResult<Json<Vec<FeedItem>>> {
    let v = vendor_of(&st, &me).await?;
    let principal = vendor_principal(&v.data);
    let rules = st.db.latest_rule_set(me.id()).await?.map(|r| r.data);
    let tax = st.taxonomy().await?;
    let center = geo::cell_parent(&v.data.yard_h3_r8, geo::FEED_RES)?;
    let globs: Vec<String> =
        q.categories.map(|c| c.split(',').map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect()).unwrap_or_default();
    let mut out = Vec::new();
    for cell in geo::grid_disk(&center, q.ring.unwrap_or(1).min(3))? {
        let (ls, _) = st.db.feed(&cell, None, 50).await?;
        for l in ls {
            let l = l.data;
            if l.seller_id == *me.id() || l.status != kabados_domain::listing::ListingStatus::Published {
                continue;
            }
            if !globs.is_empty() && !l.categories.iter().any(|c| globs.iter().any(|g| glob_matches(g, c))) {
                continue;
            }
            if q.ladder.is_some_and(|min| l.min_ladder < min)
                || q.pooled.is_some_and(|p| p != l.drive_id.is_some())
                || q.hazardous.is_some_and(|h| h != l.contains_hazardous)
            {
                continue;
            }
            if !allowed(&st, &principal, Action::ViewListing, &Resource::Listing(listing_attrs(&l, None, None, None))).await? {
                continue;
            }
            let quote = match (&rules, st.db.listing_bundle(&l.id, None).await?) {
                (Some(rs), Some(b)) => Some(quote_with(
                    &tax,
                    &quote_items(&b.item_data()),
                    &pickup_ctx(&b.listing.data, &b.version.data),
                    &v.data,
                    rs,
                    None,
                    &BTreeMap::new(),
                    QuoteMode::Estimate,
                )),
                _ => None,
            };
            if let (Some(min), Some(qv)) = (q.min_value, &quote)
                && qv.quote_high.0 < min
            {
                continue;
            }
            out.push(FeedItem { distance_m: l.approx.center.distance_m(&v.data.yard_location), listing: ListingSummary::from(&l), quote });
        }
    }
    out.sort_by_key(|i| std::cmp::Reverse(i.listing.published_at));
    Ok(Json(out))
}

/// A first-look lead written by the matching worker (PLAN.md §7.3).
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct MatchLead {
    pub listing_id: ListingId,
    pub ver: u32,
    pub quote_low: Paise,
    pub quote_high: Paise,
    pub coverage_permille: u32,
    pub distance_m: i64,
    pub first_look_until: Option<chrono::DateTime<chrono::Utc>>,
    pub at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MatchView {
    pub lead: MatchLead,
    pub listing: ListingSummary,
}

#[utoipa::path(get, path = "/v1/vendor/matches", tag = "vendor", responses((status = 200, body = Vec<MatchView>)))]
pub async fn matches(State(st): State<AppState>, me: Me) -> ApiResult<Json<Vec<MatchView>>> {
    vendor_of(&st, &me).await?;
    let (leads, _) =
        st.db.query_recs::<MatchLead>(DbQuery::new(Table::Core, keys::user(me.id())).begins_with("MATCH#").reverse().limit(50)).await?;
    let mut out = Vec::new();
    for lead in leads {
        if let Some(l) = st.db.listing(&lead.data.listing_id).await?
            && l.data.status == kabados_domain::listing::ListingStatus::Published
            && l.data.current_version == lead.data.ver
        {
            out.push(MatchView { listing: ListingSummary::from(&l.data), lead: lead.data });
        }
    }
    Ok(Json(out))
}
