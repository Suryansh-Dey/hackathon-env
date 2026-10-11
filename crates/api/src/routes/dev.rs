//! Development helpers (never available in prod, PLAN.md §15.4): dev inbox,
//! local blob storage, dev sign-in tokens and demo seed data.

use crate::auth::Caller;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{FixedOffset, Utc};
use kabados_db::repo::InboxMessage;
use kabados_domain::geo::LatLng;
use kabados_domain::ids::*;
use kabados_domain::listing::Weekday;
use kabados_domain::ruleset::*;
use kabados_domain::user::*;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/dev/inbox", get(inbox))
        .route("/dev/blobs/{bucket}/{*key}", get(get_blob).put(put_blob))
        .route("/dev/token", post(token))
        .route("/dev/seed", post(seed))
}

fn non_prod(st: &AppState) -> ApiResult<()> {
    if st.cfg.is_prod() { Err(ApiError::NotFound("route")) } else { Ok(()) }
}

/// Staff tokens, seeding and the inbox need the admin key from Secrets
/// Manager (`/kabados/{env}/admin_key`). A local stack without one is open.
async fn require_admin(st: &AppState, given: Option<&str>) -> ApiResult<()> {
    match st.secrets.try_get("admin_key", "KABADOS_ADMIN_KEY").await {
        Some(key) if given.is_some_and(|g| st.hasher.verify("admin-key", g, &st.hasher.hash("admin-key", &key))) => Ok(()),
        None if st.cfg.is_local() => Ok(()),
        _ => Err(ApiError::forbidden("admin key required")),
    }
}

fn admin_header(headers: &HeaderMap) -> Option<&str> {
    headers.get("x-admin-key").and_then(|v| v.to_str().ok())
}

