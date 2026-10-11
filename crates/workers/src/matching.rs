//! Matching worker (PLAN.md §7.3, §12.7).
//!
//! Reacts to listing lifecycle events and (re)computes the set of vendors that
//! can buy a published lot, using [`kabados_api::matching::fast_path`] — the
//! same coverage-index → rules-engine → rank → cache pipeline the synchronous
//! API uses. For each top-K vendor it writes a *first-look lead* row under the
//! vendor's partition (read back by `GET /v1/vendor/matches`), emits the
//! `MatchComputed`/`VendorNotified` domain events and raises a `NewMatch`
//! notification so eligible vendors get first dibs during the first-look window.
//!
//! The fast path is sufficient for v1. The agent slow path (PLAN.md §11.6 —
//! widening the ring, relaxing constraints and reasoning over near-misses when
//! too few vendors cover the whole lot) is a future extension and is
//! deliberately not invoked here.
//!
//! ## Idempotency
//! Workers may be redelivered, so every effect is safe to repeat:
//! * the stored match result and the per-vendor lead rows are keyed
//!   deterministically by `(listing, ver)` and are *overwritten* in place;
//! * event publication and notifications (which would otherwise mint fresh
//!   random ids on each delivery and thus duplicate) are gated behind a
//!   [`Db::first_time`](kabados_db::repo::Db::first_time) exactly-once guard
//!   keyed by `(listing, ver)`.

use crate::WorkerCtx;
use chrono::{DateTime, Duration, Utc};
use kabados_api::matching::{self, FastPathResult, MatchedVendor};
use kabados_api::routes::vendor::MatchLead;
use kabados_db::repo::{Notification, NotificationKind, StoredMatch, notify_op};
use kabados_db::{Meta, Record, Table, keys};
use kabados_domain::events::{DomainEvent, EventEnvelope};
use kabados_domain::ids::{ListingId, UserId};
use kabados_domain::listing::{Listing, ListingStatus};

/// Number of top-ranked vendors that receive a first-look lead and a push.
const TOP_K: usize = 5;

/// How long the exactly-once guard for notifications/events is remembered.
const GUARD_TTL_DAYS: i64 = 7;

/// EventBridge consumer entry point: run matching on the lifecycle events that
/// change which vendors should see a lot. All other events are ignored.
pub async fn handle_event(ctx: &WorkerCtx, env: &EventEnvelope) -> anyhow::Result<()> {
    match &env.event {
        DomainEvent::ListingPublished { listing_id, ver }
        | DomainEvent::ListingVersionCreated { listing_id, ver }
        | DomainEvent::ProofProvided { listing_id, ver, .. } => run(ctx, listing_id, *ver).await,
        _ => Ok(()),
    }
}

/// Compute and persist matches for one published listing version.
///
/// Loads the bundle; if the listing is gone or no longer `PUBLISHED` this is a
/// no-op (the lot has been reserved, withdrawn or superseded). Otherwise it
/// runs the fast path, stores the result, writes the top-K first-look leads and
/// — once per `(listing, ver)` — notifies those vendors.
pub async fn run(ctx: &WorkerCtx, listing_id: &ListingId, ver: u32) -> anyhow::Result<()> {
    let Some(bundle) = ctx.db.listing_bundle(listing_id, Some(ver)).await? else {
        return Ok(());
    };
    if bundle.listing.data.status != ListingStatus::Published {
        return Ok(());
    }
    let listing = &bundle.listing.data;

    // Shared fast path: coverage index → rules engine → rank → 6h cache.
    let fp = matching::fast_path(&ctx.app, &bundle, matching::DEFAULT_RING).await?;

    // Persist the full result for this version (idempotent overwrite).
    ctx.db
        .put_match_result(
            listing_id,
            ver,
            StoredMatch { result: serde_json::to_value(&fp)?, cache_key: fp.cache_key.clone(), computed_at: Utc::now() },
        )
        .await?;

    // Deterministic timestamp so re-runs overwrite the same lead rows rather
    // than appending duplicates; it also orders a vendor's matches by publish
    // time (newest first, via the reversed `begins_with("MATCH#")` query).
    let at = listing.published_at.unwrap_or(listing.created_at);
    let top = top_k(&fp);

    write_leads(ctx, listing, ver, at, top).await?;

    // Notifications and events mint fresh ids per delivery, so gate them.
    let guard_key = format!("{listing_id}:{ver}");
    if ctx.db.first_time("match", &guard_key, Duration::days(GUARD_TTL_DAYS)).await? {
        emit(ctx, listing, ver, fp.from_cache, top).await?;
    }
    Ok(())
}

