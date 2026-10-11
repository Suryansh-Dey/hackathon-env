//! Coverage index rebuild worker (PLAN.md §12.5).
//!
//! The coverage index records which vendor prices which category in which r8
//! geo-cell. It is derived state: it must be rebuilt whenever a vendor's
//! published rule set changes (new service area or category list) or the
//! vendor's authorization changes (KYC approval, suspension, hazardous-waste
//! permit expiry), since any of those flips which `(cell, category)` pairs the
//! vendor can serve.
//!
//! This worker is an EventBridge consumer. It owns no index logic of its own:
//! it reacts to the relevant [`DomainEvent`]s and delegates to
//! [`kabados_api::coverage::rebuild`], which recomputes the vendor's full set of
//! pairs and diffs them against the stored index. `rebuild` is itself
//! idempotent — it reads current state and rewrites the vendor's rows to match —
//! so redelivery of the same event is safe without an explicit exactly-once
//! guard.

use kabados_domain::events::{DomainEvent, EventEnvelope};
use kabados_domain::ids::UserId;

/// Handles one domain event envelope.
///
/// On [`DomainEvent::RuleSetPublished`] and
/// [`DomainEvent::VendorAuthorizationChanged`], rebuilds the referenced vendor's
/// coverage index via [`kabados_api::coverage::rebuild`] and logs the
/// `(puts, deletes)` the rewrite applied. All other events are ignored.
pub async fn handle_event(ctx: &crate::WorkerCtx, env: &EventEnvelope) -> anyhow::Result<()> {
    let vendor_id: &UserId = match &env.event {
        DomainEvent::RuleSetPublished { vendor_id, .. } | DomainEvent::VendorAuthorizationChanged { vendor_id } => vendor_id,
        _ => return Ok(()),
    };

    rebuild_vendor(ctx, vendor_id, env.event.detail_type()).await
}

/// Rebuilds a single vendor's coverage index and logs the resulting diff.
async fn rebuild_vendor(ctx: &crate::WorkerCtx, vendor_id: &UserId, trigger: &str) -> anyhow::Result<()> {
    let (puts, deletes) = kabados_api::coverage::rebuild(&ctx.app, vendor_id).await.map_err(anyhow::Error::msg)?;

    tracing::info!(
        vendor = %vendor_id,
        trigger,
        puts,
        deletes,
        "coverage index rebuilt"
    );

    Ok(())
}
