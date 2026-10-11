//! Users, roles, onboarding and KYC (PLAN.md §3, §5, §7.1, §8.3).

use crate::geo::LatLng;
use crate::ids::*;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Roles are attributes of a single account (PLAN.md §3).
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Role {
    Seller,
    Vendor,
    Transporter,
    SocietyAdmin,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Seller => "SELLER",
            Role::Vendor => "VENDOR",
            Role::Transporter => "TRANSPORTER",
            Role::SocietyAdmin => "SOCIETY_ADMIN",
        }
    }

    pub fn parse(s: &str) -> Option<Role> {
        match s.to_ascii_uppercase().as_str() {
            "SELLER" => Some(Role::Seller),
            "VENDOR" => Some(Role::Vendor),
            "TRANSPORTER" => Some(Role::Transporter),
            "SOCIETY_ADMIN" => Some(Role::SocietyAdmin),
            _ => None,
        }
    }
}

/// Ciphertext of a field-level encrypted attribute (PLAN.md §16.2). Opaque to the domain.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(transparent)]
pub struct Sealed(pub String);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct UserProfile {
    pub id: UserId,
    pub cognito_sub: String,
    /// Keyed hash of the E.164 phone number (lookups only).
    pub phone_hash: String,
    /// Last four digits for display to the account owner only.
    pub phone_last4: String,
    pub name: Option<String>,
    pub roles: BTreeSet<Role>,
    pub adult_confirmed: bool,
    #[serde(default)]
    pub suspended: bool,
    #[serde(default)]
    pub honesty: ScoreStats,
    pub created_at: DateTime<Utc>,
}

