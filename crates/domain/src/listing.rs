//! Listings, versions, items, claims and proof requests (PLAN.md §4.2, §6.1, §11.4–11.5).

use crate::geo::LatLng;
use crate::ids::*;
use crate::ladder::LadderLevel;
use crate::state::StateMachine;
use crate::taxonomy::{HazardClass, Unit, WeightRange};
use crate::{Grams, Paise};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ListingStatus {
    Draft,
    Analyzing,
    NeedsInput,
    Ready,
    Published,
    Reserved,
    Completed,
    PartiallyCompleted,
    Cancelled,
    Expired,
    Withdrawn,
    Blocked,
}

impl ListingStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "DRAFT",
            Self::Analyzing => "ANALYZING",
            Self::NeedsInput => "NEEDS_INPUT",
            Self::Ready => "READY",
            Self::Published => "PUBLISHED",
            Self::Reserved => "RESERVED",
            Self::Completed => "COMPLETED",
            Self::PartiallyCompleted => "PARTIALLY_COMPLETED",
            Self::Cancelled => "CANCELLED",
            Self::Expired => "EXPIRED",
            Self::Withdrawn => "WITHDRAWN",
            Self::Blocked => "BLOCKED",
        }
    }

    /// Seller may still edit items, photos and preferences.
    pub fn is_editable(self) -> bool {
        matches!(self, Self::Draft | Self::NeedsInput | Self::Ready | Self::Published | Self::Expired | Self::Cancelled)
    }
}

impl StateMachine for ListingStatus {
    fn can_transition_to(self, next: Self) -> bool {
        use ListingStatus::*;
        if next == Blocked {
            return self != Blocked;
        }
        if next == Withdrawn {
            return !matches!(self, Completed | PartiallyCompleted | Withdrawn | Blocked | Reserved);
        }
        matches!(
            (self, next),
            (Draft, Analyzing)
                | (Draft, Ready)
                | (Analyzing, NeedsInput)
                | (Analyzing, Ready)
                | (NeedsInput, Analyzing)
                | (NeedsInput, Ready)
                | (Ready, Analyzing)
                | (Ready, Published)
                | (Published, Analyzing)
                | (Published, Reserved)
                | (Published, Expired)
                | (Reserved, Completed)
                | (Reserved, PartiallyCompleted)
                | (Reserved, Cancelled)
                | (Cancelled, Published)
                | (Cancelled, Analyzing)
                | (Expired, Published)
                | (Expired, Analyzing)
        )
    }

    fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::PartiallyCompleted | Self::Withdrawn | Self::Blocked)
    }
}

