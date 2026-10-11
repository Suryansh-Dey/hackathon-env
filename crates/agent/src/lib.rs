//! `kabados-agent`: a thin runtime over `gemini-client-api` (PLAN.md §11.2).
//!
//! Adds what the library leaves to the caller: step, token and wall-clock
//! budgets; retries with backoff on rate-limit and 5xx errors; schema
//! validation of every JSON output with one repair retry, then fail closed;
//! `tracing` spans per model and tool call; and an output cache keyed by
//! agent, prompt version and input hash.

pub mod cache;
pub mod gemini;
pub mod mock;
pub mod model;
pub mod runtime;

pub use cache::{InMemoryCache, NoCache, OutputCache, cache_key};
pub use gemini::GeminiModel;
pub use mock::{MockModel, schema_default};
pub use model::*;
pub use runtime::*;

use std::sync::Arc;

/// `AI_MODE` (external_api.md #11): `mock` (default; deterministic, offline) or `live`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AiMode {
    Mock,
    Live,
}

impl AiMode {
    pub fn from_env() -> Result<Self, ModelError> {
        match std::env::var("AI_MODE").unwrap_or_else(|_| "mock".into()).to_ascii_lowercase().as_str() {
            "mock" | "" => Ok(AiMode::Mock),
            "live" => Ok(AiMode::Live),
            other => Err(ModelError::NotConfigured(format!("AI_MODE must be mock or live, got {other:?}"))),
        }
    }
}

/// Builds the model client selected by `AI_MODE`. Returns an error instead of
/// silently falling back to the mock when live mode is misconfigured.
pub fn try_client_from_env() -> Result<Arc<dyn ModelClient>, ModelError> {
    match AiMode::from_env()? {
        AiMode::Mock => Ok(Arc::new(MockModel::new())),
        AiMode::Live => Ok(Arc::new(GeminiModel::from_env()?)),
    }
}

/// Like [`try_client_from_env`], panicking with a clear message on misconfiguration
/// (a live deployment must never run on the mock by accident).
pub fn client_from_env() -> Arc<dyn ModelClient> {
    try_client_from_env().unwrap_or_else(|e| panic!("AI client misconfigured: {e}"))
}
