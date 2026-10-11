//! Pooled collection drives (PLAN.md §9).

use super::{id_action, now, parse};
use crate::auth::Me;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use chrono::NaiveDate;
use kabados_domain::Grams;
use kabados_domain::deal::LineStatus;
use kabados_domain::drive::*;
use kabados_domain::geo::LatLng;
use kabados_domain::ids::*;
use kabados_domain::user::Role;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use utoipa::ToSchema;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/drives", get(my_drives).post(create_drive))
        .route("/v1/drives/{id}", get(get_drive).post(drive_action))
        .route("/v1/drives/{id}/report", get(report))
        .route("/v1/drives/by-code/{code}", get(by_code))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct NewDrive {
    pub name: String,
    #[schema(value_type = String, format = Date)]
    pub date: NaiveDate,
    pub window: String,
    pub collection_point: LatLng,
    pub collection_note: Option<String>,
    pub city: String,
    #[serde(default)]
    pub categories: Vec<String>,
    pub recurrence: Recurrence,
}

fn join_code() -> String {
    const A: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut r = rand::rng();
    (0..8).map(|_| A[r.random_range(0..A.len())] as char).collect()
}

#[utoipa::path(post, path = "/v1/drives", tag = "drives", request_body = NewDrive, responses((status = 201, body = Drive)))]
pub async fn create_drive(State(st): State<AppState>, me: Me, Json(b): Json<NewDrive>) -> ApiResult<(StatusCode, Json<Drive>)> {
    if !me.has_role(Role::SocietyAdmin) {
        return Err(ApiError::forbidden("set up a society admin role first"));
    }
    b.collection_point.validate()?;
    if kabados_domain::ruleset::parse_window(&b.window).is_none() {
        return Err(ApiError::bad("window must be HH:MM-HH:MM"));
    }
    if b.date < now().date_naive() {
        return Err(ApiError::bad("the drive date is in the past"));
    }
    let tax = st.taxonomy().await?;
    if let Some(c) = b.categories.iter().find(|c| tax.get(c).is_none() && !kabados_domain::taxonomy::is_valid_glob(c)) {
        return Err(ApiError::bad(format!("unknown category {c}")));
    }
    let d = Drive {
        id: DriveId::new(),
        admin_id: me.id().clone(),
        name: b.name.trim().chars().take(80).collect(),
        date: b.date,
        window: b.window,
        collection_point: b.collection_point,
        collection_note: b.collection_note.map(|n| n.chars().take(300).collect()),
        h3_r7: b.collection_point.cell_r7()?,
        h3_r8: b.collection_point.cell_r8()?,
        city: b.city.trim().to_lowercase(),
        categories: b.categories,
        status: DriveStatus::Open,
        recurrence: b.recurrence,
        join_code: join_code(),
        member_count: 0,
        pool_weight: Grams(0),
        listing_id: None,
        created_at: now(),
    };
    Ok((StatusCode::CREATED, Json(st.db.create_drive(d).await?.data)))
}

#[utoipa::path(get, path = "/v1/drives", tag = "drives", responses((status = 200, body = Vec<Drive>)))]
pub async fn my_drives(State(st): State<AppState>, me: Me) -> ApiResult<Json<Vec<Drive>>> {
    Ok(Json(st.db.admin_drives(me.id()).await?.into_iter().map(|r| r.data).collect()))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DrivePublic {
    pub id: DriveId,
    pub name: String,
    #[schema(value_type = String, format = Date)]
    pub date: NaiveDate,
    pub window: String,
    pub collection_note: Option<String>,
    pub categories: Vec<String>,
    pub member_count: u32,
    pub pool_weight: Grams,
    pub status: DriveStatus,
}

impl From<&Drive> for DrivePublic {
    fn from(d: &Drive) -> Self {
        Self {
            id: d.id.clone(),
            name: d.name.clone(),
            date: d.date,
            window: d.window.clone(),
            collection_note: d.collection_note.clone(),
            categories: d.categories.clone(),
            member_count: d.member_count,
            pool_weight: d.pool_weight,
            status: d.status,
        }
    }
}

/// Drive details for the join page (link or QR).
#[utoipa::path(get, path = "/v1/drives/by-code/{code}", tag = "drives", responses((status = 200, body = DrivePublic)))]
pub async fn by_code(State(st): State<AppState>, _me: Me, Path(code): Path<String>) -> ApiResult<Json<DrivePublic>> {
    if code.len() != 8 || !code.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(ApiError::NotFound("drive"));
    }
    let d = st.db.drive_by_code(&code.to_ascii_uppercase()).await?.ok_or(ApiError::NotFound("drive"))?;
    Ok(Json(DrivePublic::from(&d.data)))
}

