//! Ops console API: staff pool tokens (mandatory MFA) only; every action is
//! audited (PLAN.md §10.1).

use super::{PageQuery, id_action, now, parse};
use crate::access::*;
use crate::auth::Staff;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::views::{ListingSummary, MeView};
use axum::extract::{Path, Query, State};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use kabados_authz::{Action, Resource};
use kabados_db::repo::{AuditEntry, KycSubject, Notification, NotificationKind, RateIndexEntry};
use kabados_domain::deal::*;
use kabados_domain::events::DomainEvent;
use kabados_domain::ids::*;
use kabados_domain::state::StateMachine;
use kabados_domain::taxonomy::Category;
use kabados_domain::user::KycStatus;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/ops/kyc", get(kyc_queue))
        .route("/v1/ops/kyc/{seg}", post(kyc_decision))
        .route("/v1/ops/disputes", get(dispute_queue))
        .route("/v1/ops/disputes/{deal}/{seg}", post(resolve_dispute))
        .route("/v1/ops/deals/{id}", get(deal_evidence))
        .route("/v1/ops/listings/{id}", get(listing_evidence))
        .route("/v1/ops/ai-reviews", get(ai_reviews))
        .route("/v1/ops/taxonomy/categories/{id}", put(put_category))
        .route("/v1/ops/users/{seg}", post(user_action))
        .route("/v1/ops/rate-index:publish", post(publish_rate_index))
        .route("/v1/ops/config", get(config))
        .route("/v1/ops/audit/{subject}", get(audit))
}

