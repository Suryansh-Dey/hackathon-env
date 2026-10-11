//! Deals, lines and weighings (PLAN.md §4.4, §13).

use crate::ids::*;
use crate::offer::{PaymentMode, PickupMethod, Rate, Slot};
use crate::state::StateMachine;
use crate::{Grams, Paise};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DealStatus {
    Created,
    PickupScheduled,
    JobAssigned,
    AtPickup,
    Weighed,
    LineRegraded,
    SellerConfirmed,
    AwaitingPayment,
    Paid,
    PaidDirect,
    HandedOver,
    InTransit,
    Delivered,
    Settled,
    Closed,
    SellerNoShow,
    PickupNoShow,
    Disputed,
    Cancelled,
}

impl DealStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Created => "CREATED",
            Self::PickupScheduled => "PICKUP_SCHEDULED",
            Self::JobAssigned => "JOB_ASSIGNED",
            Self::AtPickup => "AT_PICKUP",
            Self::Weighed => "WEIGHED",
            Self::LineRegraded => "LINE_REGRADED",
            Self::SellerConfirmed => "SELLER_CONFIRMED",
            Self::AwaitingPayment => "AWAITING_PAYMENT",
            Self::Paid => "PAID",
            Self::PaidDirect => "PAID_DIRECT",
            Self::HandedOver => "HANDED_OVER",
            Self::InTransit => "IN_TRANSIT",
            Self::Delivered => "DELIVERED",
            Self::Settled => "SETTLED",
            Self::Closed => "CLOSED",
            Self::SellerNoShow => "SELLER_NO_SHOW",
            Self::PickupNoShow => "PICKUP_NO_SHOW",
            Self::Disputed => "DISPUTED",
            Self::Cancelled => "CANCELLED",
        }
    }

    /// States in which the vendor may see the exact address (PLAN.md §18.2).
    pub const EXACT_ADDRESS_STATES: [DealStatus; 8] = [
        Self::Created,
        Self::PickupScheduled,
        Self::JobAssigned,
        Self::AtPickup,
        Self::Weighed,
        Self::SellerConfirmed,
        Self::AwaitingPayment,
        Self::Paid,
    ];

    /// Deal is still open (calls allowed, PLAN.md §18.2).
    pub fn is_open(self) -> bool {
        !matches!(self, Self::Settled | Self::Closed | Self::Cancelled | Self::SellerNoShow | Self::PickupNoShow)
    }

    /// States before goods are weighed (no-show and cancel are possible).
    pub fn is_pre_pickup(self) -> bool {
        matches!(self, Self::Created | Self::PickupScheduled | Self::JobAssigned | Self::AtPickup)
    }
}

impl StateMachine for DealStatus {
    fn can_transition_to(self, next: Self) -> bool {
        use DealStatus::*;
        if matches!(next, SellerNoShow | PickupNoShow | Cancelled) && self.is_pre_pickup() {
            return true;
        }
        if next == Disputed {
            return matches!(self, AtPickup | Weighed | LineRegraded | SellerConfirmed | AwaitingPayment | Delivered);
        }
        matches!(
            (self, next),
            (Created, PickupScheduled)
                | (PickupScheduled, JobAssigned)
                | (PickupScheduled, AtPickup)
                | (JobAssigned, AtPickup)
                | (JobAssigned, PickupScheduled)
                | (AtPickup, Weighed)
                | (Weighed, Weighed)
                | (Weighed, LineRegraded)
                | (LineRegraded, Weighed)
                | (Weighed, SellerConfirmed)
                | (SellerConfirmed, AwaitingPayment)
                | (SellerConfirmed, PaidDirect)
                | (AwaitingPayment, Paid)
                | (Paid, HandedOver)
                | (PaidDirect, HandedOver)
                | (HandedOver, InTransit)
                | (HandedOver, Delivered)
                | (InTransit, Delivered)
                | (Delivered, Settled)
                | (Settled, Closed)
                | (Disputed, Weighed)
                | (Disputed, SellerConfirmed)
                | (Disputed, Cancelled)
                | (Disputed, Settled)
                | (Disputed, Closed)
                | (Weighed, Cancelled)
                | (LineRegraded, Cancelled)
        )
    }