/// Approximate location vendors may see before a deal (PLAN.md §20.2).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct ApproxLocation {
    pub h3_r7: String,
    pub h3_r8: String,
    /// Centre of the r7 cell, never the exact point.
    pub center: LatLng,
    pub city: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct Listing {
    pub id: ListingId,
    pub seller_id: UserId,
    pub status: ListingStatus,
    /// Latest version number (versions are immutable once published).
    pub current_version: u32,
    pub published_version: Option<u32>,
    pub address_id: AddressId,
    /// Exact point; only served to parties allowed `ViewExactAddress`.
    pub location: LatLng,
    pub approx: ApproxLocation,
    pub drive_id: Option<DriveId>,
    pub title: String,
    pub contains_hazardous: bool,
    pub categories: Vec<String>,
    pub total_est_weight_g: Grams,
    pub min_ladder: LadderLevel,
    pub published_at: Option<DateTime<Utc>>,
    pub first_look_until: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub accepted_offer_id: Option<OfferId>,
    pub deal_id: Option<DealId>,
    pub ai_flagged_for_review: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Weekday {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

impl Weekday {
    pub fn from_chrono(d: chrono::Weekday) -> Self {
        match d {
            chrono::Weekday::Mon => Self::Mon,
            chrono::Weekday::Tue => Self::Tue,
            chrono::Weekday::Wed => Self::Wed,
            chrono::Weekday::Thu => Self::Thu,
            chrono::Weekday::Fri => Self::Fri,
            chrono::Weekday::Sat => Self::Sat,
            chrono::Weekday::Sun => Self::Sun,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct PickupPreferences {
    /// Empty means any day.
    #[serde(default)]
    pub days: Vec<Weekday>,
    /// `HH:MM-HH:MM` windows; empty means any time.
    #[serde(default)]
    pub slots: Vec<String>,
    /// Floor the items are on (0 = ground).
    #[serde(default)]
    pub floor: i32,
    #[serde(default = "yes")]
    pub lift_available: bool,
    pub item_location: Option<String>,
    pub parking_note: Option<String>,
}

fn yes() -> bool {
    true
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AnalysisStatus {
    NotStarted,
    Queued,
    Running,
    Done,
    /// AI failed closed; listing proceeds without verdicts and is flagged (PLAN.md §11.9).
    FailedClosed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct AnalysisState {
    pub status: AnalysisStatus,
    pub proof_rounds: u8,
    pub model: Option<String>,
    pub prompt_version: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct ListingVersion {
    pub listing_id: ListingId,
    pub ver: u32,
    /// Seller text; untrusted data, never instructions (PLAN.md §11.9).
    pub description: Option<String>,
    pub asking_price: Option<Paise>,
    pub claim_reason: Option<String>,
    pub pickup: PickupPreferences,
    pub analysis: AnalysisState,
    pub created_at: DateTime<Utc>,
}

/// Item attribute values used by rule-line conditions.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(untagged)]
pub enum AttrValue {
    Bool(bool),
    Int(i64),
    Text(String),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WeightSource {
    ReferenceTable,
    ModelEstimate,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct PlausibleWeight {
    pub range: WeightRange,
    pub source: WeightSource,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct Hazard {
    pub is_hazardous: bool,
    pub class: Option<HazardClass>,
    pub handling: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct Evidence {
    pub image_id: ImageId,
    /// Normalized `[x0, y0, x1, y1]`.
    pub bbox: Option<[f32; 4]>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ItemSource {
    Ai,
    Seller,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct Item {
    pub id: ItemId,
    pub category_id: String,
    pub grade: Option<String>,
    #[serde(default)]
    pub attributes: BTreeMap<String, AttrValue>,
    pub quantity: u32,
    pub unit: Unit,
    /// Seller's estimate (required before publish; the system never overwrites it).
    pub est_weight_g: Option<Grams>,
    pub plausible_weight: Option<PlausibleWeight>,
    pub hazard: Hazard,
    #[serde(default)]
    pub evidence: Vec<Evidence>,
    pub source: ItemSource,
    /// Category the AI proposed, to measure acceptance (PLAN.md §2 #1).
    pub ai_category_id: Option<String>,
    pub ai_confidence: Option<f32>,
    /// Strongest supported ladder level over this item's claims.
    pub ladder: LadderLevel,
    pub note: Option<String>,
}

impl Item {
    /// A soft note is shown when the estimate falls outside the plausible range; never blocking.
    pub fn estimate_outside_plausible(&self) -> bool {
        match (self.est_weight_g, &self.plausible_weight) {
            (Some(w), Some(p)) => {
                let per_piece = if self.quantity > 0 { Grams(w.0 / self.quantity as i64) } else { w };
                let w = if self.unit == Unit::Piece { per_piece } else { w };
                !p.range.contains(w)
            }
            _ => false,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Verdict {
    Supported,
    InsufficientEvidence,
    Inconsistent,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct Claim {
    pub id: ClaimId,
    pub item_id: Option<ItemId>,
    pub text: String,
    /// e.g. `seller_description#span-14-32`.
    pub text_ref: Option<String>,
    pub verdict: Verdict,
    pub ladder_level: LadderLevel,
    pub rationale: String,
    #[serde(default)]
    pub evidence: Vec<Evidence>,
    pub confidence: f32,
    /// Seller says the AI is wrong; vendors see "seller disputes AI assessment".
    #[serde(default)]
    pub contested: bool,
    pub contest_note: Option<String>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ShotType {
    CloseUp,
    Label,
    CutSection,
    ComponentSide,
    WorkingDemo,
    Scale,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProofStatus {
    Open,
    Provided,
    Skipped,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct ProofRequest {
    pub id: ProofRequestId,
    pub claim_id: ClaimId,
    pub round: u8,
    pub shot_type: ShotType,
    pub instruction: String,
    pub benefit: String,
    pub status: ProofStatus,
    #[serde(default)]
    pub image_ids: Vec<ImageId>,
}

/// Max proof requests per round and rounds per listing version (PLAN.md §6.1).
pub const MAX_PROOFS_PER_ROUND: usize = 3;
pub const MAX_PROOF_ROUNDS: u8 = 2;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ImageKind {
    Wide,
    CloseUp,
    Proof,
    Scale,
    Pickup,
    Document,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CaptureSource {
    Camera,
    Gallery,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PhotoIssue {
    Blur,
    Dark,
    CutOff,
    NoItems,
    PeopleVisible,
    Duplicate,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct Triage {
    pub usable: bool,
    #[serde(default)]
    pub issues: Vec<PhotoIssue>,
    pub retake_hint: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct ImageMeta {
    pub id: ImageId,
    pub kind: ImageKind,
    pub source: CaptureSource,
    /// Private original (EXIF intact, backend only).
    pub original_key: String,
    /// Processed copy: EXIF stripped, faces blurred, downscaled (vendor-visible).
    pub processed_key: Option<String>,
    pub thumb_key: Option<String>,
    pub content_type: String,
    pub size_bytes: u64,
    pub sha256: Option<String>,
    pub phash: Option<String>,
    pub triage: Option<Triage>,
    pub faces_blurred: u32,
    pub proof_request_id: Option<ProofRequestId>,
    pub uploaded: bool,
    pub created_at: DateTime<Utc>,
}

/// Listing summary derived from items.
pub fn summarize(items: &[Item]) -> (Grams, Vec<String>, bool, LadderLevel) {
    let total = items.iter().filter_map(|i| i.est_weight_g).sum();
    let mut cats: Vec<String> = items.iter().map(|i| i.category_id.clone()).collect();
    cats.sort();
    cats.dedup();
    let hazardous = items.iter().any(|i| i.hazard.is_hazardous);
    let min_ladder = items.iter().map(|i| i.ladder).min().unwrap_or_default();
    (total, cats, hazardous, min_ladder)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ListingStatus::*;

    #[test]
    fn happy_path_transitions() {
        let path = [Draft, Analyzing, NeedsInput, Ready, Published, Reserved, Completed];
        for w in path.windows(2) {
            assert!(w[0].can_transition_to(w[1]), "{:?} -> {:?}", w[0], w[1]);
        }
        assert!(Completed.is_terminal());
    }

    #[test]
    fn seller_can_skip_needs_input_and_republish() {
        assert!(NeedsInput.can_transition_to(Ready));
        assert!(Expired.can_transition_to(Published));
        assert!(Cancelled.can_transition_to(Published));
    }

    #[test]
    fn illegal_transitions_are_rejected() {
        assert!(Draft.transition(Published).is_err());
        assert!(Completed.transition(Withdrawn).is_err());
        assert!(Reserved.transition(Withdrawn).is_err(), "reserved listings are cancelled via the deal");
        assert!(Blocked.transition(Blocked).is_err());
        assert!(Published.transition(Blocked).is_ok());
    }
}
