//! Structured offers (PLAN.md §4.3, §7.4).

use crate::ids::*;
use crate::state::StateMachine;
use crate::taxonomy::Unit;
use crate::{Grams, Paise};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OfferStatus {
    Active,
    Accepted,
    Rejected,
    Withdrawn,
    Expired,
    Superseded,
}

impl OfferStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Accepted => "ACCEPTED",
            Self::Rejected => "REJECTED",
            Self::Withdrawn => "WITHDRAWN",
            Self::Expired => "EXPIRED",
            Self::Superseded => "SUPERSEDED",
        }
    }
}

impl StateMachine for OfferStatus {
    fn can_transition_to(self, next: Self) -> bool {
        self == OfferStatus::Active && next != OfferStatus::Active
    }
    fn is_terminal(self) -> bool {
        self != OfferStatus::Active
    }
}

/// Rate for one line: per kg or per piece.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(tag = "unit", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Rate {
    Kg { paise_per_kg: Paise },
    Piece { paise_per_piece: Paise },
}

impl Rate {
    pub fn unit(&self) -> Unit {
        match self {
            Rate::Kg { .. } => Unit::Kg,
            Rate::Piece { .. } => Unit::Piece,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct OfferLine {
    pub item_id: ItemId,
    pub category_id: String,
    pub grade: Option<String>,
    pub rate: Rate,
    /// Verification modifier percentage applied to this line (+bonus / −discount).
    #[serde(default)]
    pub modifier_pct: i64,
    pub value_low: Paise,
    pub value_high: Paise,
    /// True when the vendor edited the rate away from their rate card (logged, PLAN.md §7.4).
    #[serde(default)]
    pub edited: bool,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PickupMethod {
    SelfPickup,
    Transporter,
}

/// Mode A (platform-mediated) or Mode B (direct), PLAN.md §13.2.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PaymentMode {
    #[serde(rename = "A_PLATFORM")]
    Platform,
    #[serde(rename = "B_DIRECT")]
    Direct,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct Slot {
    #[schema(value_type = String, format = Date)]
    pub date: NaiveDate,
    /// `HH:MM-HH:MM` (IST).
    pub window: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct LotAdjustment {
    pub code: String,
    pub amount_low: Paise,
    pub amount_high: Paise,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct SellerCounter {
    pub total: Paise,
    pub note: Option<String>,
    pub at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct Offer {
    pub id: OfferId,
    pub listing_id: ListingId,
    pub listing_version: u32,
    pub vendor_id: UserId,
    pub vendor_name: String,
    pub status: OfferStatus,
    pub lines: Vec<OfferLine>,
    #[serde(default)]
    pub adjustments: Vec<LotAdjustment>,
    pub total_low: Paise,
    pub total_high: Paise,
    pub covered_weight_low: Grams,
    pub covered_weight_high: Grams,
    pub valid_until: DateTime<Utc>,
    pub slot: Option<Slot>,
    pub pickup_method: PickupMethod,
    pub payment_mode: PaymentMode,
    pub rule_set_version: Option<u32>,
    pub engine_version: String,
    pub supersedes: Option<OfferId>,
    pub seller_counter: Option<SellerCounter>,
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_active_offers_move() {
        assert!(OfferStatus::Active.can_transition_to(OfferStatus::Accepted));
        assert!(OfferStatus::Active.can_transition_to(OfferStatus::Superseded));
        assert!(!OfferStatus::Accepted.can_transition_to(OfferStatus::Rejected));
        assert!(!OfferStatus::Active.can_transition_to(OfferStatus::Active));
    }

    #[test]
    fn payment_mode_wire_names() {
        assert_eq!(serde_json::to_string(&PaymentMode::Platform).unwrap(), "\"A_PLATFORM\"");
        let r: Rate = serde_json::from_str(r#"{"unit":"KG","paise_per_kg":65000}"#).unwrap();
        assert_eq!(r.unit(), Unit::Kg);
    }
}
