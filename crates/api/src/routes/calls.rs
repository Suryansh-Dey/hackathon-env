//! In-app audio calls over the Amazon Chime SDK; phone numbers are never
//! exchanged (PLAN.md §19.3).

use super::{id_action, now, parse};
use crate::auth::Me;
use crate::error::{ApiError, ApiResult};
use crate::routes::deals::deal_principal;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::routing::post;
use axum::{Json, Router};
use kabados_authz::{Action, Resource};
use kabados_db::repo::{Notification, NotificationKind};
use kabados_domain::deal::CallSession;
use kabados_domain::ids::*;
use serde::Serialize;
use utoipa::ToSchema;

pub fn router() -> Router<AppState> {
    Router::new().route("/v1/deals/{id}/calls", post(start_call)).route("/v1/calls/{seg}", post(call_action))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CallJoin {
    pub call_id: CallId,
    pub deal_id: DealId,
    /// Chime `Meeting` object for the client SDK.
    #[schema(value_type = Object)]
    pub meeting: serde_json::Value,
    /// Chime `Attendee` object (join token) for this caller.
    #[schema(value_type = Object)]
    pub attendee: serde_json::Value,
}

#[utoipa::path(post, path = "/v1/deals/{id}/calls", tag = "calls", responses((status = 200, body = CallJoin)))]
pub async fn start_call(State(st): State<AppState>, me: Me, Path(id): Path<String>) -> ApiResult<Json<CallJoin>> {
    let id: DealId = parse(&id, "deal id")?;
    let d = st.db.deal(&id).await?.ok_or(ApiError::NotFound("deal"))?;
    let (p, _) = deal_principal(&st, &me, &d.data).await?;
    crate::access::ensure(&st, &p, Action::StartCall, &Resource::Deal(crate::access::deal_attrs(&d.data))).await?;
    let call_id = CallId::new();
    let meeting = st.ix.calls.create_meeting(call_id.as_str()).await?;
    let meeting_id = meeting["Meeting"]["MeetingId"].as_str().unwrap_or_default().to_owned();
    let attendee = st.ix.calls.create_attendee(&meeting_id, me.id().as_str()).await?;
    let session = CallSession {
        id: call_id.clone(),
        deal_id: id.clone(),
        meeting_id,
        started_by: me.id().clone(),
        started_at: now(),
        ended_at: None,
    };
    let mut ops = vec![st.db.call_op(&session)?];
    for other in [&d.data.seller_id, &d.data.vendor_id].into_iter().chain(d.data.transporter_id.as_ref()).filter(|u| *u != me.id()) {
        let n = Notification::new(other.clone(), NotificationKind::Arrived, "Incoming call", "Tap to join the call about your pickup.")
            .link(format!("/call?id={call_id}"))
            .data(serde_json::json!({"call_id": call_id, "deal_id": id}));
        ops.push(kabados_db::repo::notify_op(&kabados_db::keys::deal(&id), n)?);
    }
    st.db.store.transact(ops).await?;
    st.ix.realtime.publish(&format!("deal/{id}"), &[serde_json::json!({"type": "call", "call_id": call_id})]).await.ok();
    Ok(Json(CallJoin { call_id, deal_id: id, meeting, attendee }))
}

/// `POST /v1/calls/{id}:join | :end`.
#[utoipa::path(post, path = "/v1/calls/{id}:join", tag = "calls", responses((status = 200, body = CallJoin)))]
pub async fn call_action(State(st): State<AppState>, me: Me, Path(seg): Path<String>) -> ApiResult<Json<serde_json::Value>> {
    let (call_id, action): (CallId, String) = id_action(&seg, "call id")?;
    let (deal, session) = find_call(&st, &me, &call_id).await?;
    let (p, _) = deal_principal(&st, &me, &deal).await?;
    crate::access::ensure(&st, &p, Action::StartCall, &Resource::Deal(crate::access::deal_attrs(&deal))).await?;
    match action.as_str() {
        "join" => {
            if session.ended_at.is_some() {
                return Err(ApiError::conflict("this call has ended"));
            }
            let attendee = st.ix.calls.create_attendee(&session.meeting_id, me.id().as_str()).await?;
            let meeting = serde_json::json!({"Meeting": {"MeetingId": session.meeting_id}});
            Ok(Json(serde_json::to_value(CallJoin { call_id, deal_id: deal.id, meeting, attendee }).map_err(anyhow::Error::from)?))
        }
        "end" => {
            st.ix.calls.end_meeting(&session.meeting_id).await.ok();
            let mut s = session;
            s.ended_at = Some(now());
            st.db.store.transact(vec![st.db.call_op(&s)?]).await?;
            Ok(Json(serde_json::json!({"ended": true})))
        }
        _ => Err(ApiError::NotFound("action")),
    }
}

async fn find_call(st: &AppState, me: &Me, id: &CallId) -> ApiResult<(kabados_domain::deal::Deal, CallSession)> {
    let (deals, _) = st.db.user_deals(me.id(), None, None).await?;
    for idx in deals {
        let c = st.db.deal_children(&idx.deal_id).await?;
        if let Some(s) = c.calls.into_iter().find(|c| &c.data.id == id)
            && let Some(d) = st.db.deal(&idx.deal_id).await?
        {
            return Ok((d.data, s.data));
        }
    }
    Err(ApiError::NotFound("call"))
}
