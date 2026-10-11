//! KabadOS worker Lambdas (PLAN.md §15.1, §17.3). Each module exposes pure
//! entry functions; thin binaries in `src/bin` adapt the Lambda event types to
//! them. Workers reuse [`kabados_api`]'s `AppState`, so pricing, matching,
//! settlement and repositories are never reimplemented.
//!
//! Module entry contracts (filled by the workers workflow):
//! - `outbox::handle_records(&WorkerCtx, &[StreamItem]) -> anyhow::Result<()>`
//!   DynamoDB Streams consumer: publishes new `outbox` items' events to
//!   EventBridge (`ctx.ix.events`) and dispatches new `notify` items to
//!   `notifications::deliver`. Idempotent via `ctx.db.first_time`.
//! - `notifications::deliver(&WorkerCtx, &Notification) -> anyhow::Result<()>`
//!   Channel fan-out per PLAN §19.1 (web push, WhatsApp/SMS utility templates,
//!   SNS→FCM for job offers, SES email); records delivery, honours quiet hours
//!   except urgent kinds.
//! - `analysis::handle_task(&WorkerCtx, Task) -> anyhow::Result<()>`
//!   SQS media/analysis consumer: ProcessImage (download original, EXIF strip,
//!   Rekognition face blur + moderation, thumbnail, pHash bands, triage agent,
//!   store processed copy), AnalyzeListing (itemize→verify agents, write items/
//!   claims/proofs, update ladder/status), VerifyListing, ProcessDocument
//!   (Textract queries). Uses `kabados_agents` with a runtime from env.
//! - `matching::handle_event(&WorkerCtx, &EventEnvelope) -> anyhow::Result<()>`
//!   On ListingPublished/ListingVersionCreated/ProofProvided: run the fast path
//!   (`kabados_api::matching::fast_path`), optionally the agent slow path, store
//!   the match result + per-vendor first-look leads (`MATCH#…` items under the
//!   vendor), emit MatchComputed/VendorNotified and notify the top-K.
//! - `coverage::handle_event(&WorkerCtx, &EventEnvelope) -> anyhow::Result<()>`
//!   On RuleSetPublished / VendorAuthorizationChanged: rebuild the vendor's
//!   coverage index via `kabados_api::coverage::rebuild`.
//! - `rate_index::run(&WorkerCtx, iso_week: &str) -> anyhow::Result<RateIndexRun>`
//!   Scheduled: aggregate settled-deal lines into weekly `(city,category,grade)`
//!   medians/IQR (`kabados_pricing::stats`), write unpublished `RateIndexEntry`
//!   rows (ops publishes above k-anonymity), and refresh `CityImpact`.

pub mod analysis;
pub mod coverage;
pub mod ctx;
pub mod matching;
pub mod notifications;
pub mod outbox;
pub mod rate_index;

pub use ctx::{StreamItem, WorkerCtx};
