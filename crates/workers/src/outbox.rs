//! Transactional-outbox → EventBridge bridge and notification dispatch
//! (PLAN.md §17.3, §19.2).
//!
//! State changes write `outbox` and `notify` items inside the same DynamoDB
//! transaction as the aggregate they belong to (see
//! [`kabados_db::repo::outbox_op`] / [`kabados_db::repo::notify_op`]). This
//! worker is the DynamoDB Streams consumer that fans those items out:
//!
//! - `outbox` items carry a serialized [`EventEnvelope`] under their `data`
//!   attribute; the envelope is published to EventBridge via
//!   [`EventBusPort::publish`](kabados_integrations::events::EventBusPort::publish).
//! - `notify` items carry a serialized [`Notification`]; it is handed to
//!   [`crate::notifications::deliver`] for channel fan-out.
//!
//! Stream records can be redelivered, so every externally visible side effect
//! is guarded by an exactly-once marker ([`Db::first_time`](kabados_db::repo::Db::first_time)).
//! A malformed record is a poison pill: it is logged and dropped so it never
//! wedges the shard. A transient failure (datastore, EventBridge, a delivery
//! channel) releases its idempotency marker and fails the batch so the Lambda
//! retries without double-dispatching the records that already succeeded.

use crate::{StreamItem, WorkerCtx};
use chrono::Duration;
use kabados_db::repo::{NOTIFY_TYPE, Notification, OUTBOX_TYPE};
use kabados_domain::events::EventEnvelope;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// Idempotency scope for published domain events.
const EVENTBUS_SCOPE: &str = "eventbus";
/// Idempotency scope for the outbox-side notify guard.
///
/// Deliberately *not* `"notify-deliver"`: [`crate::notifications::deliver`]
/// already dedupes internally with `first_time("notify-deliver", id)`, so
/// sharing that scope+key here would reserve the marker first and make
/// `deliver` a silent no-op. A distinct scope keeps both guards effective.
const NOTIFY_SCOPE: &str = "outbox-notify";
/// Markers outlive the outbox TTL so a very late redelivery still dedupes.
const GUARD_TTL_DAYS: i64 = 7;

/// Process a batch of DynamoDB Streams records.
///
/// INSERTs of `outbox` / `notify` items are dispatched; every other record is
/// ignored. Poison records are logged and skipped. If any record fails
/// transiently the whole batch is reported as failed (via `Err`) so the Lambda
/// redelivers it — the idempotency guards make that redelivery safe. Returns
/// `Ok(())` when every dispatchable record either succeeded or was a dropped
/// poison pill.
pub async fn handle_records(ctx: &WorkerCtx, items: &[StreamItem]) -> anyhow::Result<()> {
    let mut transient_failures = 0usize;

    for item in items {
        if !item.is_insert() {
            continue;
        }

        let outcome = match item.item_type() {
            Some(OUTBOX_TYPE) => dispatch_event(ctx, item).await,
            Some(NOTIFY_TYPE) => dispatch_notification(ctx, item).await,
            // Streams carry every mutation; only outbox/notify items are ours.
            _ => continue,
        };

        if let Err(err) = outcome {
            match err {
                RecordError::Poison(e) => {
                    tracing::warn!(error = %e, event = %item.event_name, "dropping malformed outbox record");
                }
                RecordError::Transient(e) => {
                    tracing::warn!(error = %e, event = %item.event_name, "transient failure dispatching outbox record; batch will retry");
                    transient_failures += 1;
                }
            }
        }
    }

    if transient_failures > 0 {
        anyhow::bail!("{transient_failures} outbox record(s) failed transiently; retrying batch");
    }
    Ok(())
}

/// Publish one `outbox` item's event to EventBridge, exactly once.
async fn dispatch_event(ctx: &WorkerCtx, item: &StreamItem) -> Result<(), RecordError> {
    let envelope: EventEnvelope = decode(item).map_err(RecordError::Poison)?;
    let key = envelope.id.to_string();

    if !claim(ctx, EVENTBUS_SCOPE, &key).await? {
        // Already published by an earlier delivery of this record.
        return Ok(());
    }

    if let Err(e) = ctx.ix.events.publish(std::slice::from_ref(&envelope)).await {
        release(ctx, EVENTBUS_SCOPE, &key).await;
        return Err(RecordError::Transient(e));
    }
    Ok(())
}

/// Deliver one `notify` item through the notification channels, exactly once.
async fn dispatch_notification(ctx: &WorkerCtx, item: &StreamItem) -> Result<(), RecordError> {
    let notification: Notification = decode(item).map_err(RecordError::Poison)?;
    let key = notification.id.to_string();

    if !claim(ctx, NOTIFY_SCOPE, &key).await? {
        return Ok(());
    }

    if let Err(e) = crate::notifications::deliver(ctx, &notification).await {
        release(ctx, NOTIFY_SCOPE, &key).await;
        return Err(RecordError::Transient(e));
    }
    Ok(())
}

/// Deserialize the entity stored under the record's `data` attribute.
///
/// [`Record::to_item`](kabados_db::record::Record) nests the entity under
/// `data` so it can never collide with envelope attributes, so that is where
/// the [`EventEnvelope`] / [`Notification`] lives.
fn decode<T: DeserializeOwned>(item: &StreamItem) -> anyhow::Result<T> {
    let image = item.new_image.as_ref().ok_or_else(|| anyhow::anyhow!("INSERT record without a new image"))?;
    let data = image.get("data").ok_or_else(|| anyhow::anyhow!("outbox record missing `data` attribute"))?;
    Ok(serde_json::from_value::<T>(Value::clone(data))?)
}

/// Reserve the exactly-once marker. `Ok(true)` means this delivery won the
/// race and must perform the side effect; `Ok(false)` means it was already
/// done. A datastore error is transient.
async fn claim(ctx: &WorkerCtx, scope: &str, key: &str) -> Result<bool, RecordError> {
    ctx.db
        .first_time(scope, key, Duration::days(GUARD_TTL_DAYS))
        .await
        .map_err(|e| RecordError::Transient(anyhow::Error::msg(e.to_string())))
}

/// Release a marker after the guarded side effect failed, so a batch retry can
/// try again. Best effort: if the release itself fails the batch still retries,
/// and the marker's TTL eventually reclaims it.
async fn release(ctx: &WorkerCtx, scope: &str, key: &str) {
    if let Err(e) = ctx.db.idem_abort(scope, key).await {
        tracing::warn!(error = %e, scope, "failed to release outbox idempotency marker after a transient error");
    }
}

/// Why a single stream record could not be dispatched.
enum RecordError {
    /// The record is malformed and can never succeed; drop it.
    Poison(anyhow::Error),
    /// A transient dependency failure; the batch should be retried.
    Transient(anyhow::Error),
}
