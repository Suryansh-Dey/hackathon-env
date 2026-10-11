//! Platform-mediated payments (Mode A) and the Razorpay webhook (PLAN.md §13.2, §17.2).

use super::{now, parse};
use crate::access::*;
use crate::auth::Me;
use crate::error::{ApiError, ApiResult};
use crate::settle;
use crate::state::AppState;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use kabados_authz::{Action, Resource};
use kabados_db::repo::{Notification, NotificationKind, PostOutcome};
use kabados_domain::Paise;
use kabados_domain::deal::DealStatus;
use kabados_domain::events::DomainEvent;
use kabados_domain::ids::*;
use kabados_domain::ledger::{AccountId, AccountKind, CaptureSplit, Entry, Side, Transaction, TxnKind};
use kabados_domain::offer::PaymentMode;
use kabados_domain::state::StateMachine;
use kabados_integrations::payments::{Order, WebhookEvent, parse_webhook};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use utoipa::ToSchema;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/deals/{id}/payment-request", post(payment_request))
        .route("/v1/deals/{id}/payment:verify-checkout", post(verify_checkout))
        .route("/webhooks/razorpay", post(razorpay_webhook))
        .route("/v1/payments/{seg}", post(simulate))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PaymentRequestView {
    pub deal_id: DealId,
    pub order: Order,
    pub seller_amount: Paise,
    pub transporter_fare: Paise,
    pub platform_fee: Paise,
}

/// Vendor requests payment: seller amount + transporter fare + platform fee.
#[utoipa::path(post, path = "/v1/deals/{id}/payment-request", tag = "payments", responses((status = 200, body = PaymentRequestView)))]
pub async fn payment_request(State(st): State<AppState>, me: Me, Path(id): Path<String>) -> ApiResult<Json<PaymentRequestView>> {
    let id: DealId = parse(&id, "deal id")?;
    let mut d = st.db.deal(&id).await?.ok_or(ApiError::NotFound("deal"))?;
    let v = vendor_of(&st, &me).await?;
    ensure(&st, &vendor_principal(&v.data), Action::RequestPayment, &Resource::Deal(deal_attrs(&d.data))).await?;
    if d.data.payment_mode != PaymentMode::Platform {
        return Err(ApiError::conflict("this deal is paid directly"));
    }
    let seller_amount = d.data.final_amount.ok_or_else(|| ApiError::conflict("weigh and confirm first"))?;
    let fare = d.data.transporter_fare.unwrap_or_default();
    let fee = d.data.platform_fee;
    let notes = BTreeMap::from([("deal_id".to_owned(), id.to_string()), ("purpose".to_owned(), "kabados_deal".to_owned())]);
    let order = st.ix.payments.create_order(seller_amount + fare + fee, id.as_str(), &notes).await?;
    d.data.payment_order_id = Some(order.id.clone());
    d.data.status = d.data.status.transition(DealStatus::AwaitingPayment)?;
    st.db.save_deal_with(&mut d, vec![], vec![], vec![]).await?;
    Ok(Json(PaymentRequestView { deal_id: id, order, seller_amount, transporter_fare: fare, platform_fee: fee }))
}

