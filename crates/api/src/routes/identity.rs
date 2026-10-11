//! Identity context: me, roles, addresses, payout methods, consents,
//! notifications, data rights, KYC documents and the Places proxy.

use super::{now, parse, split_action};
use crate::access::vendor_of;
use crate::auth::{Caller, Me};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::views::*;
use axum::extract::{Path, Query, State};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use kabados_db::Record;
use kabados_db::repo::{Notification, OtpChannel};
use kabados_domain::events::DomainEvent;
use kabados_domain::geo::LatLng;
use kabados_domain::ids::*;
use kabados_domain::user::*;
use kabados_integrations::blob::PresignedUpload;
use kabados_integrations::places::Place;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use utoipa::ToSchema;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/me", get(get_me).patch(patch_me).delete(delete_me))
        .route("/v1/me/roles/{seg}", post(apply_role))
        .route("/v1/me/addresses", get(list_addresses).post(add_address))
        .route("/v1/me/addresses/{id}", delete(delete_address))
        .route("/v1/me/payout-methods", get(list_payouts).post(add_payout))
        .route("/v1/me/consents", get(list_consents).post(set_consent))
        .route("/v1/me/notifications", get(notifications))
        .route("/v1/me/data-export", post(data_export))
        .route("/v1/me/documents", get(list_documents).post(add_document))
        .route("/v1/me/{seg}", post(me_action))
        .route("/v1/vendor/authorizations", post(add_authorization))
        .route("/v1/auth/otp-channel", post(otp_channel))
        .route("/v1/places/autocomplete", get(autocomplete))
        .route("/v1/places/geocode", get(geocode))
        .route("/v1/places/reverse", get(reverse))
}

pub async fn me_view(st: &AppState, user: &UserProfile) -> ApiResult<MeView> {
    let vendor = st.db.vendor(&user.id).await?.map(|v| VendorView::from(&v.data));
    let transporter = st.db.transporter(&user.id).await?.map(|t| TransporterView::from(&t.data));
    Ok(MeView {
        id: user.id.clone(),
        name: user.name.clone(),
        phone_last4: user.phone_last4.clone(),
        roles: user.roles.clone(),
        adult_confirmed: user.adult_confirmed,
        honesty_score: user.honesty.smoothed(),
        vendor,
        transporter,
    })
}

