//! Transporters and jobs (PLAN.md §7.5, §8.4).

use super::{id_action, now, parse};
use crate::access::*;
use crate::auth::Me;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use chrono::{DateTime, Duration, Utc};
use kabados_authz::{Action, Resource};
use kabados_db::Record;
use kabados_db::repo::{Notification, NotificationKind};
use kabados_domain::Paise;
use kabados_domain::deal::{Deal, DealStatus};
use kabados_domain::events::DomainEvent;
use kabados_domain::geo::LatLng;
use kabados_domain::ids::*;
use kabados_domain::job::*;
use kabados_domain::state::StateMachine;
use kabados_domain::user::*;
use kabados_integrations::routes::Position;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub const DEFAULT_ACCEPT_TIMEOUT_SECS: u32 = 120;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/vendor/transporters", get(my_transporters))
        .route("/v1/vendor/transporters:invite", post(invite))
        .route("/v1/deals/{id}/jobs", post(create_job))
        .route("/v1/transporter/availability", put(availability))
        .route("/v1/transporter/device", put(device))
        .route("/v1/transporter/jobs", get(transporter_jobs))
        .route("/v1/transporter/job-offers", get(job_offers))
        .route("/v1/jobs/{id}", get(get_job).post(job_action))
        .route("/v1/jobs/{id}/events", post(job_event))
        .route("/v1/jobs/{id}/positions", post(positions))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TransporterCard {
    pub transporter_id: UserId,
    pub display_name: String,
    pub fare: Option<FareTemplate>,
    pub on_duty: bool,
    pub vehicle: Option<Vehicle>,
    pub rating: f64,
    pub acceptance: f64,
    pub eta_s: Option<i64>,
    pub distance_m: Option<i64>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct TransportersQuery {
    /// Rank by ETA to this deal's pickup point.
    pub deal_id: Option<DealId>,
}

/// "My transporters", ranked by ETA (Amazon Location route matrix), rating and fit.
#[utoipa::path(get, path = "/v1/vendor/transporters", tag = "logistics", params(TransportersQuery), responses((status = 200, body = Vec<TransporterCard>)))]
pub async fn my_transporters(
    State(st): State<AppState>,
    me: Me,
    Query(q): Query<TransportersQuery>,
) -> ApiResult<Json<Vec<TransporterCard>>> {
    vendor_of(&st, &me).await?;
    let mut cards = Vec::new();
    let mut origins = Vec::new();
    for m in st.db.my_transporters(me.id()).await?.into_iter().filter(|m| m.data.status == TrustStatus::Active) {
        let Some(t) = st.db.transporter(&m.data.transporter_id).await? else { continue };
        let pos = st
            .ix
            .tracker
            .latest(m.data.transporter_id.as_str())
            .await
            .ok()
            .flatten()
            .map(|p| LatLng { lat: p.lat, lng: p.lng })
            .or(t.data.base_location);
        origins.push(pos);
        cards.push(TransporterCard {
            transporter_id: m.data.transporter_id.clone(),
            display_name: m.data.display_name.clone(),
            fare: m.data.fare.clone(),
            on_duty: t.data.on_duty,
            vehicle: t.data.vehicles.first().cloned(),
            rating: t.data.rating.stars(),
            acceptance: t.data.acceptance.smoothed(),
            eta_s: None,
            distance_m: None,
        });
    }
    if let Some(deal_id) = q.deal_id {
        let deal = st.db.deal(&deal_id).await?.ok_or(ApiError::NotFound("deal"))?;
        if deal.data.vendor_id != *me.id() {
            return Err(ApiError::NotFound("deal"));
        }
        let l = st.db.listing(&deal.data.listing_id).await?.ok_or(ApiError::NotFound("listing"))?;
        let known: Vec<(usize, LatLng)> = origins.iter().enumerate().filter_map(|(i, o)| o.map(|p| (i, p))).collect();
        if !known.is_empty() {
            let m = st.ix.routes.matrix(&known.iter().map(|(_, p)| *p).collect::<Vec<_>>(), &[l.data.location]).await.unwrap_or_default();
            for (row, (i, _)) in m.iter().zip(known.iter()) {
                if let Some(Some(leg)) = row.first() {
                    cards[*i].eta_s = Some(leg.duration_s);
                    cards[*i].distance_m = Some(leg.distance_m);
                }
            }
        }
    }
    cards.sort_by(|a, b| {
        (!a.on_duty, a.eta_s.unwrap_or(i64::MAX)).cmp(&(!b.on_duty, b.eta_s.unwrap_or(i64::MAX))).then(b.rating.total_cmp(&a.rating))
    });
    Ok(Json(cards))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct Invite {
    pub phone: String,
    pub display_name: String,
    pub fare: Option<FareTemplate>,
}

/// Adds a transporter (who has signed up in the app) to the vendor's trusted list.
#[utoipa::path(post, path = "/v1/vendor/transporters:invite", tag = "logistics", request_body = Invite, responses((status = 201, body = MyTransporter)))]
pub async fn invite(State(st): State<AppState>, me: Me, Json(b): Json<Invite>) -> ApiResult<(StatusCode, Json<MyTransporter>)> {
    vendor_of(&st, &me).await?;
    let phone = normalize_in_phone(&b.phone).ok_or_else(|| ApiError::bad("enter a valid Indian mobile number"))?;
    let uid = st
        .db
        .user_by_phone_hash(&st.hasher.phone(&phone))
        .await?
        .ok_or_else(|| ApiError::bad("ask them to install the KabadOS transporter app and sign up first"))?;
    if st.db.transporter(&uid).await?.is_none() {
        return Err(ApiError::bad("this person hasn't set up a transporter account yet"));
    }
    if uid == *me.id() {
        return Err(ApiError::bad("you can't add yourself"));
    }
    let m = MyTransporter {
        vendor_id: me.id().clone(),
        transporter_id: uid,
        display_name: b.display_name.trim().chars().take(60).collect(),
        fare: b.fare,
        status: TrustStatus::Active,
        invited_at: now(),
    };
    st.db.put_my_transporter(m.clone()).await?;
    Ok((StatusCode::CREATED, Json(m)))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct NewJob {
    pub dispatch: Dispatch,
    /// Ranked transporters (sequential offers go down this list on timeout/decline).
    pub transporter_ids: Vec<UserId>,
    pub fare: Paise,
    pub accept_timeout_secs: Option<u32>,
}

/// Offers the pickup to the vendor's transporters (PLAN.md §7.5).
#[utoipa::path(post, path = "/v1/deals/{id}/jobs", tag = "logistics", request_body = NewJob, responses((status = 201, body = Job)))]
pub async fn create_job(
    State(st): State<AppState>,
    me: Me,
    Path(id): Path<String>,
    Json(b): Json<NewJob>,
) -> ApiResult<(StatusCode, Json<Job>)> {
    let deal_id: DealId = parse(&id, "deal id")?;
    let v = vendor_of(&st, &me).await?;
    let mut deal = st.db.deal(&deal_id).await?.ok_or(ApiError::NotFound("deal"))?;
    ensure(&st, &vendor_principal(&v.data), Action::AssignTransporter, &Resource::Deal(deal_attrs(&deal.data))).await?;
    if b.transporter_ids.is_empty() || b.transporter_ids.len() > 10 {
        return Err(ApiError::bad("choose 1–10 transporters"));
    }
    if b.fare.0 < 0 || b.fare.0 > 50_000_00 {
        return Err(ApiError::bad("fare out of range"));
    }
    let trusted: Vec<UserId> = st
        .db
        .my_transporters(me.id())
        .await?
        .into_iter()
        .filter(|m| m.data.status == TrustStatus::Active)
        .map(|m| m.data.transporter_id)
        .collect();
    if let Some(t) = b.transporter_ids.iter().find(|t| !trusted.contains(t)) {
        return Err(ApiError::bad(format!("{t} is not in your transporter list")));
    }
    if let Some(j) = &deal.data.job_id
        && let Some(existing) = st.db.job(j).await?
        && !existing.data.status.is_terminal()
    {
        return Err(ApiError::conflict("this deal already has an active job"));
    }
    let listing =
        st.db.listing_bundle(&deal.data.listing_id, Some(deal.data.listing_version)).await?.ok_or(ApiError::NotFound("listing"))?;
    let timeout = b.accept_timeout_secs.unwrap_or(DEFAULT_ACCEPT_TIMEOUT_SECS).clamp(30, 1800);
    let offered_to = match b.dispatch {
        Dispatch::Sequential => vec![b.transporter_ids[0].clone()],
        Dispatch::Broadcast => b.transporter_ids.clone(),
    };
    let job = Job {
        id: JobId::new(),
        deal_id: deal_id.clone(),
        vendor_id: me.id().clone(),
        status: JobStatus::Offered,
        dispatch: b.dispatch,
        candidates: b.transporter_ids.clone(),
        offered_to: offered_to.clone(),
        transporter_id: None,
        fare: b.fare,
        fare_history: vec![],
        load_weight: listing.listing.data.total_est_weight_g,
        bulky_items: listing.items.iter().filter(|i| i.data.unit == kabados_domain::taxonomy::Unit::Piece).map(|i| i.data.quantity).sum(),
        floors: listing.version.data.pickup.floor,
        pickup_cell_r8: listing.listing.data.approx.h3_r8.clone(),
        offer_expires_at: now() + Duration::seconds(timeout as i64),
        accept_timeout_secs: timeout,
        created_at: now(),
        updated_at: now(),
    };
    if deal.data.status == DealStatus::Created {
        deal.data.status = deal.data.status.transition(DealStatus::PickupScheduled)?;
    }
    if deal.data.status != DealStatus::PickupScheduled {
        return Err(ApiError::conflict("transport can only be arranged before pickup"));
    }
    deal.data.job_id = Some(job.id.clone());
    deal.data.transporter_fare = Some(b.fare);
    let notes = offered_to
        .iter()
        .map(|t| {
            Notification::new(
                t.clone(),
                NotificationKind::JobOffer,
                "New pickup job",
                format!("{} · {} · fare {}", v.data.business_name, job.load_weight, job.fare),
            )
            .link(format!("/job?id={}", job.id))
            .data(serde_json::json!({"job_id": job.id, "expires_at": job.offer_expires_at}))
        })
        .collect();
    let ev = DomainEvent::JobOffered { job_id: job.id.clone(), deal_id: deal_id.clone(), transporter_ids: offered_to };
    let rec = st.db.create_job(&mut deal, job, vec![ev], notes).await?;
    Ok((StatusCode::CREATED, Json(rec.data)))
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct Availability {
    pub on_duty: bool,
}

/// "On duty" toggle; only on-duty transporters receive offers (PLAN.md §8.4).
#[utoipa::path(put, path = "/v1/transporter/availability", tag = "logistics", request_body = Availability, responses((status = 200, body = Availability)))]
pub async fn availability(State(st): State<AppState>, me: Me, Json(b): Json<Availability>) -> ApiResult<Json<Availability>> {
    let mut t = transporter_of(&st, &me).await?;
    t.data.on_duty = b.on_duty;
    st.db.save_transporter(&mut t).await?;
    Ok(Json(b))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeviceReg {
    pub device_id: String,
    pub platform: String,
    pub fcm_token: Option<String>,
    pub integrity_token: Option<String>,
}

/// Registers the device for push (SNS → FCM) and records its integrity verdict.
#[utoipa::path(put, path = "/v1/transporter/device", tag = "logistics", request_body = DeviceReg, responses((status = 200, body = Device)))]
pub async fn device(State(st): State<AppState>, me: Me, Json(b): Json<DeviceReg>) -> ApiResult<Json<Device>> {
    transporter_of(&st, &me).await?;
    if b.device_id.is_empty() || b.device_id.len() > 128 || b.device_id.contains('#') {
        return Err(ApiError::bad("invalid device id"));
    }
    let verdict = match &b.integrity_token {
        Some(t) => {
            let v = st.ix.integrity.verify(t).await?;
            if !v.meets_device_integrity() {
                return Err(ApiError::forbidden("this device failed the integrity check"));
            }
            Some(v.device_integrity.join(","))
        }
        None => None,
    };
    let endpoint = match &b.fcm_token {
        Some(tok) => Some(st.ix.push.register(tok, me.id().as_str()).await?),
        None => None,
    };
    let d =
        Device { device_id: b.device_id, platform: b.platform, push_endpoint_arn: endpoint, integrity_verdict: verdict, updated_at: now() };
    st.db.put_device(me.id(), d.clone()).await?;
    Ok(Json(d))
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct JobsQuery {
    pub status: Option<JobStatus>,
}

#[utoipa::path(get, path = "/v1/transporter/jobs", tag = "logistics", params(JobsQuery), responses((status = 200, body = Vec<Job>)))]
pub async fn transporter_jobs(State(st): State<AppState>, me: Me, Query(q): Query<JobsQuery>) -> ApiResult<Json<Vec<Job>>> {
    transporter_of(&st, &me).await?;
    Ok(Json(st.db.transporter_jobs(me.id(), q.status).await?.into_iter().map(|r| r.data).collect()))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct JobOfferView {
    pub job: Job,
    pub vendor_name: String,
    pub vendor_rating: f64,
    /// Approximate pickup area (exact after acceptance).
    pub pickup_area: LatLng,
    pub expires_at: DateTime<Utc>,
}

/// Incoming job offers (approximate area only until accepted).
#[utoipa::path(get, path = "/v1/transporter/job-offers", tag = "logistics", responses((status = 200, body = Vec<JobOfferView>)))]
pub async fn job_offers(State(st): State<AppState>, me: Me) -> ApiResult<Json<Vec<JobOfferView>>> {
    transporter_of(&st, &me).await?;
    let mut out = Vec::new();
    for o in st.db.transporter_job_offers(me.id()).await? {
        if o.expires_at < now() {
            continue;
        }
        let Some(job) = st.db.job(&o.job_id).await? else { continue };
        if job.data.status != JobStatus::Offered || !job.data.offered_to.contains(me.id()) {
            continue;
        }
        let vendor = st.db.vendor(&job.data.vendor_id).await?;
        out.push(JobOfferView {
            pickup_area: kabados_domain::geo::cell_center(&job.data.pickup_cell_r8)?,
            vendor_name: vendor.as_ref().map(|v| v.data.business_name.clone()).unwrap_or_default(),
            vendor_rating: vendor.map(|v| v.data.rating.stars()).unwrap_or(0.0),
            expires_at: job.data.offer_expires_at,
            job: job.data,
        });
    }
    Ok(Json(out))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct JobView {
    pub job: Job,
    pub deal_status: DealStatus,
    pub exact_address: Option<crate::views::ExactAddress>,
    pub pickup_area: LatLng,
    pub yard: Option<LatLng>,
    pub events: Vec<JobEvent>,
}

async fn load_job(st: &AppState, id: &str) -> ApiResult<(Record<Job>, Record<Deal>)> {
    let jid: JobId = parse(id, "job id")?;
    let job = st.db.job(&jid).await?.ok_or(ApiError::NotFound("job"))?;
    let deal = st.db.deal(&job.data.deal_id).await?.ok_or(ApiError::NotFound("deal"))?;
    Ok((job, deal))
}

#[utoipa::path(get, path = "/v1/jobs/{id}", tag = "logistics", responses((status = 200, body = JobView)))]
pub async fn get_job(State(st): State<AppState>, me: Me, Path(id): Path<String>) -> ApiResult<Json<JobView>> {
    let (job, deal) = load_job(&st, &id).await?;
    let res = Resource::Job(job_attrs(&job.data, &deal.data));
    let principal = match (st.db.transporter(me.id()).await?, deal.data.vendor_id == *me.id()) {
        (_, true) => vendor_principal(&vendor_of(&st, &me).await?.data),
        (Some(t), false) => transporter_principal(&t.data),
        (None, false) => seller(me.id()),
    };
    ensure(&st, &principal, Action::ViewJob, &res).await?;
    let listing = st.db.listing(&deal.data.listing_id).await?.ok_or(ApiError::NotFound("listing"))?;
    let lattrs = listing_attrs(&listing.data, Some(&deal.data), Some(&job.data), None);
    let exact = if allowed(&st, &principal, Action::ViewExactAddress, &Resource::Listing(lattrs)).await? {
        crate::routes::listings::exact_address(&st, &listing.data).await?
    } else {
        None
    };
    let children = st.db.deal_children(&deal.data.id).await?;
    let yard = st.db.vendor(&deal.data.vendor_id).await?.map(|v| v.data.yard_location);
    Ok(Json(JobView {
        pickup_area: listing.data.approx.center,
        deal_status: deal.data.status,
        exact_address: exact,
        yard,
        events: children.job_events.into_iter().map(|e| e.data).collect(),
        job: job.data,
    }))
}

/// `POST /v1/jobs/{id}:accept | :decline`.
#[utoipa::path(post, path = "/v1/jobs/{id}:accept", tag = "logistics", responses((status = 200, body = Job)))]
pub async fn job_action(State(st): State<AppState>, me: Me, Path(seg): Path<String>) -> ApiResult<Json<Job>> {
    let (jid, action): (JobId, String) = id_action(&seg, "job id")?;
    let (mut job, mut deal) = load_job(&st, jid.as_str()).await?;
    let t = transporter_of(&st, &me).await?;
    let p = transporter_principal(&t.data);
    let res = Resource::Job(job_attrs(&job.data, &deal.data));
    match action.as_str() {
        "accept" => {
            ensure(&st, &p, Action::AcceptJob, &res).await?;
            if job.data.offer_expires_at < now() {
                return Err(ApiError::conflict("this offer has expired"));
            }
            let clear = st.db.clear_job_offer_ops(&job.data);
            job.data.status = job.data.status.transition(JobStatus::Accepted)?;
            job.data.transporter_id = Some(me.id().clone());
            deal.data.transporter_id = Some(me.id().clone());
            deal.data.status = deal.data.status.transition(DealStatus::JobAssigned)?;
            let job_op = st.db.save_job_op(&mut job)?;
            let mut extra = vec![job_op];
            extra.extend(clear);
            let n = Notification::new(
                deal.data.vendor_id.clone(),
                NotificationKind::JobAccepted,
                "Transporter on the way",
                format!("{} accepted the pickup", me.user.data.name.clone().unwrap_or_default()),
            )
            .link(format!("/deal?id={}", deal.data.id));
            let ns = Notification::new(
                deal.data.seller_id.clone(),
                NotificationKind::PickupEta,
                "Pickup assigned",
                "A transporter has accepted your pickup.",
            )
            .link(format!("/deal?id={}", deal.data.id));
            let accepted = DomainEvent::JobAccepted { job_id: jid.clone(), deal_id: deal.data.id.clone(), transporter_id: me.id().clone() };
            st.db
                .save_deal_with(&mut deal, extra, vec![accepted], vec![n, ns])
                .await
                .map_err(|e| if e.is_conflict() { ApiError::conflict("someone else accepted this job first") } else { e.into() })?;
            let mut t = t;
            t.data.acceptance.record(1000);
            let _ = st.db.save_transporter(&mut t).await;
        }
        "decline" => {
            ensure(&st, &p, Action::DeclineJob, &res).await?;
            let clear = st.db.clear_job_offer_ops(&job.data);
            job.data.offered_to.retain(|x| x != me.id());
            let mut next_notes = vec![];
            if job.data.offered_to.is_empty() {
                // Sequential dispatch moves to the next candidate (PLAN.md §4.5).
                let pos = job.data.candidates.iter().position(|c| c == me.id()).map(|i| i + 1).unwrap_or(usize::MAX);
                match job.data.candidates.get(pos).cloned() {
                    Some(next) if job.data.dispatch == Dispatch::Sequential => {
                        job.data.offered_to = vec![next.clone()];
                        job.data.offer_expires_at = now() + Duration::seconds(job.data.accept_timeout_secs as i64);
                        next_notes.push(
                            Notification::new(next, NotificationKind::JobOffer, "New pickup job", format!("Fare {}", job.data.fare))
                                .link(format!("/job?id={}", job.data.id)),
                        );
                    }
                    _ => job.data.status = job.data.status.transition(JobStatus::Declined)?,
                }
            }
            let mut extra = clear;
            extra.extend(st.db.job_offer_index_ops(&job.data)?);
            if job.data.status == JobStatus::Declined {
                next_notes.push(
                    Notification::new(
                        deal.data.vendor_id.clone(),
                        NotificationKind::JobOffer,
                        "No transporter accepted",
                        "Offer the job again or pick up yourself.",
                    )
                    .link(format!("/deal?id={}", deal.data.id)),
                );
            }
            st.db.save_job(&mut job, extra, vec![], next_notes).await?;
            let mut t = t;
            t.data.acceptance.record(0);
            let _ = st.db.save_transporter(&mut t).await;
        }
        _ => return Err(ApiError::NotFound("action")),
    }
    let (job, _) = load_job(&st, jid.as_str()).await?;
    Ok(Json(job.data))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct NewJobEvent {
    pub kind: JobEventKind,
    pub device_at: Option<DateTime<Utc>>,
    pub lat: Option<f64>,
    pub lng: Option<f64>,
    #[serde(default)]
    pub image_ids: Vec<ImageId>,
    pub note: Option<String>,
    pub mock_location: Option<bool>,
    /// For FARE_CHANGED: the new fare agreed on a call.
    pub fare: Option<Paise>,
}

/// Checklist step: each is timestamped and GPS-tagged (PLAN.md §8.4).
#[utoipa::path(post, path = "/v1/jobs/{id}/events", tag = "logistics", request_body = NewJobEvent, responses((status = 200, body = Job)))]
pub async fn job_event(State(st): State<AppState>, me: Me, Path(id): Path<String>, Json(b): Json<NewJobEvent>) -> ApiResult<Json<Job>> {
    let (mut job, mut deal) = load_job(&st, &id).await?;
    let res = Resource::Job(job_attrs(&job.data, &deal.data));
    let is_vendor = deal.data.vendor_id == *me.id();
    let p = if is_vendor {
        vendor_principal(&vendor_of(&st, &me).await?.data)
    } else {
        transporter_principal(&transporter_of(&st, &me).await?.data)
    };
    ensure(&st, &p, Action::UpdateJob, &res).await?;
    if b.mock_location == Some(true) {
        st.db.audit(deal.data.id.as_str(), me.id().as_str(), "MOCK_LOCATION_DETECTED", serde_json::json!({"job": job.data.id})).await?;
    }
    let event = JobEvent {
        kind: b.kind,
        at: now(),
        device_at: b.device_at,
        lat: b.lat,
        lng: b.lng,
        image_ids: b.image_ids,
        note: b.note.map(|n| n.chars().take(300).collect()),
        mock_location: b.mock_location,
    };
    let mut extra = vec![st.db.job_event_op(&deal.data.id, &job.data.id, &event)?];
    let mut events = vec![];
    let mut notes = vec![];
    if b.kind == JobEventKind::FareChanged {
        let to = b.fare.ok_or_else(|| ApiError::bad("fare is required"))?;
        job.data.fare_history.push(kabados_domain::job::FareChange {
            from: job.data.fare,
            to,
            by: me.id().clone(),
            at: now(),
            note: event.note.clone(),
        });
        job.data.fare = to;
        deal.data.transporter_fare = Some(to);
    }
    if let Some(next) = b.kind.target_status() {
        job.data.status = job.data.status.transition(next)?;
        match next {
            JobStatus::ArrivedPickup => {
                deal.data.status = deal.data.status.transition(DealStatus::AtPickup)?;
                events.push(DomainEvent::ArrivedAtPickup { deal_id: deal.data.id.clone(), job_id: Some(job.data.id.clone()) });
                notes.push(
                    Notification::new(
                        deal.data.seller_id.clone(),
                        NotificationKind::Arrived,
                        "Pickup has arrived",
                        "Your pickup is at the door.",
                    )
                    .link(format!("/deal?id={}", deal.data.id)),
                );
            }
            JobStatus::InTransit if deal.data.status == DealStatus::HandedOver => {
                deal.data.status = deal.data.status.transition(DealStatus::InTransit)?;
            }
            JobStatus::ArrivedYard => {
                notes.push(
                    Notification::new(
                        deal.data.vendor_id.clone(),
                        NotificationKind::Arrived,
                        "Load arrived at your yard",
                        "Confirm receipt to settle the deal.",
                    )
                    .link(format!("/deal?id={}", deal.data.id)),
                );
            }
            _ => {}
        }
    }
    let job_op = st.db.save_job_op(&mut job)?;
    extra.push(job_op);
    st.db.save_deal_with(&mut deal, extra, events, notes).await?;
    Ok(Json(job.data))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct Positions {
    pub positions: Vec<Position>,
}

/// Batched GPS → Amazon Location tracker; only while the job shares location.
#[utoipa::path(post, path = "/v1/jobs/{id}/positions", tag = "logistics", request_body = Positions, responses((status = 204)))]
pub async fn positions(State(st): State<AppState>, me: Me, Path(id): Path<String>, Json(b): Json<Positions>) -> ApiResult<StatusCode> {
    let (job, deal) = load_job(&st, &id).await?;
    let t = transporter_of(&st, &me).await?;
    ensure(&st, &transporter_principal(&t.data), Action::PostJobPosition, &Resource::Job(job_attrs(&job.data, &deal.data))).await?;
    if !job.data.status.shares_location() {
        return Err(ApiError::conflict("location is only shared during an active job"));
    }
    if b.positions.is_empty() || b.positions.len() > 100 {
        return Err(ApiError::bad("send 1–100 positions"));
    }
    for p in &b.positions {
        LatLng::new(p.lat, p.lng)?;
    }
    st.ix.tracker.update(me.id().as_str(), &b.positions).await?;
    if let Some(last) = b.positions.iter().max_by_key(|p| p.at) {
        st.ix
            .realtime
            .publish(
                &format!("job/{}", job.data.id),
                &[serde_json::json!({"type": "position", "lat": last.lat, "lng": last.lng, "at": last.at})],
            )
            .await
            .ok();
    }
    Ok(StatusCode::NO_CONTENT)
}