async fn guard(st: &AppState, staff: &Staff, a: Action) -> ApiResult<()> {
    ensure(st, &ops_principal(&staff.0.sub), a, &Resource::Platform).await
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct KycQuery {
    pub status: Option<KycStatus>,
    pub cursor: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct KycItem {
    pub kind: String,
    pub user_id: UserId,
    pub name: String,
    pub status: KycStatus,
    pub submitted_at: Option<chrono::DateTime<chrono::Utc>>,
    pub documents: Vec<kabados_domain::user::KycDocument>,
}

#[utoipa::path(get, path = "/v1/ops/kyc", tag = "ops", params(KycQuery), responses((status = 200, body = Vec<KycItem>)))]
pub async fn kyc_queue(State(st): State<AppState>, staff: Staff, Query(q): Query<KycQuery>) -> ApiResult<Json<Vec<KycItem>>> {
    guard(&st, &staff, Action::OpsReviewKyc).await?;
    let (items, _) = st.db.kyc_queue(q.status.unwrap_or(KycStatus::PendingManualReview), q.cursor, 50).await?;
    let mut out = Vec::new();
    for s in items {
        let (kind, uid, name, status, at) = match s {
            KycSubject::Vendor(v) => {
                ("VENDOR", v.data.user_id.clone(), v.data.business_name.clone(), v.data.kyc_status, v.data.kyc_submitted_at)
            }
            KycSubject::Transporter(t) => {
                ("TRANSPORTER", t.data.user_id.clone(), String::new(), t.data.kyc_status, t.data.kyc_submitted_at)
            }
        };
        let documents = st.db.documents(&uid).await?.into_iter().map(|r| r.data).collect();
        out.push(KycItem { kind: kind.into(), user_id: uid, name, status, submitted_at: at, documents });
    }
    Ok(Json(out))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct KycDecision {
    /// VENDOR or TRANSPORTER.
    pub kind: String,
    pub note: Option<String>,
}

/// `POST /v1/ops/kyc/{userId}:approve | :reject`.
#[utoipa::path(post, path = "/v1/ops/kyc/{userId}:approve", tag = "ops", request_body = KycDecision, responses((status = 200, body = Object)))]
pub async fn kyc_decision(
    State(st): State<AppState>,
    staff: Staff,
    Path(seg): Path<String>,
    Json(b): Json<KycDecision>,
) -> ApiResult<Json<serde_json::Value>> {
    guard(&st, &staff, Action::OpsReviewKyc).await?;
    let (uid, action): (UserId, String) = id_action(&seg, "user id")?;
    let status = match action.as_str() {
        "approve" => KycStatus::Approved,
        "reject" => KycStatus::Rejected,
        _ => return Err(ApiError::NotFound("action")),
    };
    match b.kind.as_str() {
        "VENDOR" => {
            let mut v = st.db.vendor(&uid).await?.ok_or(ApiError::NotFound("vendor"))?;
            v.data.kyc_status = status;
            st.db.save_vendor(&mut v).await?;
        }
        "TRANSPORTER" => {
            let mut t = st.db.transporter(&uid).await?.ok_or(ApiError::NotFound("transporter"))?;
            t.data.kyc_status = status;
            st.db.save_transporter(&mut t).await?;
        }
        _ => return Err(ApiError::bad("kind must be VENDOR or TRANSPORTER")),
    }
    st.db
        .audit(uid.as_str(), &staff.0.sub, &format!("KYC_{}", action.to_uppercase()), serde_json::json!({"kind": b.kind, "note": b.note}))
        .await?;
    let n = Notification::new(
        uid.clone(),
        NotificationKind::KycDecision,
        "Account review",
        if status == KycStatus::Approved { "Your account is approved." } else { "Your account needs changes. Check your documents." },
    );
    st.db.store.transact(vec![kabados_db::repo::notify_op(&kabados_db::keys::user(&uid), n)?]).await?;
    Ok(Json(serde_json::json!({"user_id": uid, "status": status})))
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct DisputeQuery {
    pub status: Option<DisputeStatus>,
    pub cursor: Option<String>,
}

#[utoipa::path(get, path = "/v1/ops/disputes", tag = "ops", params(DisputeQuery), responses((status = 200, body = Vec<Dispute>)))]
pub async fn dispute_queue(State(st): State<AppState>, staff: Staff, Query(q): Query<DisputeQuery>) -> ApiResult<Json<Vec<Dispute>>> {
    guard(&st, &staff, Action::OpsResolveDispute).await?;
    let (recs, _) = st.db.dispute_queue(q.status.unwrap_or(DisputeStatus::Open), q.cursor).await?;
    Ok(Json(recs.into_iter().map(|r| r.data).collect()))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct Resolution {
    pub resolution: String,
    /// Deal status to continue from (defaults to the status before the dispute).
    pub resume: Option<DealStatus>,
}

/// `POST /v1/ops/disputes/{dealId}/{disputeId}:resolve`.
#[utoipa::path(post, path = "/v1/ops/disputes/{dealId}/{disputeId}:resolve", tag = "ops", request_body = Resolution, responses((status = 200, body = Dispute)))]
pub async fn resolve_dispute(
    State(st): State<AppState>,
    staff: Staff,
    Path((deal, seg)): Path<(String, String)>,
    Json(b): Json<Resolution>,
) -> ApiResult<Json<Dispute>> {
    guard(&st, &staff, Action::OpsResolveDispute).await?;
    let deal_id: DealId = parse(&deal, "deal id")?;
    let (dispute_id, action): (DisputeId, String) = id_action(&seg, "dispute id")?;
    if action != "resolve" {
        return Err(ApiError::NotFound("action"));
    }
    let mut d = st.db.deal(&deal_id).await?.ok_or(ApiError::NotFound("deal"))?;
    let mut dispute = st.db.dispute(&deal_id, &dispute_id).await?.ok_or(ApiError::NotFound("dispute"))?.data;
    dispute.status = DisputeStatus::Resolved;
    dispute.resolution = Some(b.resolution.chars().take(2000).collect());
    if d.data.status == DealStatus::Disputed {
        let to = b.resume.unwrap_or(dispute.resume_status);
        d.data.status = d.data.status.transition(to)?;
    }
    let notes = [d.data.seller_id.clone(), d.data.vendor_id.clone()]
        .into_iter()
        .map(|u| {
            Notification::new(u, NotificationKind::Dispute, "Dispute resolved", dispute.resolution.clone().unwrap_or_default())
                .link(format!("/deal?id={deal_id}"))
        })
        .collect();
    st.db
        .save_deal_with(
            &mut d,
            vec![st.db.dispute_op(&dispute)?],
            vec![DomainEvent::DisputeResolved { deal_id: deal_id.clone(), dispute_id }],
            notes,
        )
        .await?;
    st.db.audit(deal_id.as_str(), &staff.0.sub, "DISPUTE_RESOLVED", serde_json::json!({"resolution": dispute.resolution})).await?;
    Ok(Json(dispute))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DealEvidence {
    pub deal: Deal,
    pub lines: Vec<DealLine>,
    pub weighings: Vec<Weighing>,
    pub jobs: Vec<kabados_domain::job::Job>,
    pub job_events: Vec<kabados_domain::job::JobEvent>,
    pub disputes: Vec<Dispute>,
    pub ratings: Vec<Rating>,
    pub audit: Vec<AuditEntry>,
}

/// Evidence timeline per deal: photos, scale readings, GPS, payments, audit.
#[utoipa::path(get, path = "/v1/ops/deals/{id}", tag = "ops", responses((status = 200, body = DealEvidence)))]
pub async fn deal_evidence(State(st): State<AppState>, staff: Staff, Path(id): Path<String>) -> ApiResult<Json<DealEvidence>> {
    guard(&st, &staff, Action::OpsResolveDispute).await?;
    let id: DealId = parse(&id, "deal id")?;
    let d = st.db.deal(&id).await?.ok_or(ApiError::NotFound("deal"))?;
    let c = st.db.deal_children(&id).await?;
    Ok(Json(DealEvidence {
        deal: d.data,
        lines: c.lines.into_iter().map(|r| r.data).collect(),
        weighings: c.weighings.into_iter().map(|r| r.data).collect(),
        jobs: c.jobs.into_iter().map(|r| r.data).collect(),
        job_events: c.job_events.into_iter().map(|r| r.data).collect(),
        disputes: c.disputes.into_iter().map(|r| r.data).collect(),
        ratings: c.ratings.into_iter().map(|r| r.data).collect(),
        audit: st.db.audit_log(id.as_str()).await?,
    }))
}

/// Full listing view for AI review (ops never see the exact address).
#[utoipa::path(get, path = "/v1/ops/listings/{id}", tag = "ops", responses((status = 200, body = crate::views::ListingView)))]
pub async fn listing_evidence(
    State(st): State<AppState>,
    staff: Staff,
    Path(id): Path<String>,
) -> ApiResult<Json<crate::views::ListingView>> {
    guard(&st, &staff, Action::OpsReviewAi).await?;
    let id: ListingId = parse(&id, "listing id")?;
    let b = crate::routes::listings::load_bundle(&st, &id).await?;
    Ok(Json(crate::routes::listings::build_view(&st, &b, crate::views::Viewer::Ops, None).await?))
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AiReviewItem {
    pub listing_id: ListingId,
    pub ver: u32,
    pub reason: String,
    pub at: chrono::DateTime<chrono::Utc>,
}

/// Low-confidence assessments, failed-closed analyses and contested verdicts.
#[utoipa::path(get, path = "/v1/ops/ai-reviews", tag = "ops", params(PageQuery), responses((status = 200, body = Vec<AiReviewItem>)))]
pub async fn ai_reviews(State(st): State<AppState>, staff: Staff, Query(q): Query<PageQuery>) -> ApiResult<Json<Vec<serde_json::Value>>> {
    guard(&st, &staff, Action::OpsReviewAi).await?;
    let (recs, _) = st
        .db
        .query_recs::<AiReviewItem>(kabados_db::Query::new(kabados_db::Table::Core, "AIREVIEW").reverse().limit(50).cursor(q.cursor))
        .await?;
    let mut out = Vec::new();
    for r in recs {
        let summary = st.db.listing(&r.data.listing_id).await?.map(|l| ListingSummary::from(&l.data));
        out.push(serde_json::json!({"item": r.data, "listing": summary}));
    }
    Ok(Json(out))
}

#[utoipa::path(put, path = "/v1/ops/taxonomy/categories/{id}", tag = "ops", request_body = Category, responses((status = 200, body = Category)))]
pub async fn put_category(
    State(st): State<AppState>,
    staff: Staff,
    Path(id): Path<String>,
    Json(c): Json<Category>,
) -> ApiResult<Json<Category>> {
    guard(&st, &staff, Action::OpsManageTaxonomy).await?;
    if c.id != id || !kabados_domain::taxonomy::is_valid_category_id(&id) {
        return Err(ApiError::bad("category id mismatch"));
    }
    if c.hazardous && c.hazard_class.is_none() {
        return Err(ApiError::bad("hazardous categories need a hazard class"));
    }
    st.db.put_category(c.clone()).await?;
    st.invalidate_taxonomy().await;
    st.db.audit(&format!("CAT#{id}"), &staff.0.sub, "TAXONOMY_UPDATED", serde_json::to_value(&c).map_err(anyhow::Error::from)?).await?;
    Ok(Json(c))
}

/// `POST /v1/ops/users/{userId}:suspend | :unsuspend`.
#[utoipa::path(post, path = "/v1/ops/users/{userId}:suspend", tag = "ops", responses((status = 200, body = MeView)))]
pub async fn user_action(State(st): State<AppState>, staff: Staff, Path(seg): Path<String>) -> ApiResult<Json<MeView>> {
    guard(&st, &staff, Action::OpsViewFraud).await?;
    let (uid, action): (UserId, String) = id_action(&seg, "user id")?;
    let suspend = match action.as_str() {
        "suspend" => true,
        "unsuspend" => false,
        _ => return Err(ApiError::NotFound("action")),
    };
    let mut u = st.db.user(&uid).await?.ok_or(ApiError::NotFound("user"))?;
    u.data.suspended = suspend;
    st.db.save_user(&mut u).await?;
    if let Some(mut v) = st.db.vendor(&uid).await? {
        v.data.suspended = suspend;
        st.db.save_vendor(&mut v).await?;
    }
    if let Some(mut t) = st.db.transporter(&uid).await? {
        t.data.suspended = suspend;
        st.db.save_transporter(&mut t).await?;
    }
    st.db.audit(uid.as_str(), &staff.0.sub, if suspend { "SUSPENDED" } else { "UNSUSPENDED" }, serde_json::json!({})).await?;
    Ok(Json(crate::routes::identity::me_view(&st, &u.data).await?))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PublishRateIndex {
    pub city: String,
    pub iso_week: String,
}

/// k-anonymity thresholds for publishing (PLAN.md §10.2).
pub const MIN_DEALS: u32 = 20;
pub const MIN_VENDORS: u32 = 5;

#[utoipa::path(post, path = "/v1/ops/rate-index:publish", tag = "ops", request_body = PublishRateIndex, responses((status = 200, body = Vec<RateIndexEntry>)))]
pub async fn publish_rate_index(
    State(st): State<AppState>,
    staff: Staff,
    Json(b): Json<PublishRateIndex>,
) -> ApiResult<Json<Vec<RateIndexEntry>>> {
    guard(&st, &staff, Action::OpsPublishRateIndex).await?;
    let city = b.city.trim().to_lowercase();
    let (recs, _) = st
        .db
        .query_recs::<RateIndexEntry>(
            kabados_db::Query::new(kabados_db::Table::Core, kabados_db::keys::rate_city(&city))
                .index(kabados_db::Index::Gsi1)
                .begins_with(format!("{}#", b.iso_week))
                .limit(500),
        )
        .await?;
    let mut out = Vec::new();
    for r in recs {
        let mut e = r.data;
        if e.deals >= MIN_DEALS && e.vendors >= MIN_VENDORS {
            e.published = true;
            st.db.put_rate(e.clone()).await?;
            out.push(e);
        }
    }
    st.db
        .audit(&format!("RATE#{city}"), &staff.0.sub, "RATE_INDEX_PUBLISHED", serde_json::json!({"week": b.iso_week, "entries": out.len()}))
        .await?;
    Ok(Json(out))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OpsConfig {
    pub env: String,
    pub otp_enabled: bool,
    pub first_look_minutes: i64,
    pub platform_fee_bps: i64,
    #[schema(value_type = Object)]
    pub provider_modes: kabados_runtime::ProviderModes,
    pub checked_at: chrono::DateTime<chrono::Utc>,
}

#[utoipa::path(get, path = "/v1/ops/config", tag = "ops", responses((status = 200, body = OpsConfig)))]
pub async fn config(State(st): State<AppState>, staff: Staff) -> ApiResult<Json<OpsConfig>> {
    guard(&st, &staff, Action::OpsManageConfig).await?;
    Ok(Json(OpsConfig {
        env: st.cfg.env.clone(),
        otp_enabled: st.cfg.otp_enabled,
        first_look_minutes: st.cfg.first_look_minutes,
        platform_fee_bps: st.cfg.platform_fee_bps,
        provider_modes: st.cfg.modes.clone(),
        checked_at: now(),
    }))
}

#[utoipa::path(get, path = "/v1/ops/audit/{subject}", tag = "ops", responses((status = 200, body = Vec<AuditEntry>)))]
pub async fn audit(State(st): State<AppState>, staff: Staff, Path(subject): Path<String>) -> ApiResult<Json<Vec<AuditEntry>>> {
    guard(&st, &staff, Action::OpsViewFraud).await?;
    if subject.contains('#') && !subject.starts_with("CAT#") && !subject.starts_with("RATE#") {
        return Err(ApiError::bad("invalid subject"));
    }
    Ok(Json(st.db.audit_log(&subject).await?))
}
