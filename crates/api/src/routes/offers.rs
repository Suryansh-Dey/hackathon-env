//! Offers (PLAN.md §4.3, §6.2, §7.4).

use super::{id_action, now, parse};
use crate::access::*;
use crate::auth::Me;
use crate::error::{ApiError, ApiResult};
use crate::matching::{pickup_ctx, quote_items, quote_with};
use crate::routes::listings::load_bundle;
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Duration;
use kabados_authz::{Action, Resource};
use kabados_db::Record;
use kabados_db::repo::{Notification, NotificationKind};
use kabados_domain::Paise;
use kabados_domain::deal::*;
use kabados_domain::ids::*;
use kabados_domain::listing::ListingStatus;
use kabados_domain::offer::*;
use kabados_domain::state::StateMachine;
use kabados_pricing::{ENGINE_VERSION, Quote, QuoteMode};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use utoipa::ToSchema;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/listings/{id}/offers", get(listing_offers).post(make_offer))
        .route("/v1/listings/{id}/my-quote", get(my_quote))
        .route("/v1/offers/{seg}", post(offer_action))
        .route("/v1/vendor/offers", get(vendor_offers))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OfferView {
    #[serde(flatten)]
    pub offer: Offer,
    pub vendor_rating: f64,
    /// Quote meets the seller's asking price (highlighted to the seller, PLAN.md §12.6).
    pub meets_ask: bool,
    pub stale: bool,
}

/// Seller: all offers on the listing. Vendor: their own offers on it.
#[utoipa::path(get, path = "/v1/listings/{id}/offers", tag = "offers", responses((status = 200, body = Vec<OfferView>)))]
pub async fn listing_offers(State(st): State<AppState>, me: Me, Path(id): Path<String>) -> ApiResult<Json<Vec<OfferView>>> {
    let id: ListingId = parse(&id, "listing id")?;
    let b = load_bundle(&st, &id).await?;
    let l = &b.listing.data;
    let (deal, job, drive) = listing_context(&st, l).await?;
    let res = Resource::Listing(listing_attrs(l, deal.as_ref(), job.as_ref(), drive.as_ref()));
    let all = st.db.offers_for_listing(&id).await?;
    let is_seller = allowed(&st, &seller(me.id()), Action::ViewOffers, &res).await?;
    let ask = b.version.data.asking_price;
    let mut out = Vec::new();
    for o in all.into_iter().map(|r| r.data).filter(|o| is_seller || &o.vendor_id == me.id()) {
        let rating = st.db.vendor(&o.vendor_id).await?.map(|v| v.data.rating.stars()).unwrap_or(0.0);
        out.push(OfferView {
            meets_ask: ask.is_some_and(|a| o.total_low >= a),
            stale: o.listing_version != l.current_version,
            vendor_rating: rating,
            offer: o,
        });
    }
    if !is_seller && out.is_empty() && st.db.vendor(me.id()).await?.is_none() {
        return Err(ApiError::NotFound("listing"));
    }
    out.sort_by_key(|o| std::cmp::Reverse(o.offer.total_high));
    Ok(Json(out))
}

async fn vendor_quote(
    st: &AppState,
    me: &Me,
    id: &ListingId,
    overrides: &BTreeMap<ItemId, Rate>,
) -> ApiResult<(Record<kabados_domain::user::VendorProfile>, kabados_domain::ruleset::RuleSet, Quote, kabados_db::repo::ListingBundle)> {
    let v = vendor_of(st, me).await?;
    let b = load_bundle(st, id).await?;
    let l = &b.listing.data;
    let (deal, job, drive) = listing_context(st, l).await?;
    ensure(
        st,
        &vendor_principal(&v.data),
        Action::ViewListing,
        &Resource::Listing(listing_attrs(l, deal.as_ref(), job.as_ref(), drive.as_ref())),
    )
    .await?;
    let rs = st.db.latest_rule_set(me.id()).await?.ok_or_else(|| ApiError::bad("publish your rate card first"))?.data;
    let tax = st.taxonomy().await?;
    let pickup = pickup_ctx(l, &b.version.data);
    let road = st.ix.routes.leg(v.data.yard_location, pickup.location).await.ok().flatten().map(|l| l.distance_m);
    let q = quote_with(&tax, &quote_items(&b.item_data()), &pickup, &v.data, &rs, road, overrides, QuoteMode::Estimate);
    Ok((v, rs, q, b))
}

