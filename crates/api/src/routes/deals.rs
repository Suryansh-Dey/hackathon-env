//! Deals: pickup, weighing, confirmation, payment (Mode B), handover,
//! delivery, settlement, disputes and ratings (PLAN.md §4.4, §6.2, §13).

use super::{id_action, now, parse, split_action};
use crate::access::*;
use crate::auth::Me;
use crate::error::{ApiError, ApiResult};
use crate::settle;
use crate::state::AppState;
use crate::views::*;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Duration;
use kabados_authz::{Action, Principal, Resource};
use kabados_db::Record;
use kabados_db::repo::{DealIndex, Notification, NotificationKind};
use kabados_domain::deal::*;
use kabados_domain::events::DomainEvent;
use kabados_domain::ids::*;
use kabados_domain::job::{JobEventKind, JobStatus};
use kabados_domain::offer::{PaymentMode, PickupMethod, Rate, Slot};
use kabados_domain::state::StateMachine;
use kabados_domain::{Grams, Paise};
use kabados_integrations::blob::PresignedUpload;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/deals", get(my_deals))
        .route("/v1/deals/{id}", get(get_deal).post(deal_action))
        .route("/v1/deals/{id}/weighings", post(weigh))
        .route("/v1/deals/{id}/lines/{seg}", post(line_action))
        .route("/v1/deals/{id}/disputes", post(open_dispute))
        .route("/v1/deals/{id}/ratings", post(rate))
        .route("/v1/deals/{id}/receipt", get(get_receipt))
        .route("/v1/deals/{id}/images:presign", post(presign_deal_image))
        .route("/v1/deals/{id}/scale-reading", post(scale_reading))
}

/// The caller's role on this deal, as a Cedar principal.
pub async fn deal_principal(st: &AppState, me: &Me, d: &Deal) -> ApiResult<(Principal, Viewer)> {
    if d.seller_id == *me.id() {
        return Ok((seller(me.id()), Viewer::Seller));
    }
    if d.vendor_id == *me.id() {
        return Ok((vendor_principal(&vendor_of(st, me).await?.data), Viewer::Vendor));
    }
    if d.transporter_id.as_ref() == Some(me.id()) {
        return Ok((transporter_principal(&transporter_of(st, me).await?.data), Viewer::Transporter));
    }
    Err(ApiError::NotFound("deal"))
}

async fn load(st: &AppState, me: &Me, id: &str) -> ApiResult<(Record<Deal>, Principal, Viewer)> {
    let id: DealId = parse(id, "deal id")?;
    let d = st.db.deal(&id).await?.ok_or(ApiError::NotFound("deal"))?;
    let (p, v) = deal_principal(st, me, &d.data).await?;
    ensure(st, &p, Action::ViewDeal, &Resource::Deal(deal_attrs(&d.data))).await?;
    Ok((d, p, v))
}