#[utoipa::path(get, path = "/v1/drives/{id}", tag = "drives", responses((status = 200, body = Drive)))]
pub async fn get_drive(State(st): State<AppState>, me: Me, Path(id): Path<String>) -> ApiResult<Json<serde_json::Value>> {
    let id: DriveId = parse(&id, "drive id")?;
    let d = st.db.drive(&id).await?.ok_or(ApiError::NotFound("drive"))?;
    if d.data.admin_id == *me.id() {
        return Ok(Json(serde_json::to_value(&d.data).map_err(anyhow::Error::from)?));
    }
    let member = st.db.drive_members(&id).await?.iter().any(|m| m.data.seller_id == *me.id());
    if !member {
        return Err(ApiError::NotFound("drive"));
    }
    Ok(Json(serde_json::to_value(DrivePublic::from(&d.data)).map_err(anyhow::Error::from)?))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct JoinDrive {
    pub code: String,
    pub listing_id: ListingId,
}

/// `POST /v1/drives/{id}:join` (resident attaches a listing) and `:close` (admin).
#[utoipa::path(post, path = "/v1/drives/{id}:join", tag = "drives", request_body = JoinDrive, responses((status = 200, body = DrivePublic)))]
pub async fn drive_action(
    State(st): State<AppState>,
    me: Me,
    Path(seg): Path<String>,
    body: Option<Json<JoinDrive>>,
) -> ApiResult<Json<DrivePublic>> {
    let (id, action): (DriveId, String) = id_action(&seg, "drive id")?;
    let mut d = st.db.drive(&id).await?.ok_or(ApiError::NotFound("drive"))?;
    match action.as_str() {
        "join" => {
            let b = body.ok_or_else(|| ApiError::bad("missing body"))?.0;
            if !b.code.eq_ignore_ascii_case(&d.data.join_code) || d.data.status != DriveStatus::Open {
                return Err(ApiError::forbidden("this join code is not valid"));
            }
            let mut l = st.db.listing(&b.listing_id).await?.ok_or(ApiError::NotFound("listing"))?;
            if l.data.seller_id != *me.id() {
                return Err(ApiError::NotFound("listing"));
            }
            l.data.drive_id = Some(id.clone());
            let member = DriveMember {
                drive_id: id.clone(),
                listing_id: l.data.id.clone(),
                seller_id: me.id().clone(),
                est_weight: l.data.total_est_weight_g,
                joined_at: now(),
            };
            d.data.member_count += 1;
            d.data.pool_weight += l.data.total_est_weight_g;
            let ops = vec![st.db.drive_member_op(&member)?, st.db.save_drive_op(&mut d)?, st.db.save_listing_op(&mut l)?];
            st.db
                .store
                .transact(ops)
                .await
                .map_err(|e| if e.is_conflict() { ApiError::conflict("already joined, or the drive changed; retry") } else { e.into() })?;
        }
        "close" => {
            if d.data.admin_id != *me.id() {
                return Err(ApiError::forbidden("only the drive admin can close it"));
            }
            d.data.status = DriveStatus::Closed;
            let op = st.db.save_drive_op(&mut d)?;
            st.db.store.transact(vec![op]).await?;
        }
        _ => return Err(ApiError::NotFound("action")),
    }
    Ok(Json(DrivePublic::from(&d.data)))
}

/// Drive report: kg by category, hazardous kg to authorized vendors, participation.
#[utoipa::path(get, path = "/v1/drives/{id}/report", tag = "drives", responses((status = 200, body = DriveReport)))]
pub async fn report(State(st): State<AppState>, me: Me, Path(id): Path<String>) -> ApiResult<Json<DriveReport>> {
    let id: DriveId = parse(&id, "drive id")?;
    let d = st.db.drive(&id).await?.ok_or(ApiError::NotFound("drive"))?;
    if d.data.admin_id != *me.id() {
        return Err(ApiError::forbidden("only the drive admin can see the report"));
    }
    let tax = st.taxonomy().await?;
    let mut by_cat: BTreeMap<String, Grams> = BTreeMap::new();
    let mut hazardous = Grams(0);
    let members = st.db.drive_members(&id).await?;
    for m in &members {
        let Some(l) = st.db.listing(&m.data.listing_id).await? else { continue };
        let Some(deal_id) = l.data.deal_id else { continue };
        let Some(deal) = st.db.deal(&deal_id).await? else { continue };
        let authorized = st.db.vendor(&deal.data.vendor_id).await?.is_some_and(|v| v.data.hazardous_auth_valid(now()));
        for line in st.db.deal_children(&deal_id).await?.lines.into_iter().map(|r| r.data).filter(|l| l.status == LineStatus::Final) {
            let w = line.weighed_g.unwrap_or_default();
            *by_cat.entry(line.category_id.clone()).or_default() += w;
            if authorized && tax.is_hazardous(&line.category_id) {
                hazardous += w;
            }
        }
    }
    let total = by_cat.values().copied().sum();
    Ok(Json(DriveReport {
        drive_id: id,
        participants: members.len() as u32,
        kg_by_category: by_cat.into_iter().collect(),
        hazardous_to_authorized: hazardous,
        total_weight: total,
    }))
}
