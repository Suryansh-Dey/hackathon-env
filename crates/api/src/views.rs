//! Response bodies. Views never leak what the viewer may not see: vendors get
//! the approximate (H3 r7) location and processed images only; the exact
//! address is added only when `ViewExactAddress` is allowed.

use chrono::{DateTime, Utc};
use kabados_domain::deal::*;
use kabados_domain::geo::LatLng;
use kabados_domain::ids::*;
use kabados_domain::job::*;
use kabados_domain::ladder::LadderLevel;
use kabados_domain::listing::*;
use kabados_domain::user::*;
use kabados_domain::{Grams, Paise};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use utoipa::ToSchema;

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct MeView {
    pub id: UserId,
    pub name: Option<String>,
    pub phone_last4: String,
    pub roles: BTreeSet<Role>,
    pub adult_confirmed: bool,
    pub honesty_score: f64,
    pub vendor: Option<VendorView>,
    pub transporter: Option<TransporterView>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct VendorView {
    pub business_name: String,
    pub yard_location: LatLng,
    pub city: String,
    pub kyc_status: KycStatus,
    pub categories: BTreeSet<String>,
    pub hazardous_authorized: bool,
    pub hazardous_valid_until: Option<DateTime<Utc>>,
    pub active_rule_set_version: Option<u32>,
    pub rating: f64,
    pub reliability: f64,
}

impl From<&VendorProfile> for VendorView {
    fn from(v: &VendorProfile) -> Self {
        Self {
            business_name: v.business_name.clone(),
            yard_location: v.yard_location,
            city: v.city.clone(),
            kyc_status: v.kyc_status,
            categories: v.categories.clone(),
            hazardous_authorized: v.hazardous_auth_valid(Utc::now()),
            hazardous_valid_until: v.hazardous_auth.as_ref().map(|a| a.valid_until),
            active_rule_set_version: v.active_rule_set.as_ref().map(|r| r.version),
            rating: v.rating.stars(),
            reliability: v.reliability.smoothed(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct TransporterView {
    pub kyc_status: KycStatus,
    pub on_duty: bool,
    pub vehicles: Vec<Vehicle>,
    pub rating: f64,
    pub acceptance: f64,
}

impl From<&TransporterProfile> for TransporterView {
    fn from(t: &TransporterProfile) -> Self {
        Self {
            kyc_status: t.kyc_status,
            on_duty: t.on_duty,
            vehicles: t.vehicles.clone(),
            rating: t.rating.stars(),
            acceptance: t.acceptance.smoothed(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct AddressView {
    pub id: AddressId,
    pub label: String,
    pub location: LatLng,
    pub city: String,
    pub text: String,
    pub landmark: Option<String>,
}

/// Listing fields safe for any viewer allowed `ViewListing`.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ListingSummary {
    pub id: ListingId,
    pub status: ListingStatus,
    pub title: String,
    pub approx: ApproxLocation,
    pub categories: Vec<String>,
    pub total_est_weight_g: Grams,
    pub contains_hazardous: bool,
    pub min_ladder: LadderLevel,
    pub current_version: u32,
    pub drive_id: Option<DriveId>,
    pub published_at: Option<DateTime<Utc>>,
    pub first_look_until: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl From<&Listing> for ListingSummary {
    fn from(l: &Listing) -> Self {
        Self {
            id: l.id.clone(),
            status: l.status,
            title: l.title.clone(),
            approx: l.approx.clone(),
            categories: l.categories.clone(),
            total_est_weight_g: l.total_est_weight_g,
            contains_hazardous: l.contains_hazardous,
            min_ladder: l.min_ladder,
            current_version: l.current_version,
            drive_id: l.drive_id.clone(),
            published_at: l.published_at,
            first_look_until: l.first_look_until,
            created_at: l.created_at,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Viewer {
    Seller,
    Vendor,
    Transporter,
    Ops,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ImageView {
    pub id: ImageId,
    pub kind: ImageKind,
    pub source: CaptureSource,
    pub url: Option<String>,
    pub thumb_url: Option<String>,
    pub triage: Option<Triage>,
    pub proof_request_id: Option<ProofRequestId>,
    pub processed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ExactAddress {
    pub location: LatLng,
    pub text: String,
    pub landmark: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ListingView {
    pub viewer: Viewer,
    pub listing: ListingSummary,
    pub version: ListingVersionView,
    pub items: Vec<Item>,
    pub claims: Vec<Claim>,
    pub proof_requests: Vec<ProofRequest>,
    pub images: Vec<ImageView>,
    pub exact_address: Option<ExactAddress>,
    pub deal_id: Option<DealId>,
    pub seller_honesty: f64,
    /// Seller-only: estimates outside the plausible range (soft notes).
    pub weight_notes: Vec<ItemId>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ListingVersionView {
    pub ver: u32,
    pub description: Option<String>,
    pub asking_price: Option<Paise>,
    pub claim_reason: Option<String>,
    pub pickup: PickupPreferences,
    pub analysis: AnalysisState,
}

impl From<&ListingVersion> for ListingVersionView {
    fn from(v: &ListingVersion) -> Self {
        Self {
            ver: v.ver,
            description: v.description.clone(),
            asking_price: v.asking_price,
            claim_reason: v.claim_reason.clone(),
            pickup: v.pickup.clone(),
            analysis: v.analysis.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct DealView {
    pub deal: Deal,
    pub viewer: Viewer,
    pub listing: ListingSummary,
    pub lines: Vec<DealLine>,
    pub weighings: Vec<Weighing>,
    pub job: Option<Job>,
    pub disputes: Vec<Dispute>,
    pub exact_address: Option<ExactAddress>,
    /// Seller-only, after payment (Mode A) or receipt confirmation (Mode B).
    pub handover_otp: Option<String>,
    pub counterpart_name: Option<String>,
    pub my_rating: Option<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct Created<T> {
    pub id: T,
}
