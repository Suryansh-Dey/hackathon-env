//! KabadOS worker Lambdas (PLAN.md §15.1, §17.3). Each module is a handler
//! invoked from a thin binary in `src/bin`. Shared setup lives in [`ctx`].

pub mod analysis;
pub mod coverage;
pub mod ctx;
pub mod notifications;
pub mod outbox;
pub mod rate_index;

pub use ctx::WorkerCtx;