pub async fn inbox(State(st): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Vec<InboxMessage>>> {
    non_prod(&st)?;
    require_admin(&st, admin_header(&headers)).await?;
    Ok(Json(st.db.inbox(&st.cfg.env, 100).await?))
}

pub async fn put_blob(
    State(st): State<AppState>,
    Path((bucket, key)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<StatusCode> {
    if !st.cfg.is_local() {
        return Err(ApiError::NotFound("route"));
    }
    let ct = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("application/octet-stream");
    st.ix.blobs.put(&bucket, &key, body.to_vec(), ct).await?;
    Ok(StatusCode::OK)
}

pub async fn get_blob(State(st): State<AppState>, Path((bucket, key)): Path<(String, String)>) -> ApiResult<impl IntoResponse> {
    if !st.cfg.is_local() {
        return Err(ApiError::NotFound("route"));
    }
    let bytes = st.ix.blobs.get(&bucket, &key).await.map_err(|_| ApiError::NotFound("blob"))?;
    let ct = match key.rsplit('.').next() {
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        Some("pdf") => "application/pdf",
        Some("json") => "application/json",
        _ => "image/jpeg",
    };
    Ok(([(header::CONTENT_TYPE, ct)], bytes))
}

#[derive(Debug, Deserialize)]
pub struct TokenReq {
    pub phone: String,
    #[serde(default)]
    pub staff: bool,
    #[serde(default)]
    pub admin_key: Option<String>,
}

/// Phone-only sign-in without Cognito: `{"phone": "+9198…"}` → bearer token.
/// Staff tokens also need `admin_key`.
pub async fn token(State(st): State<AppState>, Json(b): Json<TokenReq>) -> ApiResult<Json<serde_json::Value>> {
    if st.cfg.is_prod() {
        return Err(ApiError::NotFound("route"));
    }
    if b.staff {
        require_admin(&st, b.admin_key.as_deref()).await?;
    }
    let phone = normalize_in_phone(&b.phone).ok_or_else(|| ApiError::bad("invalid phone"))?;
    let sub = format!("dev-{}", &st.hasher.phone(&phone)[..16]);
    let token = Caller::dev_token(&st.hasher, &sub, &phone, b.staff);
    Ok(Json(serde_json::json!({"token": token, "sub": sub, "phone": phone})))
}

/// Demo vendors with published rate cards around Delhi and Bengaluru.
pub async fn seed(State(st): State<AppState>, headers: HeaderMap) -> ApiResult<Json<serde_json::Value>> {
    non_prod(&st)?;
    require_admin(&st, admin_header(&headers)).await?;
    let demos = [
        ("+919800000001", "Raju Kabadiwala", "delhi", LatLng { lat: 28.6315, lng: 77.2167 }, false),
        ("+919800000002", "GreenCycle E-Waste (authorized)", "delhi", LatLng { lat: 28.5677, lng: 77.2433 }, true),
        ("+919800000003", "Metro Metals", "delhi", LatLng { lat: 28.6139, lng: 77.2090 }, false),
        ("+919800000004", "Bengaluru Paper Mart", "bengaluru", LatLng { lat: 12.9716, lng: 77.5946 }, false),
        ("+919800000005", "Koramangala E-Recyclers (authorized)", "bengaluru", LatLng { lat: 12.9352, lng: 77.6245 }, true),
    ];
    let mut created = Vec::new();
    for (phone, name, city, yard, hazardous) in demos {
        let hash = st.hasher.phone(phone);
        let uid = match st.db.user_by_phone_hash(&hash).await? {
            Some(u) => u,
            None => {
                let p = UserProfile {
                    id: UserId::new(),
                    cognito_sub: format!("dev-{}", &hash[..16]),
                    phone_hash: hash.clone(),
                    phone_last4: phone[phone.len() - 4..].into(),
                    name: Some(name.into()),
                    roles: BTreeSet::from([Role::Seller, Role::Vendor]),
                    adult_confirmed: true,
                    suspended: false,
                    honesty: Default::default(),
                    created_at: Utc::now(),
                };
                st.db.create_user(p).await?.data.id
            }
        };
        let mut v = match st.db.vendor(&uid).await? {
            Some(v) => v,
            None => {
                st.db
                    .insert_vendor(VendorProfile {
                        user_id: uid.clone(),
                        business_name: name.into(),
                        yard_location: yard,
                        yard_h3_r8: yard.cell_r8()?,
                        city: city.into(),
                        kyc_status: KycStatus::Approved,
                        kyc_submitted_at: Some(Utc::now()),
                        categories: BTreeSet::new(),
                        hazardous_auth: hazardous.then(|| HazardousAuthorization {
                            authorization_no: format!("DEMO-{}", &hash[..6]),
                            state: "DL".into(),
                            via_tie_up: false,
                            valid_until: Utc::now() + chrono::Duration::days(365),
                            verified_by: Some("seed".into()),
                            verified_at: Some(Utc::now()),
                        }),
                        suspended: false,
                        rating: Default::default(),
                        reliability: Default::default(),
                        response_minutes_p50: Some(20),
                        active_rule_set: None,
                        payment_ref: None,
                        created_at: Utc::now(),
                    })
                    .await?
            }
        };
        if st.db.latest_rule_set(&uid).await?.is_none() {
            let mut lines = vec![
                line("metal.copper", Some("clean"), Some(68_000), None),
                line("metal.copper", Some("mixed"), Some(58_000), None),
                line("metal.brass", None, Some(42_000), None),
                line("metal.aluminium", None, Some(14_000), None),
                line("metal.iron_steel", None, Some(3_200), None),
                line("paper.newspaper", None, Some(1_500), None),
                line("paper.cardboard", None, Some(1_100), None),
                line("paper.books_magazines", None, Some(1_200), None),
                line("plastic.pet_bottle", None, Some(1_800), None),
                line("plastic.hard_mixed", None, Some(1_000), None),
            ];
            if hazardous {
                lines.extend([
                    line("ewaste.itew.laptop", None, None, Some(45_000)),
                    line("ewaste.itew.mobile_phone", None, None, Some(8_000)),
                    line("ewaste.ceew.refrigerator", None, None, Some(150_000)),
                    line("ewaste.ceew.split_ac_outdoor", None, None, Some(250_000)),
                    line("ewaste.ceew.split_ac_indoor", None, None, Some(90_000)),
                    line("ewaste.ceew.washing_machine", None, None, Some(110_000)),
                    line("ewaste.ceew.tv_flat", None, None, Some(30_000)),
                    line("battery.lead_acid", None, Some(9_000), None),
                    line("battery.li_ion", None, Some(6_000), None),
                ]);
            }
            let rs = RuleSet {
                rule_set_id: RuleSetId::new(),
                vendor_id: uid.clone(),
                version: 1,
                status: RuleSetStatus::Draft,
                effective_from: (Utc::now() - chrono::Duration::days(1)).with_timezone(&FixedOffset::east_opt(19_800).expect("IST")),
                service_area: ServiceArea::Radius { center: yard, radius_km: 12.0 },
                schedule: Schedule {
                    days: vec![Weekday::Mon, Weekday::Tue, Weekday::Wed, Weekday::Thu, Weekday::Fri, Weekday::Sat, Weekday::Sun],
                    slots: vec!["09:00-13:00".into(), "14:00-19:00".into()],
                    daily_capacity_kg: Some(1000),
                },
                minimums: Minimums { order_value_paise: Some(5_000), weight_g: Some(1_000) },
                lines,
                modifiers: vec![
                    Modifier::VerifiedBonusPct { min_ladder: kabados_domain::ladder::LadderLevel::L2CloseUp, value: 3 },
                    Modifier::DistanceDeduction { free_km: 4, per_km_paise: 500 },
                    Modifier::BulkTier { category_glob: "paper.*".into(), threshold_g: 50_000, bonus_pct: 5 },
                ],
                exclusions: vec!["glass.*".into()],
                accept_partial_lots: true,
                source: Some("seed".into()),
            };
            st.db.publish_rule_set(&mut v, rs).await?;
        }
        crate::coverage::rebuild(&st, &uid).await?;
        created.push(serde_json::json!({"vendor": name, "phone": phone, "user_id": uid}));
    }
    Ok(Json(serde_json::json!({"vendors": created})))
}

fn line(cat: &str, grade: Option<&str>, per_kg: Option<i64>, per_piece: Option<i64>) -> RuleLine {
    RuleLine {
        category_id: cat.into(),
        grade: grade.map(Into::into),
        unit: None,
        price_per_kg_paise: per_kg,
        price_per_piece_paise: per_piece,
        min_weight_g: None,
        conditions: BTreeMap::new(),
    }
}
