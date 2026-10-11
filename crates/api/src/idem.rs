//! `Idempotency-Key` handling for POSTs (PLAN.md §17.1): the first response
//! for a key is stored for 24 h and replayed for retries with the same body.

use crate::auth::caller_from_parts;
use crate::error::ApiError;
use crate::state::AppState;
use axum::body::{Body, to_bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use sha2::{Digest, Sha256};

pub const HEADER: &str = "idempotency-key";
const MAX_BODY: usize = 1024 * 1024;

fn exempt(path: &str) -> bool {
    !path.starts_with("/v1/") || path.starts_with("/v1/auth/") || path.starts_with("/v1/public/")
}

fn valid_key(k: &str) -> bool {
    (8..=128).contains(&k.len()) && k.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_:.".contains(&b))
}

pub async fn idempotency(State(st): State<AppState>, req: Request, next: Next) -> Response {
    if req.method() != Method::POST || exempt(req.uri().path()) {
        return next.run(req).await;
    }
    let (parts, body) = req.into_parts();
    let Some(key) = parts.headers.get(HEADER).and_then(|v| v.to_str().ok()).map(str::to_owned) else {
        return ApiError::bad("Idempotency-Key header is required on POST requests").into_response();
    };
    if !valid_key(&key) {
        return ApiError::bad("Idempotency-Key must be 8–128 characters of [A-Za-z0-9-_:.]").into_response();
    }
    let scope = caller_from_parts(&parts, &st).map(|c| c.sub).unwrap_or_else(|| "anon".into());
    let bytes = match to_bytes(body, MAX_BODY).await {
        Ok(b) => b,
        Err(_) => return ApiError::bad("request body too large").into_response(),
    };
    let mut h = Sha256::new();
    h.update(parts.uri.path().as_bytes());
    h.update(b"\n");
    h.update(&bytes);
    let hash = hex::encode(h.finalize());
    let ttl = chrono::Duration::hours(24);
    let scope_key = format!("api:{scope}");
    match st.db.idem_begin(&scope_key, &key, &hash, ttl).await {
        Ok(kabados_db::repo::IdemState::New) => {}
        Ok(kabados_db::repo::IdemState::Done { status, body }) => {
            let mut res = (StatusCode::from_u16(status).unwrap_or(StatusCode::OK), axum::Json(body)).into_response();
            res.headers_mut().insert("idempotent-replay", HeaderValue::from_static("true"));
            return res;
        }
        Ok(kabados_db::repo::IdemState::InProgress) => {
            return ApiError::conflict("a request with this Idempotency-Key is still in progress").into_response();
        }
        Ok(kabados_db::repo::IdemState::Mismatch) => {
            return ApiError::bad("Idempotency-Key was already used with a different request").into_response();
        }
        Err(e) => return ApiError::from(e).into_response(),
    }
    let res = next.run(Request::from_parts(parts, Body::from(bytes))).await;
    let status = res.status();
    if status.is_server_error() {
        let _ = st.db.idem_abort(&scope_key, &key).await;
        return res;
    }
    let (rparts, rbody) = res.into_parts();
    let rbytes = to_bytes(rbody, 8 * MAX_BODY).await.unwrap_or_default();
    let stored: serde_json::Value = serde_json::from_slice(&rbytes).unwrap_or(serde_json::Value::Null);
    if let Err(e) = st.db.idem_complete(&scope_key, &key, &hash, status.as_u16(), stored, ttl).await {
        tracing::warn!(error = %e, "failed to store idempotent response");
    }
    let mut res = Response::from_parts(rparts, Body::from(rbytes));
    if !res.headers().contains_key(header::CONTENT_TYPE) {
        res.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    }
    res
}
