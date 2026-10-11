//! Public, unauthenticated endpoints (PLAN.md §10.2, §17.2 "Public").

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use kabados_db::repo::{CityImpact, RateIndexEntry};
use kabados_domain::deal::GreenReceipt;
use kabados_domain::ids::ReceiptId;
use kabados_domain::taxonomy::Taxonomy;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/public/config", get(config))
        .route("/v1/public/taxonomy", get(taxonomy))
        .route("/v1/public/rate-index", get(rate_index))
        .route("/v1/public/impact", get(impact))
        .route("/v1/public/receipts/{id}", get(receipt))
        .route("/v1/public/openapi.json", get(openapi))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ClientConfig {
    pub env: String,
    pub otp_enabled: bool,
    pub first_look_minutes: i64,
    /// Razorpay public key for Checkout (test key in non-prod).
    pub razorpay_key_id: Option<String>,
    pub payments_simulated: bool,
    pub cities: Vec<String>,
}

#[utoipa::path(get, path = "/v1/public/config", tag = "public", responses((status = 200, body = ClientConfig)))]
pub async fn config(State(st): State<AppState>) -> ApiResult<Json<ClientConfig>> {
    let key = st.secrets.try_get("razorpay/key_id", "RAZORPAY_KEY_ID").await.filter(|k| k.starts_with("rzp_"));
    Ok(Json(ClientConfig {
        env: st.cfg.env.clone(),
        otp_enabled: st.cfg.otp_enabled,
        first_look_minutes: st.cfg.first_look_minutes,
        payments_simulated: st.cfg.modes.payments == kabados_runtime::Mode::Mock,
        razorpay_key_id: key,
        cities: ["delhi", "mumbai", "bengaluru", "chennai", "hyderabad", "kolkata", "pune"].map(String::from).to_vec(),
    }))
}

#[utoipa::path(get, path = "/v1/public/taxonomy", tag = "public", responses((status = 200, body = Taxonomy)))]
pub async fn taxonomy(State(st): State<AppState>) -> ApiResult<Json<Taxonomy>> {
    Ok(Json((*st.taxonomy().await?).clone()))
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct CityQuery {
    pub city: String,
    pub category: Option<String>,
}

/// Kabad Rate Index: weekly medians, published only above k-anonymity thresholds.
#[utoipa::path(get, path = "/v1/public/rate-index", tag = "public", params(CityQuery), responses((status = 200, body = Vec<RateIndexEntry>)))]
pub async fn rate_index(State(st): State<AppState>, Query(q): Query<CityQuery>) -> ApiResult<Json<Vec<RateIndexEntry>>> {
    let city = q.city.trim().to_lowercase();
    if city.is_empty() || city.contains('#') {
        return Err(ApiError::bad("city is required"));
    }
    let mut v = st.db.rate_index(&city, 500).await?;
    if let Some(c) = q.category {
        v.retain(|e| kabados_domain::taxonomy::glob_matches(&c, &e.category_id));
    }
    Ok(Json(v))
}

#[utoipa::path(get, path = "/v1/public/impact", tag = "public", params(CityQuery), responses((status = 200, body = Option<CityImpact>)))]
pub async fn impact(State(st): State<AppState>, Query(q): Query<CityQuery>) -> ApiResult<Json<Option<CityImpact>>> {
    Ok(Json(st.db.city_impact(&q.city.trim().to_lowercase()).await?))
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct SigQuery {
    pub sig: String,
}

/// Green Receipt verification by signed ID.
#[utoipa::path(get, path = "/v1/public/receipts/{id}", tag = "public", params(SigQuery), responses((status = 200, body = GreenReceipt)))]
pub async fn receipt(State(st): State<AppState>, Path(id): Path<String>, Query(q): Query<SigQuery>) -> ApiResult<Json<GreenReceipt>> {
    let id: ReceiptId = id.parse().map_err(|_| ApiError::NotFound("receipt"))?;
    if crate::settle::receipt_sig(&st, &id) != q.sig {
        return Err(ApiError::NotFound("receipt"));
    }
    Ok(Json(st.db.receipt(&id).await?.ok_or(ApiError::NotFound("receipt"))?.data))
}

pub async fn openapi() -> Json<utoipa::openapi::OpenApi> {
    Json(crate::openapi::spec())
}
