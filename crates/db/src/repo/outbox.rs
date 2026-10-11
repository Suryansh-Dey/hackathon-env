//! Transactional outbox (PLAN.md §16.4, §19.2): events and notifications are
//! written in the same transaction as the state change and fanned out by the
//! Streams consumer.

use super::ts_key;
use crate::keys;
use crate::record::{Meta, Record};
use crate::store::{DbResult, Table, TxOp};
use chrono::{DateTime, Duration, Utc};
use kabados_domain::events::{DomainEvent, EventEnvelope};
use kabados_domain::ids::{NotificationId, UserId};
use serde::{Deserialize, Serialize};

pub const OUTBOX_TYPE: &str = "outbox";
pub const NOTIFY_TYPE: &str = "notify";

/// Outbox items expire after the Streams consumer has had ample time.
pub const OUTBOX_TTL_DAYS: i64 = 7;

pub fn outbox_op(pk: &str, event: DomainEvent, correlation_id: Option<String>) -> DbResult<TxOp> {
    let mut env = EventEnvelope::new(event);
    env.correlation_id = correlation_id;
    let meta =
        Meta::new(pk, keys::outbox(&ts_key(env.occurred_at), &env.id), OUTBOX_TYPE).ttl(Utc::now() + Duration::days(OUTBOX_TTL_DAYS));
    Record::new(meta, env).insert_op(Table::Core)
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum NotificationKind {
    AnalysisDone,
    ProofRequested,
    NewMatch,
    NewOffer,
    CounterOffer,
    OfferAccepted,
    JobOffer,
    JobAccepted,
    PickupEta,
    Arrived,
    WeightsToConfirm,
    PaymentRequested,
    PaymentReceived,
    HandoverDone,
    Receipt,
    Dispute,
    ChatMessage,
    KycDecision,
    DriveReminder,
}

impl NotificationKind {
    /// Security and active-job messages ignore quiet hours (PLAN.md §19.2).
    pub fn urgent(self) -> bool {
        matches!(self, Self::JobOffer | Self::Arrived | Self::WeightsToConfirm | Self::PaymentReceived | Self::HandoverDone)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct Notification {
    pub id: NotificationId,
    pub user_id: UserId,
    pub kind: NotificationKind,
    pub title: String,
    pub body: String,
    /// Deep link path in the relevant client, e.g. `/listing?id=…`.
    pub link: Option<String>,
    #[serde(default)]
    pub data: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

impl Notification {
    pub fn new(user_id: UserId, kind: NotificationKind, title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            id: NotificationId::new(),
            user_id,
            kind,
            title: title.into(),
            body: body.into(),
            link: None,
            data: serde_json::Value::Null,
            created_at: Utc::now(),
        }
    }

    pub fn link(mut self, l: impl Into<String>) -> Self {
        self.link = Some(l.into());
        self
    }

    pub fn data(mut self, d: serde_json::Value) -> Self {
        self.data = d;
        self
    }
}

pub fn notify_op(pk: &str, n: Notification) -> DbResult<TxOp> {
    let meta = Meta::new(pk, keys::notify(&ts_key(n.created_at), &n.id), NOTIFY_TYPE)
        .gsi2(keys::user(&n.user_id), format!("NOTIFY#{}", ts_key(n.created_at)))
        .ttl(Utc::now() + Duration::days(30));
    Record::new(meta, n).insert_op(Table::Core)
}
