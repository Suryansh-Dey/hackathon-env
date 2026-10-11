//! Domain events (PLAN.md §17.3). Published to EventBridge by the outbox
//! stream consumer; the envelope is versioned.

use crate::ids::*;
use crate::{Grams, Paise};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const EVENT_SOURCE: &str = "kabados";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(tag = "type", content = "data")]
pub enum DomainEvent {
    ListingAnalyzed { listing_id: ListingId, ver: u32, needs_input: bool, failed_closed: bool },
    ProofRequested { listing_id: ListingId, ver: u32, count: u32 },
    ProofProvided { listing_id: ListingId, ver: u32, proof_request_id: ProofRequestId },
    ListingPublished { listing_id: ListingId, ver: u32 },
    ListingVersionCreated { listing_id: ListingId, ver: u32 },
    MatchComputed { listing_id: ListingId, ver: u32, vendor_ids: Vec<UserId>, from_cache: bool, agent_used: bool },
    VendorNotified { listing_id: ListingId, vendor_id: UserId },
    OfferMade { offer_id: OfferId, listing_id: ListingId, vendor_id: UserId },
    OfferAccepted { offer_id: OfferId, listing_id: ListingId, deal_id: DealId },
    DealCreated { deal_id: DealId, listing_id: ListingId, seller_id: UserId, vendor_id: UserId },
    JobOffered { job_id: JobId, deal_id: DealId, transporter_ids: Vec<UserId> },
    JobAccepted { job_id: JobId, deal_id: DealId, transporter_id: UserId },
    JobTimedOut { job_id: JobId, deal_id: DealId, transporter_id: UserId },
    ArrivedAtPickup { deal_id: DealId, job_id: Option<JobId> },
    LineWeighed { deal_id: DealId, line_n: u32, weight: Grams },
    LineRegraded { deal_id: DealId, line_n: u32, from_category: String, to_category: String },
    WeightsConfirmed { deal_id: DealId, amount: Paise },
    PaymentCaptured { deal_id: DealId, provider_payment_id: String, amount: Paise },
    PayoutProcessed { deal_id: DealId, provider_payout_id: String, amount: Paise, success: bool },
    HandoverCompleted { deal_id: DealId },
    DeliveredToYard { deal_id: DealId },
    DealSettled { deal_id: DealId },
    DisputeOpened { deal_id: DealId, dispute_id: DisputeId },
    DisputeResolved { deal_id: DealId, dispute_id: DisputeId },
    RuleSetPublished { vendor_id: UserId, rule_set_id: RuleSetId, version: u32 },
    VendorAuthorizationChanged { vendor_id: UserId },
    RatingSubmitted { deal_id: DealId, from_user: UserId, to_user: UserId, stars: u8 },
}

impl DomainEvent {
    /// EventBridge `detail-type`.
    pub fn detail_type(&self) -> &'static str {
        match self {
            Self::ListingAnalyzed { .. } => "ListingAnalyzed",
            Self::ProofRequested { .. } => "ProofRequested",
            Self::ProofProvided { .. } => "ProofProvided",
            Self::ListingPublished { .. } => "ListingPublished",
            Self::ListingVersionCreated { .. } => "ListingVersionCreated",
            Self::MatchComputed { .. } => "MatchComputed",
            Self::VendorNotified { .. } => "VendorNotified",
            Self::OfferMade { .. } => "OfferMade",
            Self::OfferAccepted { .. } => "OfferAccepted",
            Self::DealCreated { .. } => "DealCreated",
            Self::JobOffered { .. } => "JobOffered",
            Self::JobAccepted { .. } => "JobAccepted",
            Self::JobTimedOut { .. } => "JobTimedOut",
            Self::ArrivedAtPickup { .. } => "ArrivedAtPickup",
            Self::LineWeighed { .. } => "LineWeighed",
            Self::LineRegraded { .. } => "LineRegraded",
            Self::WeightsConfirmed { .. } => "WeightsConfirmed",
            Self::PaymentCaptured { .. } => "PaymentCaptured",
            Self::PayoutProcessed { .. } => "PayoutProcessed",
            Self::HandoverCompleted { .. } => "HandoverCompleted",
            Self::DeliveredToYard { .. } => "DeliveredToYard",
            Self::DealSettled { .. } => "DealSettled",
            Self::DisputeOpened { .. } => "DisputeOpened",
            Self::DisputeResolved { .. } => "DisputeResolved",
            Self::RuleSetPublished { .. } => "RuleSetPublished",
            Self::VendorAuthorizationChanged { .. } => "VendorAuthorizationChanged",
            Self::RatingSubmitted { .. } => "RatingSubmitted",
        }
    }
}

/// Versioned envelope stored in the outbox and sent as the EventBridge `detail`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct EventEnvelope {
    pub id: EventId,
    pub schema_v: u32,
    pub occurred_at: DateTime<Utc>,
    /// Correlation ID propagated from the originating request.
    pub correlation_id: Option<String>,
    pub event: DomainEvent,
}

impl EventEnvelope {
    pub fn new(event: DomainEvent) -> Self {
        Self { id: EventId::new(), schema_v: 1, occurred_at: Utc::now(), correlation_id: None, event }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_round_trip_and_detail_type() {
        let e = EventEnvelope::new(DomainEvent::ListingPublished { listing_id: ListingId::new(), ver: 2 });
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["event"]["type"], "ListingPublished");
        let back: EventEnvelope = serde_json::from_value(v).unwrap();
        assert_eq!(back, e);
        assert_eq!(back.event.detail_type(), "ListingPublished");
    }
}
