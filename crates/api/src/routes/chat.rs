//! Pre-deal and deal chat (PLAN.md §7.4, §11.9, §17.2).

use super::{PageQuery, now, parse};
use crate::access::*;
use crate::auth::Me;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use kabados_authz::{Action, Principal, Resource, ThreadAttrs};
use kabados_db::Record;
use kabados_db::repo::{Notification, NotificationKind};
use kabados_domain::chat::*;
use kabados_domain::ids::*;
use kabados_domain::redact::redact;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/threads", get(my_threads).post(open_thread))
        .route("/v1/threads/{id}/messages", get(messages).post(post_message))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct OpenThread {
    pub listing_id: ListingId,
    /// Required when the seller opens a thread with a vendor who offered.
    pub vendor_id: Option<UserId>,
}

#[utoipa::path(post, path = "/v1/threads", tag = "chat", request_body = OpenThread, responses((status = 200, body = Thread)))]
pub async fn open_thread(State(st): State<AppState>, me: Me, Json(b): Json<OpenThread>) -> ApiResult<Json<Thread>> {
    let l = st.db.listing(&b.listing_id).await?.ok_or(ApiError::NotFound("listing"))?.data;
    let (seller_id, vendor_id) = if l.seller_id == *me.id() {
        let v = b.vendor_id.ok_or_else(|| ApiError::bad("choose the vendor to chat with"))?;
        if !st.db.offers_for_listing(&l.id).await?.iter().any(|o| o.data.vendor_id == v) {
            return Err(ApiError::bad("you can chat with vendors who made an offer"));
        }
        (l.seller_id.clone(), v)
    } else {
        let v = vendor_of(&st, &me).await?;
        let (deal, job, drive) = listing_context(&st, &l).await?;
        ensure(
            &st,
            &vendor_principal(&v.data),
            Action::ViewListing,
            &Resource::Listing(listing_attrs(&l, deal.as_ref(), job.as_ref(), drive.as_ref())),
        )
        .await?;
        (l.seller_id.clone(), me.id().clone())
    };
    let t = Thread {
        id: ThreadId::new(),
        listing_id: l.id.clone(),
        seller_id,
        vendor_id,
        deal_id: l.deal_id.clone(),
        created_at: now(),
        last_message_at: None,
    };
    Ok(Json(st.db.thread_for(t).await?.data))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ThreadSummary {
    pub thread: Thread,
    pub listing_title: String,
    pub counterpart: Option<String>,
}

#[utoipa::path(get, path = "/v1/threads", tag = "chat", responses((status = 200, body = Vec<ThreadSummary>)))]
pub async fn my_threads(State(st): State<AppState>, me: Me) -> ApiResult<Json<Vec<ThreadSummary>>> {
    let mut out = Vec::new();
    for idx in st.db.user_threads(me.id()).await? {
        let Some(t) = st.db.thread(&idx.thread_id).await? else { continue };
        let title = st.db.listing(&t.data.listing_id).await?.map(|l| l.data.title).unwrap_or_default();
        let counterpart = if t.data.seller_id == *me.id() {
            st.db.vendor(&t.data.vendor_id).await?.map(|v| v.data.business_name)
        } else {
            st.db.user(&t.data.seller_id).await?.and_then(|u| u.data.name)
        };
        out.push(ThreadSummary { thread: t.data, listing_title: title, counterpart });
    }
    Ok(Json(out))
}

async fn thread_access(st: &AppState, me: &Me, id: &str, action: Action) -> ApiResult<Record<Thread>> {
    let id: ThreadId = parse(id, "thread id")?;
    let t = st.db.thread(&id).await?.ok_or(ApiError::NotFound("thread"))?;
    let l = st.db.listing(&t.data.listing_id).await?.ok_or(ApiError::NotFound("listing"))?;
    let deal_open = match &l.data.deal_id {
        Some(d) => st.db.deal(d).await?.is_some_and(|d| d.data.status.is_open()),
        None => false,
    };
    let attrs = ThreadAttrs {
        id: t.data.id.clone(),
        seller: t.data.seller_id.clone(),
        vendor: t.data.vendor_id.clone(),
        listing_status: l.data.status,
        deal_open,
    };
    let p: Principal = if t.data.seller_id == *me.id() {
        seller(me.id())
    } else if t.data.vendor_id == *me.id() {
        vendor_principal(&vendor_of(st, me).await?.data)
    } else {
        return Err(ApiError::NotFound("thread"));
    };
    ensure(st, &p, action, &Resource::Thread(attrs)).await?;
    Ok(t)
}

#[utoipa::path(get, path = "/v1/threads/{id}/messages", tag = "chat", params(PageQuery), responses((status = 200, body = Vec<Message>)))]
pub async fn messages(
    State(st): State<AppState>,
    me: Me,
    Path(id): Path<String>,
    Query(q): Query<PageQuery>,
) -> ApiResult<Json<Vec<Message>>> {
    let t = thread_access(&st, &me, &id, Action::ViewThread).await?;
    let (recs, _) = st.db.messages(&t.data.id, q.cursor, 100).await?;
    // Quarantined messages are visible only to their sender.
    Ok(Json(recs.into_iter().map(|r| r.data).filter(|m| m.moderation != Moderation::Quarantined || m.sender_id == *me.id()).collect()))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct NewMessage {
    pub text: String,
}

/// Server-side moderation, then delivery on `thread/{id}` (PLAN.md §17.2).
#[utoipa::path(post, path = "/v1/threads/{id}/messages", tag = "chat", request_body = NewMessage, responses((status = 201, body = Message)))]
pub async fn post_message(
    State(st): State<AppState>,
    me: Me,
    Path(id): Path<String>,
    Json(b): Json<NewMessage>,
) -> ApiResult<(StatusCode, Json<Message>)> {
    let mut t = thread_access(&st, &me, &id, Action::PostMessage).await?;
    let text = b.text.trim();
    if text.is_empty() || text.chars().count() > 2000 {
        return Err(ApiError::bad("messages must be 1–2,000 characters"));
    }
    if !st.db.rate_limit("chat", me.id().as_str(), 60, 30).await? {
        return Err(ApiError::RateLimited);
    }
    let pre_deal = t.data.deal_id.is_none();
    let r = redact(text);
    let check = st.ai.classify_text(text).await.unwrap_or_default();
    let (moderation, warning) = if check.abusive || check.prompt_attack {
        (Moderation::Quarantined, Some("This message is held for review.".to_owned()))
    } else if pre_deal && r.found_contact_details() {
        (
            Moderation::SoftWarning,
            Some("For your safety, keep contact and payment details inside KabadOS until a deal is agreed.".to_owned()),
        )
    } else {
        (Moderation::Clean, None)
    };
    let m = Message {
        id: MessageId::new(),
        thread_id: t.data.id.clone(),
        sender_id: me.id().clone(),
        text: text.to_owned(),
        moderation,
        warning,
        at: now(),
    };
    let rec = st.db.post_message(&mut t, m).await?;
    if moderation != Moderation::Quarantined {
        let other = if t.data.seller_id == *me.id() { t.data.vendor_id.clone() } else { t.data.seller_id.clone() };
        st.ix
            .realtime
            .publish(&format!("thread/{}", t.data.id), &[serde_json::to_value(&rec.data).map_err(anyhow::Error::from)?])
            .await
            .ok();
        let n = Notification::new(other, NotificationKind::ChatMessage, "New message", r.text.chars().take(80).collect::<String>())
            .link(format!("/chat?id={}", t.data.id));
        st.db.store.transact(vec![kabados_db::repo::notify_op(&kabados_db::keys::thread(&t.data.id), n)?]).await?;
    } else {
        st.db
            .audit(
                t.data.id.as_str(),
                me.id().as_str(),
                "CHAT_QUARANTINED",
                serde_json::json!({"message": rec.data.id, "reasons": check.reasons}),
            )
            .await?;
    }
    Ok((StatusCode::CREATED, Json(rec.data)))
}