    fn is_terminal(self) -> bool {
        matches!(self, Self::Closed | Self::Cancelled | Self::SellerNoShow | Self::PickupNoShow)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LineStatus {
    Pending,
    Weighed,
    /// Actual category differs; re-priced, awaiting seller accept/remove (PLAN.md §13.5).
    RegradePending,
    Removed,
    Final,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct DealLine {
    pub n: u32,
    pub item_id: ItemId,
    pub category_id: String,
    pub grade: Option<String>,
    pub quantity: u32,
    pub rate: Rate,
    pub modifier_pct: i64,
    pub status: LineStatus,
    pub weighed_g: Option<Grams>,
    pub amount: Option<Paise>,
    /// Set when re-graded at pickup.
    pub regraded_from: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct Weighing {
    pub line_n: u32,
    pub image_id: Option<ImageId>,
    /// AI reading (PLAN.md §11.8) and the confirmed value.
    pub ai_reading_g: Option<Grams>,
    pub ai_confidence: Option<f32>,
    pub reading_g: Grams,
    pub by_user: UserId,
    pub at: DateTime<Utc>,
    pub lat: Option<f64>,
    pub lng: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct Deal {
    pub id: DealId,
    pub listing_id: ListingId,
    pub listing_version: u32,
    pub offer_id: OfferId,
    pub seller_id: UserId,
    pub vendor_id: UserId,
    pub transporter_id: Option<UserId>,
    pub job_id: Option<JobId>,
    pub status: DealStatus,
    pub payment_mode: PaymentMode,
    pub pickup_method: PickupMethod,
    pub slot: Option<Slot>,
    pub rule_set_version: Option<u32>,
    pub engine_version: String,
    pub agreed_total_low: Paise,
    pub agreed_total_high: Paise,
    /// Final seller amount from scale readings (`kabados-pricing`).
    pub final_amount: Option<Paise>,
    pub transporter_fare: Option<Paise>,
    pub platform_fee: Paise,
    pub payment_order_id: Option<String>,
    pub direct_payment_marked_at: Option<DateTime<Utc>>,
    /// Keyed hash of the handover OTP; the plain code is shown only to the seller.
    pub handover_otp_hash: Option<String>,
    pub handover_at: Option<DateTime<Utc>>,
    pub delivered_at: Option<DateTime<Utc>>,
    pub receipt_id: Option<ReceiptId>,
    pub workflow_execution_arn: Option<String>,
    pub cancel_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DisputeKind {
    Weight,
    ItemMismatch,
    Payment,
    NoShow,
    Conduct,
    Other,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DisputeStatus {
    Open,
    InReview,
    Resolved,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct Dispute {
    pub id: DisputeId,
    pub deal_id: DealId,
    pub opened_by: UserId,
    pub kind: DisputeKind,
    pub description: String,
    pub status: DisputeStatus,
    pub resolution: Option<String>,
    /// Deal status to return to on resolution.
    pub resume_status: DealStatus,
    pub sla_due_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct Rating {
    pub deal_id: DealId,
    pub from_user: UserId,
    pub to_user: UserId,
    /// 1..=5
    pub stars: u8,
    pub comment: Option<String>,
    pub at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct CallSession {
    pub id: CallId,
    pub deal_id: DealId,
    pub meeting_id: String,
    pub started_by: UserId,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
}

/// Green Receipt: platform record of a completed pickup (not an EPR certificate, PLAN.md §20.3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct GreenReceipt {
    pub id: ReceiptId,
    pub deal_id: DealId,
    pub issued_at: DateTime<Utc>,
    pub city: String,
    pub vendor_name: String,
    pub vendor_authorized_for_hazardous: bool,
    pub lines: Vec<ReceiptLine>,
    pub total_weight: Grams,
    pub total_amount: Paise,
    pub co2e_avoided_g: i64,
    pub emission_factor_source: String,
    pub disclaimer: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct ReceiptLine {
    pub category_id: String,
    pub category_name: String,
    pub weight: Grams,
    pub hazardous: bool,
}

pub const RECEIPT_DISCLAIMER: &str = "This Green Receipt is a KabadOS platform record of a completed pickup. It is not an EPR certificate.";

#[cfg(test)]
mod tests {
    use super::*;
    use DealStatus::*;

    #[test]
    fn mode_a_happy_path() {
        let p = [
            Created,
            PickupScheduled,
            JobAssigned,
            AtPickup,
            Weighed,
            SellerConfirmed,
            AwaitingPayment,
            Paid,
            HandedOver,
            InTransit,
            Delivered,
            Settled,
            Closed,
        ];
        for w in p.windows(2) {
            assert!(w[0].can_transition_to(w[1]), "{:?} -> {:?}", w[0], w[1]);
        }
    }

    #[test]
    fn mode_b_happy_path() {
        let p = [Created, PickupScheduled, AtPickup, Weighed, SellerConfirmed, PaidDirect, HandedOver, Delivered, Settled];
        for w in p.windows(2) {
            assert!(w[0].can_transition_to(w[1]), "{:?} -> {:?}", w[0], w[1]);
        }
    }

    #[test]
    fn goods_never_handed_over_unpaid() {
        for s in [Created, PickupScheduled, JobAssigned, AtPickup, Weighed, LineRegraded, SellerConfirmed, AwaitingPayment, Disputed] {
            assert!(!s.can_transition_to(HandedOver), "{s:?} -> HandedOver must be impossible");
        }
    }

    #[test]
    fn branches() {
        assert!(Weighed.can_transition_to(LineRegraded));
        assert!(LineRegraded.can_transition_to(Weighed));
        assert!(AtPickup.can_transition_to(SellerNoShow));
        assert!(!Paid.can_transition_to(Cancelled));
        assert!(Weighed.can_transition_to(Disputed));
        assert!(Closed.is_terminal() && !Closed.is_open());
    }
}