#[utoipa::path(get, path = "/v1/me", tag = "me", responses((status = 200, body = MeView)))]
pub async fn get_me(State(st): State<AppState>, me: Me) -> ApiResult<Json<MeView>> {
    Ok(Json(me_view(&st, &me.user.data).await?))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PatchMe {
    pub name: Option<String>,
    /// Must be true to use the service (18+ only, PLAN.md §5.3).
    pub adult_confirmed: Option<bool>,
}

#[utoipa::path(patch, path = "/v1/me", tag = "me", request_body = PatchMe, responses((status = 200, body = MeView)))]
pub async fn patch_me(State(st): State<AppState>, me: Me, Json(body): Json<PatchMe>) -> ApiResult<Json<MeView>> {
    let mut u = me.user;
    if let Some(n) = body.name {
        let n = n.trim();
        if n.is_empty() || n.len() > 80 {
            return Err(ApiError::bad("name must be 1–80 characters"));
        }
        u.data.name = Some(n.to_owned());
    }
    if let Some(a) = body.adult_confirmed {
        if !a {
            return Err(ApiError::bad("KabadOS is for adults (18+) only"));
        }
        u.data.adult_confirmed = true;
    }
    st.db.save_user(&mut u).await?;
    Ok(Json(me_view(&st, &u.data).await?))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ApplyVendor {
    pub business_name: String,
    pub yard_location: LatLng,
    pub city: String,
    #[serde(default)]
    pub categories: BTreeSet<String>,
    pub pan: Option<String>,
    pub gstin: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ApplyTransporter {
    pub vehicle_type: VehicleType,
    pub payload_kg: u32,
    pub registration: String,
    pub licence_number: Option<String>,
    pub base_location: Option<LatLng>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum ApplyRole {
    Vendor(ApplyVendor),
    Transporter(ApplyTransporter),
    Empty {},
}

/// `POST /v1/me/roles/{role}:apply`. KYC verification is skipped by product
/// decision: applications are approved immediately.
#[utoipa::path(post, path = "/v1/me/roles/{role}:apply", tag = "me", params(("role" = String, Path, description = "VENDOR, TRANSPORTER or SOCIETY_ADMIN")),
    request_body = ApplyRole, responses((status = 200, body = MeView)))]
pub async fn apply_role(
    State(st): State<AppState>,
    me: Me,
    Path(seg): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult<Json<MeView>> {
    let (role, action) = split_action(&seg);
    if action != Some("apply") {
        return Err(ApiError::bad("use /v1/me/roles/{role}:apply"));
    }
    let role = Role::parse(role).ok_or_else(|| ApiError::bad("unknown role"))?;
    if !me.user.data.adult_confirmed {
        return Err(ApiError::bad("confirm you are 18 or older first"));
    }
    let tax = st.taxonomy().await?;
    match role {
        Role::Seller => {}
        Role::SocietyAdmin => {}
        Role::Vendor => {
            let b: ApplyVendor = serde_json::from_value(body).map_err(|e| ApiError::bad(format!("vendor application: {e}")))?;
            b.yard_location.validate()?;
            if b.business_name.trim().is_empty() {
                return Err(ApiError::bad("business name is required"));
            }
            if let Some(c) = b.categories.iter().find(|c| tax.get(c).is_none()) {
                return Err(ApiError::bad(format!("unknown category {c}")));
            }
            if st.db.vendor(me.id()).await?.is_some() {
                return Err(ApiError::conflict("you already have a vendor account"));
            }
            for (kind, number) in [(DocumentKind::Pan, b.pan.as_deref()), (DocumentKind::Gstin, b.gstin.as_deref())] {
                if number.is_some() {
                    let _ = st.ix.kyc.verify(kind, number, me.user.data.name.as_deref()).await?;
                }
            }
            st.db
                .insert_vendor(VendorProfile {
                    user_id: me.id().clone(),
                    business_name: b.business_name.trim().to_owned(),
                    yard_location: b.yard_location,
                    yard_h3_r8: b.yard_location.cell_r8()?,
                    city: b.city.trim().to_lowercase(),
                    kyc_status: KycStatus::Approved,
                    kyc_submitted_at: Some(now()),
                    categories: b.categories,
                    hazardous_auth: None,
                    suspended: false,
                    rating: Default::default(),
                    reliability: Default::default(),
                    response_minutes_p50: None,
                    active_rule_set: None,
                    payment_ref: None,
                    created_at: now(),
                })
                .await?;
        }
        Role::Transporter => {
            let b: ApplyTransporter = serde_json::from_value(body).map_err(|e| ApiError::bad(format!("transporter application: {e}")))?;
            if st.db.transporter(me.id()).await?.is_some() {
                return Err(ApiError::conflict("you already have a transporter account"));
            }
            let reg = b.registration.trim().to_ascii_uppercase();
            let _ = st.ix.kyc.verify(DocumentKind::VehicleRc, Some(&reg), None).await?;
            if let Some(dl) = &b.licence_number {
                let _ = st.ix.kyc.verify(DocumentKind::DrivingLicence, Some(dl), None).await?;
            }
            let masked = if reg.len() > 4 { format!("{}****{}", &reg[..2], &reg[reg.len() - 4..]) } else { reg.clone() };
            st.db
                .insert_transporter(TransporterProfile {
                    user_id: me.id().clone(),
                    kyc_status: KycStatus::Approved,
                    kyc_submitted_at: Some(now()),
                    on_duty: false,
                    vehicles: vec![Vehicle {
                        id: VehicleId::new(),
                        vehicle_type: b.vehicle_type,
                        payload_kg: b.payload_kg,
                        registration_masked: masked,
                        registration: Some(st.sealer.seal(&reg).await?),
                    }],
                    base_location: b.base_location,
                    rating: Default::default(),
                    acceptance: Default::default(),
                    on_time: Default::default(),
                    suspended: false,
                    created_at: now(),
                })
                .await?;
        }
    }
    let mut u = me.user;
    if u.data.roles.insert(role) {
        st.db.save_user(&mut u).await?;
    }
    Ok(Json(me_view(&st, &u.data).await?))
}

async fn address_view(st: &AppState, a: &Address) -> ApiResult<AddressView> {
    let landmark = match &a.landmark {
        Some(l) => Some(st.sealer.open(l).await?),
        None => None,
    };
    Ok(AddressView {
        id: a.id.clone(),
        label: a.label.clone(),
        location: a.location,
        city: a.city.clone(),
        text: st.sealer.open(&a.text).await?,
        landmark,
    })
}

#[utoipa::path(get, path = "/v1/me/addresses", tag = "me", responses((status = 200, body = Vec<AddressView>)))]
pub async fn list_addresses(State(st): State<AppState>, me: Me) -> ApiResult<Json<Vec<AddressView>>> {
    let mut out = Vec::new();
    for a in st.db.addresses(me.id()).await? {
        out.push(address_view(&st, &a.data).await?);
    }
    Ok(Json(out))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct NewAddress {
    pub label: String,
    pub location: LatLng,
    pub text: String,
    pub landmark: Option<String>,
    /// Optional; derived by reverse geocoding when absent.
    pub city: Option<String>,
}

#[utoipa::path(post, path = "/v1/me/addresses", tag = "me", request_body = NewAddress, responses((status = 201, body = AddressView)))]
pub async fn add_address(
    State(st): State<AppState>,
    me: Me,
    Json(b): Json<NewAddress>,
) -> ApiResult<(axum::http::StatusCode, Json<AddressView>)> {
    b.location.validate()?;
    if b.text.trim().len() < 5 || b.text.len() > 500 {
        return Err(ApiError::bad("enter the full address (5–500 characters)"));
    }
    let city = match b.city {
        Some(c) => c.trim().to_lowercase(),
        None => st.ix.places.reverse(b.location).await.ok().flatten().and_then(|p| p.city).unwrap_or_else(|| "unknown".into()),
    };
    let landmark = match b.landmark.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(l) => Some(st.sealer.seal(l).await?),
        None => None,
    };
    let a = Address {
        id: AddressId::new(),
        label: b.label.trim().chars().take(40).collect(),
        location: b.location,
        h3_r7: b.location.cell_r7()?,
        h3_r8: b.location.cell_r8()?,
        city,
        text: st.sealer.seal(b.text.trim()).await?,
        landmark,
        created_at: now(),
    };
    st.db.put_address(me.id(), a.clone()).await?;
    Ok((axum::http::StatusCode::CREATED, Json(address_view(&st, &a).await?)))
}

#[utoipa::path(delete, path = "/v1/me/addresses/{id}", tag = "me", responses((status = 204)))]
pub async fn delete_address(State(st): State<AppState>, me: Me, Path(id): Path<String>) -> ApiResult<axum::http::StatusCode> {
    let id: AddressId = parse(&id, "address id")?;
    st.db.delete_address(me.id(), &id).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PayoutView {
    pub id: PayoutMethodId,
    pub kind: PayoutKind,
    pub display: String,
    pub validation: PayoutValidation,
    pub validated_name: Option<String>,
}

#[utoipa::path(get, path = "/v1/me/payout-methods", tag = "me", responses((status = 200, body = Vec<PayoutView>)))]
pub async fn list_payouts(State(st): State<AppState>, me: Me) -> ApiResult<Json<Vec<PayoutView>>> {
    Ok(Json(
        st.db
            .payout_methods(me.id())
            .await?
            .into_iter()
            .map(|p| PayoutView {
                id: p.data.id,
                kind: p.data.kind,
                display: p.data.display,
                validation: p.data.validation,
                validated_name: p.data.validated_name,
            })
            .collect(),
    ))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct NewPayout {
    pub vpa: String,
}

/// Adds a UPI ID and validates it before the first payout (PLAN.md §6.3).
#[utoipa::path(post, path = "/v1/me/payout-methods", tag = "me", request_body = NewPayout, responses((status = 201, body = PayoutView)))]
pub async fn add_payout(
    State(st): State<AppState>,
    me: Me,
    Json(b): Json<NewPayout>,
) -> ApiResult<(axum::http::StatusCode, Json<PayoutView>)> {
    let vpa = b.vpa.trim().to_lowercase();
    if !is_valid_vpa(&vpa) {
        return Err(ApiError::bad("enter a valid UPI ID, like name@bank"));
    }
    let fav = st.ix.payouts.validate_vpa(&vpa, me.id().as_str()).await?;
    let p = PayoutMethod {
        id: PayoutMethodId::new(),
        kind: PayoutKind::Upi,
        display: mask_vpa(&vpa),
        value: st.sealer.seal(&vpa).await?,
        validation: if fav.valid { PayoutValidation::Validated } else { PayoutValidation::Failed },
        validated_name: fav.registered_name,
        provider_ref: fav.fund_account_id,
        created_at: now(),
    };
    st.db.put_payout_method(me.id(), p.clone()).await?;
    Ok((
        axum::http::StatusCode::CREATED,
        Json(PayoutView { id: p.id, kind: p.kind, display: p.display, validation: p.validation, validated_name: p.validated_name }),
    ))
}

#[utoipa::path(get, path = "/v1/me/consents", tag = "me", responses((status = 200, body = Vec<Consent>)))]
pub async fn list_consents(State(st): State<AppState>, me: Me) -> ApiResult<Json<Vec<Consent>>> {
    Ok(Json(st.db.consents(me.id()).await?.into_iter().map(|r| r.data).collect()))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetConsent {
    pub purpose: ConsentPurpose,
    pub granted: bool,
    pub notice_version: String,
}

/// Records a DPDP consent decision (separate, withdrawable per purpose).
#[utoipa::path(post, path = "/v1/me/consents", tag = "me", request_body = SetConsent, responses((status = 200, body = Consent)))]
pub async fn set_consent(State(st): State<AppState>, me: Me, Json(b): Json<SetConsent>) -> ApiResult<Json<Consent>> {
    let c = Consent { purpose: b.purpose, notice_version: b.notice_version, granted: b.granted, at: now() };
    st.db.put_consent(me.id(), c.clone()).await?;
    Ok(Json(c))
}

#[utoipa::path(get, path = "/v1/me/notifications", tag = "me", responses((status = 200, body = Vec<Notification>)))]
pub async fn notifications(State(st): State<AppState>, me: Me) -> ApiResult<Json<Vec<Notification>>> {
    use kabados_db::{Index, Query, Table, keys};
    let (recs, _) = st
        .db
        .query_recs::<Notification>(
            Query::new(Table::Core, keys::user(me.id())).index(Index::Gsi2).begins_with("NOTIFY#").reverse().limit(50),
        )
        .await?;
    Ok(Json(recs.into_iter().map(|r| r.data).collect()))
}

/// DPDP data export: everything stored about the caller, decrypted for them.
#[utoipa::path(post, path = "/v1/me/data-export", tag = "me", responses((status = 200, body = Object)))]
pub async fn data_export(State(st): State<AppState>, me: Me) -> ApiResult<Json<serde_json::Value>> {
    let mut addresses = Vec::new();
    for a in st.db.addresses(me.id()).await? {
        addresses.push(address_view(&st, &a.data).await?);
    }
    let (listings, _) = st.db.seller_listings(me.id(), None, 500).await?;
    let (deals, _) = st.db.user_deals(me.id(), None, None).await?;
    Ok(Json(serde_json::json!({
        "profile": me.user.data,
        "addresses": addresses,
        "consents": st.db.consents(me.id()).await?.into_iter().map(|r| r.data).collect::<Vec<_>>(),
        "vendor": st.db.vendor(me.id()).await?.map(|r| r.data),
        "transporter": st.db.transporter(me.id()).await?.map(|r| r.data),
        "listings": listings.into_iter().map(|r| ListingSummary::from(&r.data)).collect::<Vec<_>>(),
        "deals": deals,
        "exported_at": now(),
    })))
}

/// Erasure (DPDP): removes addresses, payout methods and devices, and
/// anonymizes the profile; financial records are kept under legal hold.
#[utoipa::path(delete, path = "/v1/me", tag = "me", responses((status = 204)))]
pub async fn delete_me(State(st): State<AppState>, me: Me) -> ApiResult<axum::http::StatusCode> {
    use kabados_db::{Table, keys};
    for a in st.db.addresses(me.id()).await? {
        st.db.delete_address(me.id(), &a.data.id).await?;
    }
    for p in st.db.payout_methods(me.id()).await? {
        st.db.store.delete(Table::Core, &keys::user(me.id()), &keys::payout(&p.data.id), None).await?;
    }
    for d in st.db.devices(me.id()).await? {
        st.db.store.delete(Table::Core, &keys::user(me.id()), &keys::device(&d.data.device_id), None).await?;
    }
    let uid = me.id().clone();
    let mut u = me.user;
    u.data.name = None;
    u.data.suspended = true;
    st.db.save_user(&mut u).await?;
    st.db.audit(uid.as_str(), uid.as_str(), "ERASURE_REQUESTED", serde_json::json!({})).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PresignDocument {
    pub kind: DocumentKind,
    pub content_type: String,
    pub size: u64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DocumentUpload {
    pub document_key: String,
    pub upload: PresignedUpload,
}

/// `POST /v1/me/documents:presign` (KYC and authorization documents).
pub async fn me_action(
    State(st): State<AppState>,
    me: Me,
    Path(seg): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult<Json<serde_json::Value>> {
    match seg.as_str() {
        "documents:presign" => {
            let b: PresignDocument = serde_json::from_value(body).map_err(|e| ApiError::bad(e.to_string()))?;
            if !["image/jpeg", "image/png", "image/webp", "application/pdf"].contains(&b.content_type.as_str())
                || b.size == 0
                || b.size > 10 * 1024 * 1024
            {
                return Err(ApiError::bad("upload a JPEG, PNG, WebP or PDF under 10 MB"));
            }
            let id = DocumentId::new();
            let ext = b.content_type.rsplit('/').next().unwrap_or("bin");
            let key = format!("kyc/{}/{id}.{ext}", me.id());
            let upload = st.ix.blobs.presign_put(&st.cfg.documents_bucket, &key, &b.content_type, b.size, None).await?;
            Ok(Json(serde_json::to_value(DocumentUpload { document_key: key, upload }).map_err(anyhow::Error::from)?))
        }
        _ => Err(ApiError::NotFound("action")),
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct NewDocument {
    pub kind: DocumentKind,
    pub document_key: String,
    pub number: Option<String>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[utoipa::path(post, path = "/v1/me/documents", tag = "me", request_body = NewDocument, responses((status = 201, body = KycDocument)))]
pub async fn add_document(
    State(st): State<AppState>,
    me: Me,
    Json(b): Json<NewDocument>,
) -> ApiResult<(axum::http::StatusCode, Json<KycDocument>)> {
    if !b.document_key.starts_with(&format!("kyc/{}/", me.id())) {
        return Err(ApiError::forbidden("document key does not belong to you"));
    }
    let check = st.ix.kyc.verify(b.kind, b.number.as_deref(), me.user.data.name.as_deref()).await?;
    let number = match b.number.as_deref() {
        Some(n) => Some(st.sealer.seal(n.trim()).await?),
        None => None,
    };
    let doc = KycDocument {
        id: DocumentId::new(),
        kind: b.kind,
        s3_key: b.document_key,
        extracted: Default::default(),
        number,
        check: Some(check),
        expires_at: b.expires_at,
        uploaded_at: now(),
    };
    st.db.put_document(me.id(), doc.clone()).await?;
    st.tasks.enqueue(crate::tasks::Task::ProcessDocument { user_id: me.id().clone(), document_id: doc.id.clone() }).await.ok();
    Ok((axum::http::StatusCode::CREATED, Json(doc)))
}

#[utoipa::path(get, path = "/v1/me/documents", tag = "me", responses((status = 200, body = Vec<KycDocument>)))]
pub async fn list_documents(State(st): State<AppState>, me: Me) -> ApiResult<Json<Vec<KycDocument>>> {
    Ok(Json(st.db.documents(me.id()).await?.into_iter().map(|r| r.data).collect()))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct NewAuthorization {
    pub authorization_no: String,
    pub state: String,
    pub valid_until: chrono::DateTime<chrono::Utc>,
    #[serde(default)]
    pub via_tie_up: bool,
    pub document_key: Option<String>,
}

/// Unlocks hazardous categories. Verification is skipped by product decision
/// (accepted on upload); expiry is still enforced and auto-locks categories.
#[utoipa::path(post, path = "/v1/vendor/authorizations", tag = "vendor", request_body = NewAuthorization, responses((status = 200, body = VendorView)))]
pub async fn add_authorization(State(st): State<AppState>, me: Me, Json(b): Json<NewAuthorization>) -> ApiResult<Json<VendorView>> {
    let mut v = vendor_of(&st, &me).await?;
    if b.valid_until <= now() {
        return Err(ApiError::bad("this authorization has already expired"));
    }
    if b.authorization_no.trim().is_empty() || b.authorization_no.contains('#') {
        return Err(ApiError::bad("enter the authorization number"));
    }
    v.data.hazardous_auth = Some(HazardousAuthorization {
        authorization_no: b.authorization_no.trim().to_owned(),
        state: b.state.trim().to_ascii_uppercase(),
        via_tie_up: b.via_tie_up,
        valid_until: b.valid_until,
        verified_by: Some("auto".into()),
        verified_at: Some(now()),
    });
    let ev = kabados_db::repo::outbox_op(
        &kabados_db::keys::user(me.id()),
        DomainEvent::VendorAuthorizationChanged { vendor_id: me.id().clone() },
        None,
    )?;
    v.data.user_id = me.id().clone();
    let mut rec: Record<VendorProfile> = v;
    rec.meta = kabados_db::Meta {
        version: rec.meta.version,
        created_at: rec.meta.created_at,
        updated_at: rec.meta.updated_at,
        ..kabados_db::repo::vendor_meta(&rec.data)
    };
    let op = rec.update_op(kabados_db::Table::Core)?;
    st.db.store.transact(vec![op, ev]).await?;
    Ok(Json(VendorView::from(&rec.data)))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct OtpChannelReq {
    pub phone: String,
    pub channel: OtpChannel,
}

/// Records the OTP channel preference before a resend (PLAN.md §5.1). OTP
/// delivery is currently disabled; the preference is stored for when it's on.
#[utoipa::path(post, path = "/v1/auth/otp-channel", tag = "auth", request_body = OtpChannelReq, responses((status = 204)))]
pub async fn otp_channel(State(st): State<AppState>, Json(b): Json<OtpChannelReq>) -> ApiResult<axum::http::StatusCode> {
    let phone = normalize_in_phone(&b.phone).ok_or_else(|| ApiError::bad("enter a valid Indian mobile number"))?;
    let hash = st.hasher.phone(&phone);
    if !st.db.rate_limit("otpch", &hash, 600, 5).await? {
        return Err(ApiError::RateLimited);
    }
    st.db.set_otp_channel(&hash, b.channel).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct PlaceQuery {
    pub q: Option<String>,
    pub lat: Option<f64>,
    pub lng: Option<f64>,
}

async fn places_limit(st: &AppState, caller: &Caller) -> ApiResult<()> {
    if st.db.rate_limit("places", &caller.sub, 60, 60).await? { Ok(()) } else { Err(ApiError::RateLimited) }
}

#[utoipa::path(get, path = "/v1/places/autocomplete", tag = "places", params(PlaceQuery), responses((status = 200, body = Vec<Place>)))]
pub async fn autocomplete(State(st): State<AppState>, caller: Caller, Query(q): Query<PlaceQuery>) -> ApiResult<Json<Vec<Place>>> {
    places_limit(&st, &caller).await?;
    let text = q.q.unwrap_or_default();
    if text.trim().len() < 2 {
        return Ok(Json(vec![]));
    }
    let bias = match (q.lat, q.lng) {
        (Some(lat), Some(lng)) => LatLng::new(lat, lng).ok(),
        _ => None,
    };
    Ok(Json(st.ix.places.autocomplete(text.trim(), bias).await?))
}

#[utoipa::path(get, path = "/v1/places/geocode", tag = "places", params(PlaceQuery), responses((status = 200, body = Vec<Place>)))]
pub async fn geocode(State(st): State<AppState>, caller: Caller, Query(q): Query<PlaceQuery>) -> ApiResult<Json<Vec<Place>>> {
    places_limit(&st, &caller).await?;
    Ok(Json(st.ix.places.geocode(q.q.unwrap_or_default().trim()).await?))
}

#[utoipa::path(get, path = "/v1/places/reverse", tag = "places", params(PlaceQuery), responses((status = 200, body = Option<Place>)))]
pub async fn reverse(State(st): State<AppState>, caller: Caller, Query(q): Query<PlaceQuery>) -> ApiResult<Json<Option<Place>>> {
    places_limit(&st, &caller).await?;
    let p = LatLng::new(q.lat.unwrap_or(f64::NAN), q.lng.unwrap_or(f64::NAN))?;
    Ok(Json(st.ix.places.reverse(p).await?))
}