/// Applies a captured payment exactly once: ledger, deal → PAID, handover
/// code, transporter transfer and seller payout.
pub async fn on_captured(
    st: &AppState,
    event_id: &str,
    payment_id: &str,
    order_id: &str,
    amount: Paise,
    deal_id: &DealId,
) -> ApiResult<()> {
    let mut d = st.db.deal(deal_id).await?.ok_or(ApiError::NotFound("deal"))?;
    if d.data.payment_order_id.as_deref() != Some(order_id) {
        tracing::warn!(%order_id, %deal_id, "capture for an unknown order; parking in suspense");
        return Ok(());
    }
    // Idempotent: a deal only leaves AWAITING_PAYMENT once, so a duplicate or
    // stale capture (even with a fresh payment id) is a no-op here. Genuine
    // duplicate webhooks are also deduped by event id in process_webhook.
    if d.data.status != DealStatus::AwaitingPayment {
        tracing::info!(%deal_id, status = d.data.status.as_str(), "capture ignored; deal not awaiting payment");
        return Ok(());
    }
    let seller_amount = d.data.final_amount.unwrap_or_default();
    let fare = d.data.transporter_fare.unwrap_or_default();
    let split = CaptureSplit {
        seller: d.data.seller_id.clone(),
        transporter: d.data.transporter_id.clone(),
        seller_amount,
        fare,
        fee: d.data.platform_fee,
    };
    let expected = seller_amount + fare + d.data.platform_fee;
    if amount != expected {
        tracing::error!(%deal_id, got = amount.0, expected = expected.0, "captured amount mismatch");
    }
    let txn = Transaction::mode_a_capture(format!("rzp-{payment_id}"), deal_id.clone(), &split, now());
    st.db.post_transaction(&txn).await?;
    d.data.status = d.data.status.transition(DealStatus::Paid)?;
    let otp_op = settle::issue_handover(st, &mut d.data).await?;
    let n = Notification::new(
        d.data.seller_id.clone(),
        NotificationKind::PaymentReceived,
        "Payment received",
        format!("{seller_amount} is on its way to you. Share the handover code to load the items."),
    )
    .link(format!("/deal?id={deal_id}"));
    let ev = DomainEvent::PaymentCaptured { deal_id: deal_id.clone(), provider_payment_id: payment_id.to_owned(), amount };
    st.db.save_deal_with(&mut d, vec![otp_op], vec![ev], vec![n]).await?;
    // Transporter fare via Route and seller payout via RazorpayX (both mocked until activated).
    if let (Some(t), true) = (&d.data.transporter_id, fare.0 > 0) {
        let acct = st.ix.route.create_linked_account(t.as_str(), "transporter").await?;
        st.ix.route.transfer(payment_id, &acct, fare).await?;
        post_simple(
            st,
            &format!("trf-{payment_id}"),
            TxnKind::TransferToTransporter,
            deal_id,
            AccountId::party(AccountKind::TransporterPayable, t.clone()),
            fare,
        )
        .await?;
    }
    let payout_to = st
        .db
        .payout_methods(&d.data.seller_id)
        .await?
        .into_iter()
        .find(|p| p.data.validation == kabados_domain::user::PayoutValidation::Validated);
    if let Some(p) = payout_to.and_then(|p| p.data.provider_ref) {
        let po = st.ix.payouts.payout(&p, seller_amount, &format!("payout-{deal_id}")).await?;
        if po.status == "processed" {
            post_simple(
                st,
                &format!("pout-{}", po.id),
                TxnKind::PayoutToSeller,
                deal_id,
                AccountId::party(AccountKind::SellerReceivable, d.data.seller_id.clone()),
                seller_amount,
            )
            .await?;
            let ev = kabados_db::repo::outbox_op(
                &kabados_db::keys::deal(deal_id),
                DomainEvent::PayoutProcessed { deal_id: deal_id.clone(), provider_payout_id: po.id, amount: seller_amount, success: true },
                Some(event_id.to_owned()),
            )?;
            st.db.store.transact(vec![ev]).await?;
        }
    }
    Ok(())
}

/// Money leaving the clearing account to a party.
async fn post_simple(st: &AppState, key: &str, kind: TxnKind, deal: &DealId, to: AccountId, amount: Paise) -> ApiResult<PostOutcome> {
    let t = Transaction {
        idempotency_key: key.to_owned(),
        kind,
        deal_id: Some(deal.clone()),
        occurred_at: now(),
        entries: vec![
            Entry { account: to, side: Side::Debit, amount },
            Entry { account: AccountId::platform(AccountKind::GatewayClearing), side: Side::Credit, amount },
        ],
        memo: None,
    };
    Ok(st.db.post_transaction(&t).await?)
}

/// Verified webhook processing shared by the real receiver and the simulator.
pub async fn process_webhook(st: &AppState, event_id: &str, body: &[u8]) -> ApiResult<()> {
    if !st.db.first_time("razorpay", event_id, chrono::Duration::days(7)).await? {
        return Ok(());
    }
    let key = format!("webhooks/razorpay/{}/{event_id}.json", now().format("%Y-%m-%d"));
    if let Err(e) = st.ix.blobs.put(&st.cfg.documents_bucket, &key, body.to_vec(), "application/json").await {
        tracing::warn!(error = %e, "could not archive webhook payload");
    }
    match parse_webhook(body)? {
        WebhookEvent::PaymentCaptured { payment_id, order_id, amount, notes } => {
            let Some(deal_id) = notes.get("deal_id").and_then(|d| DealId::parse(d).ok()) else { return Ok(()) };
            on_captured(st, event_id, &payment_id, &order_id, amount, &deal_id).await
        }
        WebhookEvent::PaymentFailed { order_id, reason, .. } => {
            tracing::info!(%order_id, ?reason, "payment failed");
            Ok(())
        }
        other => {
            tracing::info!(?other, "webhook ignored");
            Ok(())
        }
    }
}

