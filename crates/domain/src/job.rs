//! Transport jobs (PLAN.md §4.5, §8.4).

use crate::ids::*;
use crate::state::StateMachine;
use crate::{Grams, Paise};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum JobStatus {
    Offered,
    Accepted,
    Declined,
    TimedOut,
    EnRoute,
    ArrivedPickup,
    Loaded,
    InTransit,
    ArrivedYard,
    Delivered,
    PaidOut,
    Cancelled,
}

impl JobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Offered => "OFFERED",
            Self::Accepted => "ACCEPTED",
            Self::Declined => "DECLINED",
            Self::TimedOut => "TIMED_OUT",
            Self::EnRoute => "EN_ROUTE",
            Self::ArrivedPickup => "ARRIVED_PICKUP",
            Self::Loaded => "LOADED",
            Self::InTransit => "IN_TRANSIT",
            Self::ArrivedYard => "ARRIVED_YARD",
            Self::Delivered => "DELIVERED",
            Self::PaidOut => "PAID_OUT",
            Self::Cancelled => "CANCELLED",
        }
    }

    /// Transporter may see the exact address (PLAN.md §18.2).
    pub const EXACT_ADDRESS_STATES: [JobStatus; 3] = [Self::Accepted, Self::EnRoute, Self::ArrivedPickup];

    /// Live location is shared from acceptance to delivery (PLAN.md §8.4).
    pub fn shares_location(self) -> bool {
        matches!(self, Self::Accepted | Self::EnRoute | Self::ArrivedPickup | Self::Loaded | Self::InTransit | Self::ArrivedYard)
    }
}

impl StateMachine for JobStatus {
    fn can_transition_to(self, next: Self) -> bool {
        use JobStatus::*;
        if next == Cancelled {
            return matches!(self, Offered | Accepted | EnRoute | ArrivedPickup);
        }
        matches!(
            (self, next),
            (Offered, Accepted)
                | (Offered, Declined)
                | (Offered, TimedOut)
                | (Accepted, EnRoute)
                | (Accepted, ArrivedPickup)
                | (EnRoute, ArrivedPickup)
                | (ArrivedPickup, Loaded)
                | (Loaded, InTransit)
                | (InTransit, ArrivedYard)
                | (ArrivedYard, Delivered)
                | (Delivered, PaidOut)
        )
    }

    fn is_terminal(self) -> bool {
        matches!(self, Self::Declined | Self::TimedOut | Self::PaidOut | Self::Cancelled)
    }
}

/// Typed checklist events posted by the transporter app (`POST /v1/jobs/{id}/events`).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum JobEventKind {
    EnRoute,
    Arrived,
    Photos,
    Loaded,
    InTransit,
    ArrivedYard,
    Delivered,
    PaidOutConfirmed,
    FareChanged,
    Sos,
}

impl JobEventKind {
    /// Job status this event moves to, if any.
    pub fn target_status(self) -> Option<JobStatus> {
        match self {
            Self::EnRoute => Some(JobStatus::EnRoute),
            Self::Arrived => Some(JobStatus::ArrivedPickup),
            Self::Loaded => Some(JobStatus::Loaded),
            Self::InTransit => Some(JobStatus::InTransit),
            Self::ArrivedYard => Some(JobStatus::ArrivedYard),
            Self::Delivered => Some(JobStatus::Delivered),
            Self::PaidOutConfirmed => Some(JobStatus::PaidOut),
            Self::Photos | Self::FareChanged | Self::Sos => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct JobEvent {
    pub kind: JobEventKind,
    pub at: DateTime<Utc>,
    /// Device time, kept to compare against server time (PLAN.md §14).
    pub device_at: Option<DateTime<Utc>>,
    pub lat: Option<f64>,
    pub lng: Option<f64>,
    #[serde(default)]
    pub image_ids: Vec<ImageId>,
    pub note: Option<String>,
    pub mock_location: Option<bool>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Dispatch {
    /// Offer to one transporter at a time with a timeout.
    Sequential,
    /// Broadcast to the trusted list; first accept wins.
    Broadcast,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct Job {
    pub id: JobId,
    pub deal_id: DealId,
    pub vendor_id: UserId,
    pub status: JobStatus,
    pub dispatch: Dispatch,
    /// Ranked candidates; `offered_to` advances on timeout.
    pub candidates: Vec<UserId>,
    pub offered_to: Vec<UserId>,
    pub transporter_id: Option<UserId>,
    pub fare: Paise,
    #[serde(default)]
    pub fare_history: Vec<FareChange>,
    pub load_weight: Grams,
    pub bulky_items: u32,
    pub floors: i32,
    pub pickup_cell_r8: String,
    pub offer_expires_at: DateTime<Utc>,
    pub accept_timeout_secs: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct FareChange {
    pub from: Paise,
    pub to: Paise,
    pub by: UserId,
    pub at: DateTime<Utc>,
    pub note: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use JobStatus::*;

    #[test]
    fn checklist_path() {
        let p = [Offered, Accepted, EnRoute, ArrivedPickup, Loaded, InTransit, ArrivedYard, Delivered, PaidOut];
        for w in p.windows(2) {
            assert!(w[0].can_transition_to(w[1]));
        }
        assert!(!Loaded.can_transition_to(Cancelled), "loaded goods are not cancelled by the transporter");
        assert!(Offered.can_transition_to(TimedOut));
        assert_eq!(JobEventKind::Arrived.target_status(), Some(ArrivedPickup));
    }
}
