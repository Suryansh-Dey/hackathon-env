//! Request/response shapes that are defined in the API crate; mirrored here so
//! the client doesn't depend on the server crate. Kept in sync via the OpenAPI
//! contract check in CI.

use kabados_domain::Paise;
use kabados_domain::deal::{DealLine, DealStatus};
use kabados_domain::geo::LatLng;
use kabados_domain::ids::{DealId, UserId};
use kabados_domain::job::Job;
use kabados_domain::offer::PaymentMode;
use kabados_domain::user::{Role, VehicleType};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Availability {
    pub on_duty: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeviceReg {
    pub device_id: String,
    pub platform: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fcm_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integrity_token: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub lat: f64,
    pub lng: f64,
    pub at: chrono::DateTime<chrono::Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accuracy_m: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JobOfferView {
    pub job: Job,
    pub vendor_name: String,
    pub vendor_rating: f64,
    pub pickup_area: LatLng,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExactAddress {
    pub location: LatLng,
    pub text: String,
    pub landmark: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JobView {
    pub job: Job,
    pub deal_status: kabados_domain::deal::DealStatus,
    pub exact_address: Option<ExactAddress>,
    pub pickup_area: LatLng,
    pub yard: Option<LatLng>,
    pub events: Vec<kabados_domain::job::JobEvent>,
}

/// `GET /v1/me` (the fields the apps use; unknown fields are ignored).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeView {
    pub id: UserId,
    pub name: Option<String>,
    pub phone_last4: String,
    pub roles: BTreeSet<Role>,
    pub adult_confirmed: bool,
    pub transporter: Option<TransporterView>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransporterView {
    pub on_duty: bool,
}

/// `PATCH /v1/me`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PatchMe {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Must be true (18+ only) before listing or applying for a role.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adult_confirmed: Option<bool>,
}

/// `POST /v1/me/roles/TRANSPORTER:apply`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ApplyTransporter {
    pub vehicle_type: VehicleType,
    pub payload_kg: u32,
    pub registration: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub licence_number: Option<String>,
}

/// `GET /v1/deals/{id}` (the fields the transporter app uses).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DealView {
    pub deal: DealHead,
    pub listing: ListingHead,
    pub lines: Vec<DealLine>,
    pub counterpart_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DealHead {
    pub id: DealId,
    pub status: DealStatus,
    pub payment_mode: PaymentMode,
    pub final_amount: Option<Paise>,
    pub transporter_fare: Option<Paise>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ListingHead {
    pub title: String,
}

/// Dev-only sign-in response (local server).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DevToken {
    pub token: String,
    pub sub: String,
    pub phone: String,
}

/// Public client config (`GET /v1/public/config`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClientConfig {
    pub env: String,
    pub otp_enabled: bool,
    pub first_look_minutes: i64,
    pub razorpay_key_id: Option<String>,
    pub payments_simulated: bool,
    pub cities: Vec<String>,
}