/// The vendor's deterministic quote from their rate card (offer pre-fill).
#[utoipa::path(get, path = "/v1/listings/{id}/my-quote", tag = "offers", responses((status = 200, body = Quote)))]
pub async fn my_quote(State(st): State<AppState>, me: Me, Path(id): Path<String>) -> ApiResult<Json<Quote>> {
    let id: ListingId = parse(&id, "listing id")?;
    Ok(Json(vendor_quote(&st, &me, &id, &BTreeMap::new()).await?.2))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct LineEdit {
    pub item_id: ItemId,
    pub rate: Rate,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct MakeOffer {
    /// Per-line rate edits away from the rate card (logged).
    #[serde(default)]
    pub edits: Vec<LineEdit>,
    pub slot: Option<Slot>,
    pub pickup_method: PickupMethod,
    pub payment_mode: PaymentMode,
    /// Hours the offer stays open (1–72).
    pub valid_hours: Option<i64>,
    pub note: Option<String>,
}

#[utoipa::path(post, path = "/v1/listings/{id}/offers", tag = "offers", request_body = MakeOffer, responses((status = 201, body = Offer)))]
pub async fn make_offer(
    State(st): State<AppState>,
    me: Me,
    Path(id): Path<String>,
    Json(b): Json<MakeOffer>,
) -> ApiResult<(StatusCode, Json<Offer>)> {
    let id: ListingId = parse(&id, "listing id")?;
    let overrides: BTreeMap<ItemId, Rate> = b.edits.iter().map(|e| (e.item_id.clone(), e.rate)).collect();
    for r in overrides.values() {
        let p = match r {
            Rate::Kg { paise_per_kg } => *paise_per_kg,
            Rate::Piece { paise_per_piece } => *paise_per_piece,
        };
        if p.0 <= 0 || p.0 > kabados_domain::ruleset::MAX_PRICE_PAISE {
            return Err(ApiError::bad("rates must be positive"));
        }
    }
    let (v, rs, q, bundle) = vendor_quote(&st, &me, &id, &overrides).await?;
    let l = &bundle.listing.data;
    let (deal, job, drive) = listing_context(&st, l).await?;
    ensure(
        &st,
        &vendor_principal(&v.data),
        Action::MakeOffer,
        &Resource::Listing(listing_attrs(l, deal.as_ref(), job.as_ref(), drive.as_ref())),
    )
    .await?;
    if l.first_look_until.is_some_and(|t| t > now()) {
        // First-look window: only vendors notified by matching may offer early (PLAN.md §7.3).
        let matched = st
            .db
            .match_result(&l.id, l.current_version)
            .await?
            .map(|m| m.data.result.to_string().contains(me.id().as_str()))
            .unwrap_or(false);
        if !matched {
            return Err(ApiError::conflict("this listing is in its first-look window for matched vendors; try again shortly"));
        }
    }
    if q.lines.is_empty() {
        return Err(ApiError::bad("your rate card doesn't cover any item in this listing"));
    }
    if let Some(s) = &b.slot
        && kabados_domain::ruleset::parse_window(&s.window).is_none()
    {
        return Err(ApiError::bad("slot window must be HH:MM-HH:MM"));
    }
    let lines: Vec<OfferLine> = q
        .lines
        .iter()
        .map(|ql| OfferLine {
            item_id: ql.item_id.clone(),
            category_id: ql.category_id.clone(),
            grade: ql.grade.clone(),
            rate: ql.rate,
            modifier_pct: ql.modifier_pct,
            value_low: ql.value_low,
            value_high: ql.value_high,
            edited: ql.rule_line.is_none(),
        })
        .collect();
    let existing =
        st.db.offers_for_listing(&id).await?.into_iter().find(|o| o.data.vendor_id == *me.id() && o.data.status == OfferStatus::Active);
    let hours = b.valid_hours.unwrap_or(24).clamp(1, 72);
    let offer = Offer {
        id: OfferId::new(),
        listing_id: id.clone(),
        listing_version: l.current_version,
        vendor_id: me.id().clone(),
        vendor_name: v.data.business_name.clone(),
        status: OfferStatus::Active,
        lines,
        adjustments: q.adjustments.clone(),
        total_low: q.quote_low,
        total_high: q.quote_high,
        covered_weight_low: q.covered_weight_low,
        covered_weight_high: q.covered_weight_high,
        distance_m: q.distance_m,
        valid_until: now() + Duration::hours(hours),
        slot: b.slot,
        pickup_method: b.pickup_method,
        payment_mode: b.payment_mode,
        rule_set_version: Some(rs.version),
        engine_version: ENGINE_VERSION.into(),
        supersedes: existing.as_ref().map(|e| e.data.id.clone()),
        seller_counter: None,
        note: b.note.map(|n| n.chars().take(500).collect()),
        created_at: now(),
    };
    let note = Notification::new(
        l.seller_id.clone(),
        NotificationKind::NewOffer,
        "New offer",
        format!("{} offered {} – {}", offer.vendor_name, offer.total_low, offer.total_high),
    )
    .link(format!("/listing?id={id}"));
    let mut existing = existing;
    let rec = st.db.create_offer(offer, existing.as_mut(), vec![note]).await?;
    Ok((StatusCode::CREATED, Json(rec.data)))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CounterOffer {
    pub total: Paise,
    pub note: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema, Default)]
pub struct AcceptOffer {
    /// Optional slot override chosen by the seller from the vendor's proposal.
    pub slot: Option<Slot>,
}

/// `POST /v1/offers/{id}:accept | :counter | :reject | :withdraw`.
#[utoipa::path(post, path = "/v1/offers/{id}:accept", tag = "offers", request_body = AcceptOffer, responses((status = 200, body = Object)))]
pub async fn offer_action(
    State(st): State<AppState>,
    me: Me,
    Path(seg): Path<String>,
    body: Option<Json<serde_json::Value>>,
) -> ApiResult<Json<serde_json::Value>> {
    let (oid, action): (OfferId, String) = id_action(&seg, "offer id")?;
    let mut offer = st.db.offer(&oid).await?.ok_or(ApiError::NotFound("offer"))?;
    let mut bundle = load_bundle(&st, &offer.data.listing_id).await?;
    let (deal, job, drive) = listing_context(&st, &bundle.listing.data).await?;
    let lattrs = listing_attrs(&bundle.listing.data, deal.as_ref(), job.as_ref(), drive.as_ref());
    let ores = Resource::Offer(offer_attrs(&offer.data, lattrs));
    let body = body.map(|b| b.0).unwrap_or(serde_json::Value::Null);
    match action.as_str() {
        "accept" => {
            ensure(&st, &seller(me.id()), Action::AcceptOffer, &ores).await?;
            let req: AcceptOffer = if body.is_null() {
                AcceptOffer::default()
            } else {
                serde_json::from_value(body).map_err(|e| ApiError::bad(e.to_string()))?
            };
            if offer.data.valid_until < now() {
                return Err(ApiError::conflict("this offer has expired"));
            }
            if offer.data.listing_version != bundle.listing.data.current_version {
                return Err(ApiError::conflict("the listing changed after this offer; ask the vendor to update it"));
            }
            let d = accept(&st, &mut bundle.listing, &mut offer, req.slot).await?;
            Ok(Json(serde_json::json!({"deal_id": d})))
        }
        "counter" => {
            ensure(&st, &seller(me.id()), Action::CounterOffer, &ores).await?;
            let c: CounterOffer = serde_json::from_value(body).map_err(|e| ApiError::bad(e.to_string()))?;
            if offer.data.seller_counter.is_some() {
                return Err(ApiError::conflict("you can counter an offer once"));
            }
            if c.total.0 <= 0 {
                return Err(ApiError::bad("counter must be positive"));
            }
            offer.data.seller_counter =
                Some(SellerCounter { total: c.total, note: c.note.map(|n| n.chars().take(300).collect()), at: now() });
            let n = Notification::new(
                offer.data.vendor_id.clone(),
                NotificationKind::CounterOffer,
                "Counter-offer",
                format!("The seller asks {} for {}", c.total, bundle.listing.data.title),
            )
            .link(format!("/listing?id={}", offer.data.listing_id));
            st.db.save_offer(&mut offer, vec![n]).await?;
            Ok(Json(serde_json::to_value(&offer.data).map_err(anyhow::Error::from)?))
        }
        "reject" => {
            ensure(&st, &seller(me.id()), Action::RejectOffer, &ores).await?;
            offer.data.status = offer.data.status.transition(OfferStatus::Rejected)?;
            st.db.save_offer(&mut offer, vec![]).await?;
            Ok(Json(serde_json::to_value(&offer.data).map_err(anyhow::Error::from)?))
        }
        "withdraw" => {
            let v = vendor_of(&st, &me).await?;
            ensure(&st, &vendor_principal(&v.data), Action::WithdrawOffer, &ores).await?;
            offer.data.status = offer.data.status.transition(OfferStatus::Withdrawn)?;
            st.db.save_offer(&mut offer, vec![]).await?;
            Ok(Json(serde_json::to_value(&offer.data).map_err(anyhow::Error::from)?))
        }
        _ => Err(ApiError::NotFound("action")),
    }
}

/// Accepts an offer: one transaction reserves the listing, accepts the offer,
/// creates the deal and its index items (PLAN.md §16.5); lines follow.
pub async fn accept(
    st: &AppState,
    listing: &mut Record<kabados_domain::listing::Listing>,
    offer: &mut Record<Offer>,
    slot: Option<Slot>,
) -> ApiResult<DealId> {
    listing.data.status = listing.data.status.transition(ListingStatus::Reserved)?;
    offer.data.status = offer.data.status.transition(OfferStatus::Accepted)?;
    let deal_id = DealId::new();
    listing.data.accepted_offer_id = Some(offer.data.id.clone());
    listing.data.deal_id = Some(deal_id.clone());
    listing.data.updated_at = now();
    let fee = platform_fee(offer.data.total_high, st.cfg.platform_fee_bps);
    let deal = Deal {
        id: deal_id.clone(),
        listing_id: listing.data.id.clone(),
        listing_version: offer.data.listing_version,
        offer_id: offer.data.id.clone(),
        seller_id: listing.data.seller_id.clone(),
        vendor_id: offer.data.vendor_id.clone(),
        transporter_id: None,
        job_id: None,
        status: DealStatus::Created,
        payment_mode: offer.data.payment_mode,
        pickup_method: offer.data.pickup_method,
        slot: slot.or_else(|| offer.data.slot.clone()),
        rule_set_version: offer.data.rule_set_version,
        engine_version: offer.data.engine_version.clone(),
        agreed_total_low: offer.data.total_low,
        agreed_total_high: offer.data.total_high,
        final_amount: None,
        transporter_fare: None,
        platform_fee: fee,
        payment_order_id: None,
        direct_payment_marked_at: None,
        handover_otp_hash: None,
        handover_at: None,
        delivered_at: None,
        receipt_id: None,
        workflow_execution_arn: None,
        cancel_reason: None,
        created_at: now(),
        updated_at: now(),
    };
    let n = Notification::new(
        offer.data.vendor_id.clone(),
        NotificationKind::OfferAccepted,
        "Offer accepted",
        format!("Your offer for {} was accepted", listing.data.title),
    )
    .link(format!("/deal?id={deal_id}"));
    st.db.accept_offer(listing, offer, deal, vec![n]).await?;
    let items: BTreeMap<ItemId, u32> = st
        .db
        .listing_bundle(&listing.data.id, Some(offer.data.listing_version))
        .await?
        .map(|b| b.items.iter().map(|i| (i.data.id.clone(), i.data.quantity)).collect())
        .unwrap_or_default();
    let ops = offer
        .data
        .lines
        .iter()
        .enumerate()
        .map(|(n, l)| {
            st.db.line_op(
                &deal_id,
                &DealLine {
                    n: n as u32 + 1,
                    item_id: l.item_id.clone(),
                    category_id: l.category_id.clone(),
                    grade: l.grade.clone(),
                    quantity: items.get(&l.item_id).copied().unwrap_or(1),
                    rate: l.rate,
                    modifier_pct: l.modifier_pct,
                    status: LineStatus::Pending,
                    weighed_g: None,
                    amount: None,
                    regraded_from: None,
                },
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    st.db.write_ops(ops).await?;
    Ok(deal_id)
}

/// Platform fee in basis points of the amount, rounded once.
pub fn platform_fee(amount: Paise, bps: i64) -> Paise {
    Paise(kabados_domain::money::div_round(amount.0 as i128 * bps as i128, 10_000) as i64)
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct VendorOffersQuery {
    pub status: Option<OfferStatus>,
    pub cursor: Option<String>,
}

#[utoipa::path(get, path = "/v1/vendor/offers", tag = "offers", params(VendorOffersQuery), responses((status = 200, body = Vec<Offer>)))]
pub async fn vendor_offers(State(st): State<AppState>, me: Me, Query(q): Query<VendorOffersQuery>) -> ApiResult<Json<Vec<Offer>>> {
    vendor_of(&st, &me).await?;
    let (recs, _) = st.db.vendor_offers(me.id(), q.status, q.cursor).await?;
    Ok(Json(recs.into_iter().map(|r| r.data).collect()))
}
