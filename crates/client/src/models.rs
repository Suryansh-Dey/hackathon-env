//! Request/response shapes that are defined in the API crate; mirrored here so
//! the client doesn't depend on the server crate. Kept in sync via the OpenAPI
//! contract check in CI.

use kabados_domain::geo::LatLng;
use kabados_domain::job::Job;
use serde::{Deserialize, Serialize};

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