/// Running statistics for a Bayesian-smoothed score (PLAN.md §14).
#[derive(Copy, Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct ScoreStats {
    pub count: u32,
    /// Sum of observations, each in `0..=1000` (per-mille).
    pub sum_permille: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct Address {
    pub id: AddressId,
    pub label: String,
    pub location: LatLng,
    pub h3_r7: String,
    pub h3_r8: String,
    /// City used for the Rate Index (e.g. `delhi`).
    pub city: String,
    pub text: Sealed,
    pub landmark: Option<Sealed>,
    pub created_at: DateTime<Utc>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PayoutKind {
    Upi,
    Bank,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PayoutValidation {
    Pending,
    Validated,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct PayoutMethod {
    pub id: PayoutMethodId,
    pub kind: PayoutKind,
    /// Masked for display, e.g. `ra****@okhdfc`.
    pub display: String,
    pub value: Sealed,
    pub validation: PayoutValidation,
    pub validated_name: Option<String>,
    pub provider_ref: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConsentPurpose {
    ServiceDelivery,
    AiImprovement,
    Marketing,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct Consent {
    pub purpose: ConsentPurpose,
    pub notice_version: String,
    pub granted: bool,
    pub at: DateTime<Utc>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum KycStatus {
    NotStarted,
    Submitted,
    PendingManualReview,
    Approved,
    Rejected,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DocumentKind {
    Pan,
    Gstin,
    SpcbAuthorization,
    RecyclerTieUp,
    DrivingLicence,
    VehicleRc,
    Selfie,
    YardPhoto,
    ScaleCalibrationPhoto,
    Insurance,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct KycDocument {
    pub id: DocumentId,
    pub kind: DocumentKind,
    pub s3_key: String,
    /// Values extracted by Textract (field → text), numbers redacted.
    #[serde(default)]
    pub extracted: std::collections::BTreeMap<String, String>,
    pub number: Option<Sealed>,
    pub check: Option<KycCheck>,
    pub expires_at: Option<DateTime<Utc>>,
    pub uploaded_at: DateTime<Utc>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum KycCheckOutcome {
    Verified,
    Mismatch,
    PendingManualReview,
    InvalidFormat,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct KycCheck {
    pub outcome: KycCheckOutcome,
    pub provider: String,
    pub detail: Option<String>,
    pub at: DateTime<Utc>,
}

/// Hazardous-category authorization (SPCB or recycler tie-up, PLAN.md §7.1).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct HazardousAuthorization {
    pub authorization_no: String,
    pub state: String,
    pub via_tie_up: bool,
    pub valid_until: DateTime<Utc>,
    pub verified_by: Option<String>,
    pub verified_at: Option<DateTime<Utc>>,
}

impl HazardousAuthorization {
    /// Authorization is valid only when ops-verified and not expired.
    pub fn is_valid_at(&self, now: DateTime<Utc>) -> bool {
        self.verified_at.is_some() && now < self.valid_until
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct VendorProfile {
    pub user_id: UserId,
    pub business_name: String,
    pub yard_location: LatLng,
    pub yard_h3_r8: String,
    pub city: String,
    pub kyc_status: KycStatus,
    pub kyc_submitted_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub categories: BTreeSet<String>,
    pub hazardous_auth: Option<HazardousAuthorization>,
    #[serde(default)]
    pub suspended: bool,
    #[serde(default)]
    pub rating: ScoreStats,
    #[serde(default)]
    pub reliability: ScoreStats,
    /// Median minutes to first offer on matched listings.
    pub response_minutes_p50: Option<u32>,
    pub active_rule_set: Option<RuleSetRef>,
    /// Payment details on file (Razorpay customer or payout account reference).
    pub payment_ref: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl VendorProfile {
    pub fn kyc_approved(&self) -> bool {
        self.kyc_status == KycStatus::Approved
    }

    pub fn hazardous_auth_valid(&self, now: DateTime<Utc>) -> bool {
        self.hazardous_auth.as_ref().is_some_and(|a| a.is_valid_at(now))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct RuleSetRef {
    pub rule_set_id: RuleSetId,
    pub version: u32,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum VehicleType {
    Cycle,
    ERickshaw,
    Tempo,
    MiniTruck,
    Truck,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct Vehicle {
    pub id: VehicleId,
    pub vehicle_type: VehicleType,
    pub payload_kg: u32,
    pub registration_masked: String,
    pub registration: Option<Sealed>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct TransporterProfile {
    pub user_id: UserId,
    pub kyc_status: KycStatus,
    pub kyc_submitted_at: Option<DateTime<Utc>>,
    pub on_duty: bool,
    pub vehicles: Vec<Vehicle>,
    pub base_location: Option<LatLng>,
    #[serde(default)]
    pub rating: ScoreStats,
    #[serde(default)]
    pub acceptance: ScoreStats,
    #[serde(default)]
    pub on_time: ScoreStats,
    #[serde(default)]
    pub suspended: bool,
    pub created_at: DateTime<Utc>,
}

impl TransporterProfile {
    pub fn kyc_approved(&self) -> bool {
        self.kyc_status == KycStatus::Approved
    }

    pub fn max_payload_kg(&self) -> u32 {
        self.vehicles.iter().map(|v| v.payload_kg).max().unwrap_or(0)
    }
}

/// Fare agreed per job: fixed, or per-km template (PLAN.md §7.5).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FareTemplate {
    Fixed { amount_paise: i64 },
    PerKm { base_paise: i64, per_km_paise: i64 },
}

/// Vendor ↔ transporter trusted relationship.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct MyTransporter {
    pub vendor_id: UserId,
    pub transporter_id: UserId,
    pub display_name: String,
    pub fare: Option<FareTemplate>,
    pub status: TrustStatus,
    pub invited_at: DateTime<Utc>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TrustStatus {
    Invited,
    Active,
    Removed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct Device {
    pub device_id: String,
    pub platform: String,
    pub push_endpoint_arn: Option<String>,
    pub integrity_verdict: Option<String>,
    pub updated_at: DateTime<Utc>,
}

/// Masks a UPI ID for display: `ravi.k@okhdfc` → `ra****@okhdfc`.
pub fn mask_vpa(vpa: &str) -> String {
    match vpa.split_once('@') {
        Some((user, handle)) => {
            let keep: String = user.chars().take(2).collect();
            format!("{keep}****@{handle}")
        }
        None => "****".into(),
    }
}

/// Validates the format of a UPI virtual payment address.
pub fn is_valid_vpa(vpa: &str) -> bool {
    let Some((user, handle)) = vpa.split_once('@') else { return false };
    (2..=256).contains(&user.len())
        && user.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
        && (2..=64).contains(&handle.len())
        && handle.chars().all(|c| c.is_ascii_alphanumeric())
}

/// Normalizes an Indian mobile number to E.164 (`+91XXXXXXXXXX`).
pub fn normalize_in_phone(raw: &str) -> Option<String> {
    let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    let ten = match digits.len() {
        10 => digits,
        11 if digits.starts_with('0') => digits[1..].to_string(),
        12 if digits.starts_with("91") => digits[2..].to_string(),
        _ => return None,
    };
    if matches!(ten.as_bytes()[0], b'6'..=b'9') { Some(format!("+91{ten}")) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phone_normalization() {
        assert_eq!(normalize_in_phone("98765 43210").as_deref(), Some("+919876543210"));
        assert_eq!(normalize_in_phone("+91-98765-43210").as_deref(), Some("+919876543210"));
        assert_eq!(normalize_in_phone("09876543210").as_deref(), Some("+919876543210"));
        assert_eq!(normalize_in_phone("12345"), None);
        assert_eq!(normalize_in_phone("5876543210"), None);
    }

    #[test]
    fn vpa_rules() {
        assert!(is_valid_vpa("ravi.k@okhdfc"));
        assert!(!is_valid_vpa("ravi@"));
        assert!(!is_valid_vpa("no-at-sign"));
        assert_eq!(mask_vpa("ravi.k@okhdfc"), "ra****@okhdfc");
    }

    #[test]
    fn hazardous_auth_requires_verification_and_expiry() {
        let now = Utc::now();
        let mut a = HazardousAuthorization {
            authorization_no: "DL/HW/123".into(),
            state: "DL".into(),
            via_tie_up: false,
            valid_until: now + chrono::Duration::days(30),
            verified_by: None,
            verified_at: None,
        };
        assert!(!a.is_valid_at(now));
        a.verified_at = Some(now);
        assert!(a.is_valid_at(now));
        assert!(!a.is_valid_at(now + chrono::Duration::days(31)));
    }
}