/// `POST /webhooks/razorpay`: HMAC-verified, stored raw, processed exactly once.
pub async fn razorpay_webhook(State(st): State<AppState>, headers: HeaderMap, body: Bytes) -> ApiResult<StatusCode> {
    let sig = headers.get("x-razorpay-signature").and_then(|v| v.to_str().ok()).ok_or_else(|| ApiError::bad("missing signature"))?;
    if !st.ix.payments.verify_webhook(&body, sig) {
        return Err(ApiError::Unauthorized);
    }
    let event_id = headers.get("x-razorpay-event-id").and_then(|v| v.to_str().ok()).map(str::to_owned).unwrap_or_else(|| hex_digest(&body));
    process_webhook(&st, &event_id, &body).await?;
    Ok(StatusCode::OK)
}

fn hex_digest(b: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(&Sha256::digest(b)[..16])
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CheckoutResult {
    pub razorpay_payment_id: String,
    pub razorpay_order_id: String,
    pub razorpay_signature: String,
}

/// Razorpay Checkout success handler: the signature proves the payment, so
/// the deal advances even before the webhook arrives (idempotent with it).
#[utoipa::path(post, path = "/v1/deals/{id}/payment:verify-checkout", tag = "payments", request_body = CheckoutResult, responses((status = 204)))]
pub async fn verify_checkout(
    State(st): State<AppState>,
    me: Me,
    Path(id): Path<String>,
    Json(b): Json<CheckoutResult>,
) -> ApiResult<StatusCode> {
    let id: DealId = parse(&id, "deal id")?;
    let d = st.db.deal(&id).await?.ok_or(ApiError::NotFound("deal"))?;
    if d.data.vendor_id != *me.id() {
        return Err(ApiError::NotFound("deal"));
    }
    if !st.ix.payments.verify_checkout(&b.razorpay_order_id, &b.razorpay_payment_id, &b.razorpay_signature) {
        return Err(ApiError::bad("payment signature is invalid"));
    }
    let amount = d.data.final_amount.unwrap_or_default() + d.data.transporter_fare.unwrap_or_default() + d.data.platform_fee;
    if st.db.first_time("razorpay-checkout", &b.razorpay_payment_id, chrono::Duration::days(7)).await? {
        on_captured(&st, &b.razorpay_payment_id, &b.razorpay_payment_id, &b.razorpay_order_id, amount, &id).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /v1/payments/{orderId}:simulate-capture` (mock payments only): the
/// mock signs a Razorpay-shaped webhook that goes through the real receiver path.
pub async fn simulate(State(st): State<AppState>, me: Me, Path(seg): Path<String>) -> ApiResult<StatusCode> {
    let (order_id, action) = super::split_action(&seg);
    if action != Some("simulate-capture") || st.cfg.is_prod() {
        return Err(ApiError::NotFound("action"));
    }
    let (deal_id, d) = {
        let (idx, _) = st.db.user_deals(me.id(), Some(DealStatus::AwaitingPayment), None).await?;
        let mut found = None;
        for i in idx {
            if let Some(d) = st.db.deal(&i.deal_id).await?
                && d.data.payment_order_id.as_deref() == Some(order_id)
            {
                found = Some((i.deal_id, d));
                break;
            }
        }
        found.ok_or(ApiError::NotFound("order"))?
    };
    let amount = d.data.final_amount.unwrap_or_default() + d.data.transporter_fare.unwrap_or_default() + d.data.platform_fee;
    let notes = BTreeMap::from([("deal_id".to_owned(), deal_id.to_string())]);
    let wh = st
        .ix
        .payments
        .simulate_capture(order_id, amount, &notes)
        .ok_or_else(|| ApiError::bad("payments are not mocked here; pay with Razorpay test mode"))?;
    if !st.ix.payments.verify_webhook(&wh.body, &wh.signature) {
        return Err(ApiError::Internal(anyhow::anyhow!("mock signature mismatch")));
    }
    process_webhook(&st, &wh.event_id, &wh.body).await?;
    Ok(StatusCode::NO_CONTENT)
}
