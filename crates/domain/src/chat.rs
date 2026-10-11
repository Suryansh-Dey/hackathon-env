//! Chat threads (PLAN.md §7.4, §17.2).

use crate::ids::*;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct Thread {
    pub id: ThreadId,
    pub listing_id: ListingId,
    pub seller_id: UserId,
    pub vendor_id: UserId,
    pub deal_id: Option<DealId>,
    pub created_at: DateTime<Utc>,
    pub last_message_at: Option<DateTime<Utc>>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Moderation {
    Clean,
    /// Phone number or UPI ID typed in pre-deal chat: soft warning shown.
    SoftWarning,
    /// Abuse or prompt attack: held for ops review.
    Quarantined,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct Message {
    pub id: MessageId,
    pub thread_id: ThreadId,
    pub sender_id: UserId,
    pub text: String,
    pub moderation: Moderation,
    pub warning: Option<String>,
    pub at: DateTime<Utc>,
}