#[utoipa::path(get, path = "/v1/deals", tag = "deals", responses((status = 200, body = Vec<DealSummary>)))]
pub async fn my_deals(State(st): State<AppState>, me: Me) -> ApiResult<Json<Vec<DealSummary>>> {
    let (idx, _) = st.db.user_deals(me.id(), None, None).await?;
    let mut out = Vec::new();
    for i in idx {
        if let Some(d) = st.db.deal(&i.deal_id).await? {
            let title = st.db.listing(&d.data.listing_id).await?.map(|l| l.data.title).unwrap_or_default();
            out.push(DealSummary {
                index: i,
                title,
                status: d.data.status,
                agreed_total_high: d.data.agreed_total_high,
                final_amount: d.data.final_amount,
                slot: d.data.slot,
            });
        }
    }
    out.sort_by_key(|d| std::cmp::Reverse(d.index.created_at));
    Ok(Json(out))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DealSummary {
    #[schema(value_type = Object)]
    pub index: DealIndex,
    pub title: String,
    pub status: DealStatus,
    pub agreed_total_high: Paise,
    pub final_amount: Option<Paise>,
    pub slot: Option<Slot>,
}

pub async fn deal_view(st: &AppState, me: &Me, d: &Deal, p: &Principal, viewer: Viewer) -> ApiResult<DealView> {
    let listing = st.db.listing(&d.listing_id).await?.ok_or(ApiError::NotFound("listing"))?;
    let c = st.db.deal_children(&d.id).await?;
    let job = c.jobs.iter().map(|j| j.data.clone()).find(|j| Some(&j.id) == d.job_id.as_ref());
    let lattrs = listing_attrs(&listing.data, Some(d), job.as_ref(), None);
    let exact = if allowed(st, p, Action::ViewExactAddress, &Resource::Listing(lattrs)).await? {
        crate::routes::listings::exact_address(st, &listing.data).await?
    } else {
        None
    };
    let otp = if viewer == Viewer::Seller && matches!(d.status, DealStatus::Paid | DealStatus::PaidDirect) {
        settle::handover_code(st, &d.id).await?
    } else {
        None
    };
    let counterpart = match viewer {
        Viewer::Seller => st.db.vendor(&d.vendor_id).await?.map(|v| v.data.business_name),
        _ => st.db.user(&d.seller_id).await?.and_then(|u| u.data.name),
    };
    let mine = c.ratings.iter().find(|r| r.data.from_user == *me.id()).map(|r| r.data.stars);
    let mut lines: Vec<DealLine> = c.lines.into_iter().map(|r| r.data).collect();
    lines.sort_by_key(|l| l.n);
    Ok(DealView {
        deal: d.clone(),
        viewer,
        listing: ListingSummary::from(&listing.data),
        lines,
        weighings: c.weighings.into_iter().map(|r| r.data).collect(),
        job,
        disputes: c.disputes.into_iter().map(|r| r.data).collect(),
        exact_address: exact,
        handover_otp: otp,
        counterpart_name: counterpart,
        my_rating: mine,
    })
}

#[utoipa::path(get, path = "/v1/deals/{id}", tag = "deals", responses((status = 200, body = DealView)))]
pub async fn get_deal(State(st): State<AppState>, me: Me, Path(id): Path<String>) -> ApiResult<Json<DealView>> {
    let (d, p, v) = load(&st, &me, &id).await?;
    Ok(Json(deal_view(&st, &me, &d.data, &p, v).await?))
}

async fn lines_of(st: &AppState, deal: &DealId) -> ApiResult<Vec<Record<DealLine>>> {
    let mut l = st.db.deal_children(deal).await?.lines;
    l.sort_by_key(|r| r.data.n);
    Ok(l)
}

#[derive(Debug, Deserialize, ToSchema, Default)]
pub struct DealActionBody {
    pub slot: Option<Slot>,
    pub otp: Option<String>,
    pub reason: Option<String>,
    /// For `:no-show`: `SELLER` (seller absent) or `PICKUP` (vendor/transporter absent).
    pub who: Option<String>,
}

/// `POST /v1/deals/{id}:schedule | :arrive | :confirm-weights | :mark-paid-direct |
/// :confirm-received | :handover | :confirm-delivery | :cancel | :no-show`.
#[utoipa::path(post, path = "/v1/deals/{id}:confirm-weights", tag = "deals", request_body = DealActionBody, responses((status = 200, body = DealView)))]
pub async fn deal_action(
    State(st): State<AppState>,
    me: Me,
    Path(seg): Path<String>,
    body: Option<Json<DealActionBody>>,
) -> ApiResult<Json<DealView>> {
    let (id, action) = split_action(&seg);
    let action = action.ok_or_else(|| ApiError::bad("missing :action"))?.to_owned();
    let (mut d, p, viewer) = load(&st, &me, id).await?;
    let b = body.map(|b| b.0).unwrap_or_default();
    let res = Resource::Deal(deal_attrs(&d.data));
    let deal_id = d.data.id.clone();
    let link = format!("/deal?id={deal_id}");
    match action.as_str() {
        "schedule" => {
            ensure(&st, &p, Action::AssignTransporter, &res).await?;
            let slot = b.slot.ok_or_else(|| ApiError::bad("slot is required"))?;
            if kabados_domain::ruleset::parse_window(&slot.window).is_none() {
                return Err(ApiError::bad("slot window must be HH:MM-HH:MM"));
            }
            d.data.slot = Some(slot.clone());
            if d.data.status == DealStatus::Created {
                d.data.status = d.data.status.transition(DealStatus::PickupScheduled)?;
            }
            let n = Notification::new(
                d.data.seller_id.clone(),
                NotificationKind::PickupEta,
                "Pickup scheduled",
                format!("Pickup on {} {}", slot.date, slot.window),
            )
            .link(&link);
            st.db.save_deal_with(&mut d, vec![], vec![], vec![n]).await?;
        }
        "arrive" => {
            // Vendor self-pickup; transporter arrival comes from job events.
            ensure(&st, &p, Action::AssignTransporter, &res).await?;
            if d.data.pickup_method == PickupMethod::Transporter && d.data.job_id.is_some() {
                return Err(ApiError::conflict("the transporter marks arrival from the app"));
            }
            if d.data.status == DealStatus::Created {
                d.data.status = d.data.status.transition(DealStatus::PickupScheduled)?;
            }
            d.data.status = d.data.status.transition(DealStatus::AtPickup)?;
            let n =
                Notification::new(d.data.seller_id.clone(), NotificationKind::Arrived, "Pickup has arrived", "The vendor is at your door.")
                    .link(&link);
            st.db
                .save_deal_with(&mut d, vec![], vec![DomainEvent::ArrivedAtPickup { deal_id: deal_id.clone(), job_id: None }], vec![n])
                .await?;
        }
        "confirm-weights" => {
            ensure(&st, &p, Action::ConfirmWeights, &res).await?;
            let lines = lines_of(&st, &deal_id).await?;
            if lines.iter().any(|l| l.data.status == LineStatus::RegradePending) {
                return Err(ApiError::conflict("accept or remove the re-graded lines first"));
            }
            let mut extra = Vec::new();
            for l in lines {
                let mut l = l.data;
                if l.status == LineStatus::Weighed {
                    l.status = LineStatus::Final;
                    extra.push(st.db.line_op(&deal_id, &l)?);
                }
            }
            d.data.status = d.data.status.transition(DealStatus::SellerConfirmed)?;
            let amount = d.data.final_amount.unwrap_or_default();
            let msg = match d.data.payment_mode {
                PaymentMode::Platform => format!("Seller confirmed {amount}. Request payment to continue."),
                PaymentMode::Direct => format!("Seller confirmed {amount}. Pay the seller and mark it paid."),
            };
            let n = Notification::new(d.data.vendor_id.clone(), NotificationKind::WeightsToConfirm, "Weights confirmed", msg).link(&link);
            st.db.save_deal_with(&mut d, extra, vec![DomainEvent::WeightsConfirmed { deal_id: deal_id.clone(), amount }], vec![n]).await?;
        }
        "mark-paid-direct" => {
            ensure(&st, &p, Action::MarkPaidDirect, &res).await?;
            if d.data.payment_mode != PaymentMode::Direct {
                return Err(ApiError::conflict("this deal is paid through the platform"));
            }
            d.data.direct_payment_marked_at = Some(now());
            let n = Notification::new(
                d.data.seller_id.clone(),
                NotificationKind::PaymentReceived,
                "Payment sent",
                format!("The vendor says they paid {}. Confirm when you receive it.", d.data.final_amount.unwrap_or_default()),
            )
            .link(&link);
            st.db.save_deal_with(&mut d, vec![], vec![], vec![n]).await?;
        }
        "confirm-received" => {
            ensure(&st, &p, Action::ConfirmReceived, &res).await?;
            if d.data.payment_mode != PaymentMode::Direct {
                return Err(ApiError::conflict("this deal is paid through the platform"));
            }
            d.data.status = d.data.status.transition(DealStatus::PaidDirect)?;
            let otp_op = settle::issue_handover(&st, &mut d.data).await?;
            let n = Notification::new(
                d.data.vendor_id.clone(),
                NotificationKind::PaymentReceived,
                "Seller confirmed payment",
                "Ask the seller for the handover code.",
            )
            .link(&link);
            st.db.save_deal_with(&mut d, vec![otp_op], vec![], vec![n]).await?;
        }
        "handover" => {
            ensure(&st, &p, Action::Handover, &res).await?;
            let otp = b.otp.ok_or_else(|| ApiError::bad("enter the seller's handover code"))?;
            if !st.db.rate_limit("handover", deal_id.as_str(), 600, 5).await? {
                return Err(ApiError::RateLimited);
            }
            if !settle::verify_handover(&st, &d.data, &otp) {
                return Err(ApiError::bad("that code is not correct"));
            }
            d.data.status = d.data.status.transition(DealStatus::HandedOver)?;
            d.data.handover_at = Some(now());
            let mut extra = Vec::new();
            if let Some(jid) = &d.data.job_id
                && let Some(mut job) = st.db.job(jid).await?
                && job.data.status == JobStatus::ArrivedPickup
            {
                job.data.status = job.data.status.transition(JobStatus::Loaded)?;
                let ev = kabados_domain::job::JobEvent {
                    kind: JobEventKind::Loaded,
                    at: now(),
                    device_at: None,
                    lat: None,
                    lng: None,
                    image_ids: vec![],
                    note: Some("handover code verified".into()),
                    mock_location: None,
                };
                extra.push(st.db.job_event_op(&deal_id, jid, &ev)?);
                extra.push(st.db.save_job_op(&mut job)?);
            }
            let n = Notification::new(
                d.data.seller_id.clone(),
                NotificationKind::HandoverDone,
                "Handover complete",
                "Thanks! Your Green Receipt will be ready after delivery.",
            )
            .link(&link);
            st.db.save_deal_with(&mut d, extra, vec![DomainEvent::HandoverCompleted { deal_id: deal_id.clone() }], vec![n]).await?;
        }
        "confirm-delivery" => {
            ensure(&st, &p, Action::ConfirmDelivery, &res).await?;
            if d.data.status == DealStatus::HandedOver && d.data.pickup_method == PickupMethod::Transporter {
                d.data.status = d.data.status.transition(DealStatus::InTransit)?;
            }
            d.data.status = d.data.status.transition(DealStatus::Delivered)?;
            d.data.delivered_at = Some(now());
            settle_delivered(&st, &mut d).await?;
        }
        "cancel" | "no-show" => {
            ensure(&st, &p, Action::CancelDeal, &res).await?;
            let next = match (action.as_str(), b.who.as_deref()) {
                ("cancel", _) => DealStatus::Cancelled,
                (_, Some("SELLER")) if viewer != Viewer::Seller => DealStatus::SellerNoShow,
                (_, Some("PICKUP")) if viewer == Viewer::Seller => DealStatus::PickupNoShow,
                _ => return Err(ApiError::bad("who must be SELLER (reported by the vendor) or PICKUP (reported by the seller)")),
            };
            d.data.status = d.data.status.transition(next)?;
            d.data.cancel_reason = b.reason.map(|r| r.chars().take(300).collect());
            let mut extra = Vec::new();
            if let Some(mut l) = st.db.listing(&d.data.listing_id).await? {
                l.data.status = l.data.status.transition(kabados_domain::listing::ListingStatus::Cancelled)?;
                l.data.deal_id = None;
                l.data.accepted_offer_id = None;
                extra.push(st.db.save_listing_op(&mut l)?);
            }
            if let Some(jid) = &d.data.job_id
                && let Some(mut job) = st.db.job(jid).await?
                && job.data.status.can_transition_to(JobStatus::Cancelled)
            {
                extra.extend(st.db.clear_job_offer_ops(&job.data));
                job.data.status = JobStatus::Cancelled;
                extra.push(st.db.save_job_op(&mut job)?);
            }
            let other = if viewer == Viewer::Seller { d.data.vendor_id.clone() } else { d.data.seller_id.clone() };
            let n =
                Notification::new(other, NotificationKind::Dispute, "Deal cancelled", "The other party cancelled this deal.").link(&link);
            st.db.save_deal_with(&mut d, extra, vec![], vec![n]).await?;
        }
        _ => return Err(ApiError::NotFound("action")),
    }
    let (d, p, v) = load(&st, &me, deal_id.as_str()).await?;
    Ok(Json(deal_view(&st, &me, &d.data, &p, v).await?))
}

/// DELIVERED → SETTLED: ledger, Green Receipt, job completion (PLAN.md §4.1 last row).
pub async fn settle_delivered(st: &AppState, d: &mut Record<Deal>) -> ApiResult<()> {
    let deal_id = d.data.id.clone();
    let lines: Vec<DealLine> = lines_of(st, &deal_id).await?.into_iter().map(|r| r.data).collect();
    settle::post_ledger(st, &d.data).await?;
    let receipt = settle::receipt(st, &d.data, &lines).await?;
    d.data.receipt_id = Some(receipt.id.clone());
    d.data.status = d.data.status.transition(DealStatus::Settled)?;
    let mut extra = vec![st.db.receipt_op(&receipt)?];
    if let Some(jid) = &d.data.job_id
        && let Some(mut job) = st.db.job(jid).await?
        && job.data.status.can_transition_to(JobStatus::Delivered)
    {
        job.data.status = JobStatus::Delivered;
        extra.push(st.db.save_job_op(&mut job)?);
    }
    if let Some(mut l) = st.db.listing(&d.data.listing_id).await? {
        let partial = lines.iter().any(|l| l.status == LineStatus::Removed);
        let to = if partial {
            kabados_domain::listing::ListingStatus::PartiallyCompleted
        } else {
            kabados_domain::listing::ListingStatus::Completed
        };
        l.data.status = l.data.status.transition(to)?;
        extra.push(st.db.save_listing_op(&mut l)?);
    }
    let sig = settle::receipt_sig(st, &receipt.id);
    let n = Notification::new(
        d.data.seller_id.clone(),
        NotificationKind::Receipt,
        "Your Green Receipt",
        format!("{} recycled. Thank you!", receipt.total_weight),
    )
    .link(format!("/receipt?id={}&sig={sig}", receipt.id));
    st.db
        .save_deal_with(
            d,
            extra,
            vec![DomainEvent::DeliveredToYard { deal_id: deal_id.clone() }, DomainEvent::DealSettled { deal_id }],
            vec![n],
        )
        .await?;
    Ok(())
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct NewWeighing {
    pub line_n: u32,
    pub reading_g: Grams,
    pub image_id: Option<ImageId>,
    pub ai_reading_g: Option<Grams>,
    pub ai_confidence: Option<f32>,
    pub lat: Option<f64>,
    pub lng: Option<f64>,
}

/// Records a scale reading; when every line is weighed the final amount is
/// computed by `kabados-pricing` and the seller is asked to confirm.
#[utoipa::path(post, path = "/v1/deals/{id}/weighings", tag = "deals", request_body = NewWeighing, responses((status = 200, body = DealView)))]
pub async fn weigh(State(st): State<AppState>, me: Me, Path(id): Path<String>, Json(b): Json<NewWeighing>) -> ApiResult<Json<DealView>> {
    let (mut d, p, v) = load(&st, &me, &id).await?;
    ensure(&st, &p, Action::RecordWeighing, &Resource::Deal(deal_attrs(&d.data))).await?;
    if b.reading_g.0 <= 0 || b.reading_g.0 > 50_000_000 {
        return Err(ApiError::bad("reading must be between 1 g and 50 tonnes"));
    }
    let deal_id = d.data.id.clone();
    let mut lines: Vec<DealLine> = lines_of(&st, &deal_id).await?.into_iter().map(|r| r.data).collect();
    let line = lines.iter_mut().find(|l| l.n == b.line_n).ok_or(ApiError::NotFound("line"))?;
    if line.status == LineStatus::Removed {
        return Err(ApiError::conflict("this line was removed"));
    }
    line.weighed_g = Some(b.reading_g);
    if line.status == LineStatus::Pending {
        line.status = LineStatus::Weighed;
    }
    let w = Weighing {
        line_n: b.line_n,
        image_id: b.image_id,
        ai_reading_g: b.ai_reading_g,
        ai_confidence: b.ai_confidence,
        reading_g: b.reading_g,
        by_user: me.id().clone(),
        at: now(),
        lat: b.lat,
        lng: b.lng,
    };
    let mut extra = vec![st.db.weighing_op(&deal_id, &w)?, st.db.line_op(&deal_id, line)?];
    let events = vec![DomainEvent::LineWeighed { deal_id: deal_id.clone(), line_n: b.line_n, weight: b.reading_g }];
    let mut notes = vec![];
    let all_weighed = lines.iter().all(|l| l.status == LineStatus::Removed || l.weighed_g.is_some());
    if all_weighed {
        let q = settle::compute(&st, &d.data, &lines).await?;
        for l in lines.iter_mut().filter(|l| l.status != LineStatus::Removed) {
            l.amount = settle::amount_of(&q, &l.item_id);
            extra.push(st.db.line_op(&deal_id, l)?);
        }
        d.data.final_amount = Some(q.quote_low);
        d.data.platform_fee = crate::routes::offers::platform_fee(q.quote_low, st.cfg.platform_fee_bps);
        if d.data.status != DealStatus::LineRegraded {
            d.data.status = d.data.status.transition(DealStatus::Weighed)?;
        }
        let total: Grams = lines.iter().filter(|l| l.status != LineStatus::Removed).filter_map(|l| l.weighed_g).sum();
        notes.push(
            Notification::new(
                d.data.seller_id.clone(),
                NotificationKind::WeightsToConfirm,
                "Confirm the weights",
                format!("{total}, {} total", q.quote_low),
            )
            .link(format!("/deal?id={deal_id}")),
        );
    }
    // Remove duplicate line ops for the weighed line (same key written twice is not allowed in one transaction).
    dedupe_ops(&mut extra);
    st.db.save_deal_with(&mut d, extra, events, notes).await?;
    Ok(Json(deal_view(&st, &me, &d.data, &p, v).await?))
}

/// Keeps the last put per (PK, SK) so a transaction never touches an item twice.
fn dedupe_ops(ops: &mut Vec<kabados_db::TxOp>) {
    use kabados_db::TxOp;
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(ops.len());
    for op in ops.drain(..).rev() {
        let key = match &op {
            TxOp::Put { item, .. } => (
                kabados_db::str_attr(item, "PK").unwrap_or_default().to_owned(),
                kabados_db::str_attr(item, "SK").unwrap_or_default().to_owned(),
            ),
            TxOp::Delete { pk, sk, .. } | TxOp::Check { pk, sk, .. } => (pk.clone(), sk.clone()),
        };
        if seen.insert(key) {
            out.push(op);
        }
    }
    out.reverse();
    *ops = out;
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct Regrade {
    pub category_id: String,
    pub grade: Option<String>,
}

/// `POST /v1/deals/{id}/lines/{n}:regrade` (vendor/transporter) and
/// `:accept-regrade` / `:remove` (seller), PLAN.md §13.5.
#[utoipa::path(post, path = "/v1/deals/{id}/lines/{n}:regrade", tag = "deals", request_body = Regrade, responses((status = 200, body = DealView)))]
pub async fn line_action(
    State(st): State<AppState>,
    me: Me,
    Path((id, seg)): Path<(String, String)>,
    body: Option<Json<Regrade>>,
) -> ApiResult<Json<DealView>> {
    let (mut d, p, v) = load(&st, &me, &id).await?;
    let (n, action): (u32, String) = id_action(&seg, "line number")?;
    let deal_id = d.data.id.clone();
    let mut lines: Vec<DealLine> = lines_of(&st, &deal_id).await?.into_iter().map(|r| r.data).collect();
    let idx = lines.iter().position(|l| l.n == n).ok_or(ApiError::NotFound("line"))?;
    let res = Resource::Deal(deal_attrs(&d.data));
    let mut events = vec![];
    match action.as_str() {
        "regrade" => {
            ensure(&st, &p, Action::RegradeLine, &res).await?;
            let b = body.ok_or_else(|| ApiError::bad("category is required"))?.0;
            let tax = st.taxonomy().await?;
            let cat = tax.require(&b.category_id)?;
            let rules = match d.data.rule_set_version {
                Some(rv) => st.db.rule_set_version(&d.data.vendor_id, rv).await?,
                None => st.db.latest_rule_set(&d.data.vendor_id).await?,
            }
            .ok_or(ApiError::NotFound("rule set"))?
            .data;
            let probe = kabados_pricing::QuoteItem {
                item_id: lines[idx].item_id.clone(),
                category_id: cat.id.clone(),
                grade: b.grade.clone(),
                attributes: Default::default(),
                quantity: lines[idx].quantity,
                est_weight: lines[idx].weighed_g.unwrap_or_default(),
                plausible: None,
                ladder: kabados_domain::ladder::LadderLevel::L4VerifiedAtPickup,
                hazardous: cat.hazardous,
                weighed: lines[idx].weighed_g,
            };
            let (_, line) = kabados_pricing::match_line(&rules, &tax, &probe, probe.est_weight)
                .map_err(|_| ApiError::bad("your rate card has no price for that category"))?;
            let rate = match cat.unit {
                kabados_domain::taxonomy::Unit::Kg => Rate::Kg { paise_per_kg: Paise(line.price_per_kg_paise.unwrap_or(0)) },
                kabados_domain::taxonomy::Unit::Piece => Rate::Piece { paise_per_piece: Paise(line.price_per_piece_paise.unwrap_or(0)) },
            };
            let l = &mut lines[idx];
            events.push(DomainEvent::LineRegraded {
                deal_id: deal_id.clone(),
                line_n: n,
                from_category: l.category_id.clone(),
                to_category: cat.id.clone(),
            });
            l.regraded_from = Some(l.category_id.clone());
            l.category_id = cat.id.clone();
            l.grade = b.grade;
            l.rate = rate;
            l.modifier_pct = 0;
            l.status = LineStatus::RegradePending;
            d.data.status = d.data.status.transition(DealStatus::LineRegraded)?;
        }
        "accept-regrade" | "remove" => {
            if d.data.seller_id != *me.id() {
                return Err(ApiError::forbidden("only the seller can decide on re-graded lines"));
            }
            let l = &mut lines[idx];
            if action == "remove" {
                l.status = LineStatus::Removed;
                l.amount = None;
            } else if l.status == LineStatus::RegradePending {
                l.status = if l.weighed_g.is_some() { LineStatus::Weighed } else { LineStatus::Pending };
            } else {
                return Err(ApiError::conflict("this line is not awaiting a decision"));
            }
        }
        _ => return Err(ApiError::NotFound("action")),
    }
    let mut extra = vec![st.db.line_op(&deal_id, &lines[idx])?];
    if lines.iter().all(|l| l.status != LineStatus::RegradePending) && lines.iter().any(|l| l.status != LineStatus::Removed) {
        let all_weighed = lines.iter().all(|l| l.status == LineStatus::Removed || l.weighed_g.is_some());
        if all_weighed {
            let q = settle::compute(&st, &d.data, &lines).await?;
            for l in lines.iter_mut().filter(|l| l.status != LineStatus::Removed) {
                l.amount = settle::amount_of(&q, &l.item_id);
                extra.push(st.db.line_op(&deal_id, l)?);
            }
            d.data.final_amount = Some(q.quote_low);
            d.data.platform_fee = crate::routes::offers::platform_fee(q.quote_low, st.cfg.platform_fee_bps);
        }
        if d.data.status == DealStatus::LineRegraded {
            d.data.status = d.data.status.transition(DealStatus::Weighed)?;
        }
    }
    dedupe_ops(&mut extra);
    let notes = if action == "regrade" {
        vec![
            Notification::new(
                d.data.seller_id.clone(),
                NotificationKind::WeightsToConfirm,
                "An item was re-graded",
                "Accept the new price or remove the item.",
            )
            .link(format!("/deal?id={deal_id}")),
        ]
    } else {
        vec![]
    };
    st.db.save_deal_with(&mut d, extra, events, notes).await?;
    Ok(Json(deal_view(&st, &me, &d.data, &p, v).await?))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct NewDispute {
    pub kind: DisputeKind,
    pub description: String,
}

/// Opens a dispute; the deal waits for ops (PLAN.md §13.5).
#[utoipa::path(post, path = "/v1/deals/{id}/disputes", tag = "deals", request_body = NewDispute, responses((status = 201, body = Dispute)))]
pub async fn open_dispute(
    State(st): State<AppState>,
    me: Me,
    Path(id): Path<String>,
    Json(b): Json<NewDispute>,
) -> ApiResult<(StatusCode, Json<Dispute>)> {
    let (mut d, p, _) = load(&st, &me, &id).await?;
    ensure(&st, &p, Action::OpenDispute, &Resource::Deal(deal_attrs(&d.data))).await?;
    if b.description.trim().len() < 5 {
        return Err(ApiError::bad("describe the problem"));
    }
    let dispute = Dispute {
        id: DisputeId::new(),
        deal_id: d.data.id.clone(),
        opened_by: me.id().clone(),
        kind: b.kind,
        description: b.description.trim().chars().take(2000).collect(),
        status: DisputeStatus::Open,
        resolution: None,
        resume_status: d.data.status,
        sla_due_at: now() + Duration::hours(24),
        created_at: now(),
    };
    if d.data.status.can_transition_to(DealStatus::Disputed) {
        d.data.status = DealStatus::Disputed;
    }
    let ev = DomainEvent::DisputeOpened { deal_id: d.data.id.clone(), dispute_id: dispute.id.clone() };
    st.db.save_deal_with(&mut d, vec![st.db.dispute_op(&dispute)?], vec![ev], vec![]).await?;
    Ok((StatusCode::CREATED, Json(dispute)))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct NewRating {
    pub stars: u8,
    pub comment: Option<String>,
    /// Who is rated when the caller is the vendor (seller or transporter).
    pub to: Option<UserId>,
}

#[utoipa::path(post, path = "/v1/deals/{id}/ratings", tag = "deals", request_body = NewRating, responses((status = 201, body = Rating)))]
pub async fn rate(
    State(st): State<AppState>,
    me: Me,
    Path(id): Path<String>,
    Json(b): Json<NewRating>,
) -> ApiResult<(StatusCode, Json<Rating>)> {
    let (d, p, viewer) = load(&st, &me, &id).await?;
    ensure(&st, &p, Action::RateDeal, &Resource::Deal(deal_attrs(&d.data))).await?;
    if !(1..=5).contains(&b.stars) {
        return Err(ApiError::bad("stars must be 1–5"));
    }
    let to = match viewer {
        Viewer::Seller | Viewer::Transporter => d.data.vendor_id.clone(),
        _ => b.to.filter(|t| Some(t) == d.data.transporter_id.as_ref()).unwrap_or_else(|| d.data.seller_id.clone()),
    };
    let r = Rating {
        deal_id: d.data.id.clone(),
        from_user: me.id().clone(),
        to_user: to.clone(),
        stars: b.stars,
        comment: b.comment.map(|c| c.chars().take(500).collect()),
        at: now(),
    };
    st.db
        .store
        .transact(vec![
            st.db.rating_op(&r)?,
            kabados_db::repo::outbox_op(
                &kabados_db::keys::deal(&d.data.id),
                DomainEvent::RatingSubmitted { deal_id: d.data.id.clone(), from_user: me.id().clone(), to_user: to, stars: b.stars },
                None,
            )?,
        ])
        .await
        .map_err(|e| if e.is_conflict() { ApiError::conflict("you already rated this deal") } else { e.into() })?;
    Ok((StatusCode::CREATED, Json(r)))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ReceiptView {
    pub receipt: GreenReceipt,
    /// Shareable verification link path for the public site.
    pub verify_path: String,
}

#[utoipa::path(get, path = "/v1/deals/{id}/receipt", tag = "deals", responses((status = 200, body = ReceiptView)))]
pub async fn get_receipt(State(st): State<AppState>, me: Me, Path(id): Path<String>) -> ApiResult<Json<ReceiptView>> {
    let (d, p, _) = load(&st, &me, &id).await?;
    ensure(&st, &p, Action::ViewReceipt, &Resource::Deal(deal_attrs(&d.data))).await?;
    let rid = d.data.receipt_id.clone().ok_or(ApiError::NotFound("receipt"))?;
    let r = st.db.receipt(&rid).await?.ok_or(ApiError::NotFound("receipt"))?;
    let sig = settle::receipt_sig(&st, &rid);
    Ok(Json(ReceiptView { verify_path: format!("/receipt?id={rid}&sig={sig}"), receipt: r.data }))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PresignDealImage {
    pub content_type: String,
    pub size: u64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DealImageUpload {
    pub image_id: ImageId,
    pub upload: PresignedUpload,
}

/// Pickup evidence photos (items as found, scale display) by vendor or transporter.
#[utoipa::path(post, path = "/v1/deals/{id}/images:presign", tag = "deals", request_body = PresignDealImage, responses((status = 200, body = DealImageUpload)))]
pub async fn presign_deal_image(
    State(st): State<AppState>,
    me: Me,
    Path(id): Path<String>,
    Json(b): Json<PresignDealImage>,
) -> ApiResult<Json<DealImageUpload>> {
    let (d, p, _) = load(&st, &me, &id).await?;
    ensure(&st, &p, Action::RecordWeighing, &Resource::Deal(deal_attrs(&d.data))).await?;
    if !["image/jpeg", "image/png", "image/webp"].contains(&b.content_type.as_str()) || b.size == 0 || b.size > 15 * 1024 * 1024 {
        return Err(ApiError::bad("photos must be JPEG, PNG or WebP under 15 MB"));
    }
    let image_id = ImageId::new();
    let key = deal_image_key(&d.data.id, &image_id);
    let upload = st.ix.blobs.presign_put(&st.cfg.media_bucket, &key, &b.content_type, b.size, None).await?;
    Ok(Json(DealImageUpload { image_id, upload }))
}

pub fn deal_image_key(deal: &DealId, image: &ImageId) -> String {
    format!("deals/{deal}/{image}")
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ScaleRequest {
    pub image_id: ImageId,
    pub content_type: Option<String>,
}

/// AI reads the scale display (PLAN.md §11.8); on low confidence the driver types the value.
#[utoipa::path(post, path = "/v1/deals/{id}/scale-reading", tag = "deals", request_body = ScaleRequest, responses((status = 200, body = crate::ai::ScaleRead)))]
pub async fn scale_reading(
    State(st): State<AppState>,
    me: Me,
    Path(id): Path<String>,
    Json(b): Json<ScaleRequest>,
) -> ApiResult<Json<crate::ai::ScaleRead>> {
    let (d, p, _) = load(&st, &me, &id).await?;
    ensure(&st, &p, Action::RecordWeighing, &Resource::Deal(deal_attrs(&d.data))).await?;
    let bytes =
        st.ix.blobs.get(&st.cfg.media_bucket, &deal_image_key(&d.data.id, &b.image_id)).await.map_err(|_| ApiError::NotFound("image"))?;
    let read = st
        .ai
        .read_scale(bytes, b.content_type.as_deref().unwrap_or("image/jpeg"))
        .await
        .map_err(|e| ApiError::Unavailable(e.to_string()))?;
    Ok(Json(read))
}