/// The top-K ranked eligible vendors (best first).
fn top_k(fp: &FastPathResult) -> &[MatchedVendor] {
    &fp.matches[..fp.matches.len().min(TOP_K)]
}

/// Whether the exclusive first-look window is still open (an unset window is
/// treated as open so matching still pushes to vendors).
fn in_first_look_window(listing: &Listing) -> bool {
    listing.first_look_until.is_none_or(|until| Utc::now() < until)
}

/// Writes one first-look lead row per top-K vendor under their user partition.
///
/// Key layout matches `GET /v1/vendor/matches`:
/// `PK = USER#<vendor_id>`, `SK = MATCH#<ts>#<listing_id>`, read with
/// `begins_with("MATCH#").reverse()` and no secondary index.
async fn write_leads(ctx: &WorkerCtx, listing: &Listing, ver: u32, at: DateTime<Utc>, top: &[MatchedVendor]) -> anyhow::Result<()> {
    let ts = ts_key(at);
    for m in top {
        let lead = MatchLead {
            listing_id: listing.id.clone(),
            ver,
            quote_low: m.quote.quote_low,
            quote_high: m.quote.quote_high,
            coverage_permille: m.quote.coverage_permille,
            distance_m: m.distance_m,
            first_look_until: listing.first_look_until,
            at,
        };
        let rec = Record::new(Meta::new(keys::user(&m.vendor_id), format!("MATCH#{ts}#{}", listing.id), "match_lead"), lead);
        ctx.db.put_rec(Table::Core, &rec).await?;
    }
    Ok(())
}

/// Publishes `MatchComputed`, and — only while the first-look window is open —
/// one `VendorNotified` event plus a `NewMatch` notification per top-K vendor.
/// Called at most once per `(listing, ver)` thanks to the `first_time` guard.
async fn emit(ctx: &WorkerCtx, listing: &Listing, ver: u32, from_cache: bool, top: &[MatchedVendor]) -> anyhow::Result<()> {
    let vendor_ids: Vec<UserId> = top.iter().map(|m| m.vendor_id.clone()).collect();
    let mut events = vec![EventEnvelope::new(DomainEvent::MatchComputed {
        listing_id: listing.id.clone(),
        ver,
        vendor_ids,
        from_cache,
        agent_used: false,
    })];

    if in_first_look_window(listing) && !top.is_empty() {
        let mut notes = Vec::with_capacity(top.len());
        for m in top {
            events.push(EventEnvelope::new(DomainEvent::VendorNotified { listing_id: listing.id.clone(), vendor_id: m.vendor_id.clone() }));
            let note = Notification::new(
                m.vendor_id.clone(),
                NotificationKind::NewMatch,
                "New match near you",
                format!("A lot you can buy was just posted: {}", listing.title),
            )
            .link(format!("/matches?listing={}", listing.id))
            .data(serde_json::json!({ "listing_id": listing.id, "ver": ver }));
            notes.push(notify_op(&keys::listing(&listing.id), note)?);
        }
        // Single transaction (≤ TOP_K items) under the listing partition.
        ctx.db.write_ops(notes).await?;
    }

    ctx.ix.events.publish(&events).await?;
    Ok(())
}

/// RFC3339 microsecond timestamp key — lexical order equals chronological order,
/// matching the format used elsewhere for `MATCH#`/`NOTIFY#` sort keys.
fn ts_key(t: DateTime<Utc>) -> String {
    t.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()
}
