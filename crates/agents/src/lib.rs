//! `kabados-agents`: the KabadOS AI agents (PLAN.md §11.3–11.9) as a library.
//!
//! Each agent builds a versioned prompt, calls the model through
//! [`kabados_agent::AgentRuntime`] (budgets, retries, one repair, fail closed),
//! and applies deterministic post-processing so that AI informs while plain
//! code decides money and permissions. Lambda handlers live in the main tree.

pub mod classify;
pub mod common;
pub mod itemize;
pub mod keywords;
pub mod matching;
pub mod mock;
pub mod rules;
pub mod scale;
pub mod synthetic;
pub mod triage;
pub mod verify;

pub use classify::{TextVerdict, classify_text};
pub use common::ImageInput;
pub use itemize::{ItemDraft, ItemizeInput, ItemizeOutput, itemize};
pub use matching::{MatchResult, MatchedVendor, MatchingTools, SplitGroup, run_matching_agent};
pub use mock::mock_model;
pub use rules::{RulesAuthorInput, RulesDraftResult, author_rules};
pub use scale::{ScaleReading, read_scale};
pub use triage::triage;
pub use verify::{ClaimDraft, ProofRequestDraft, VerifyInput, VerifyOutput, verify};

pub use kabados_agent::{AgentError, AgentOutput, AgentResult, AgentRuntime};

use kabados_agent::{AiMode, GeminiModel, ModelClient, ModelConfig, ModelError};
use std::sync::Arc;

/// Model client selected by `AI_MODE`: the heuristic [`mock_model`] or live Gemini.
pub fn try_client_from_env() -> Result<Arc<dyn ModelClient>, ModelError> {
    match AiMode::from_env()? {
        AiMode::Mock => Ok(Arc::new(mock_model())),
        AiMode::Live => Ok(Arc::new(GeminiModel::from_env()?)),
    }
}

/// Panics on misconfiguration rather than silently running live traffic on the mock.
pub fn client_from_env() -> Arc<dyn ModelClient> {
    try_client_from_env().unwrap_or_else(|e| panic!("AI client misconfigured: {e}"))
}

/// Runtime with the agent-aware client, model IDs from env and an in-memory cache.
pub fn runtime_from_env() -> AgentRuntime {
    AgentRuntime::new(client_from_env(), ModelConfig::from_env())
}

/// Runtime over the offline mock (tests, demos, evals without `--live`).
pub fn mock_runtime() -> AgentRuntime {
    AgentRuntime::new(Arc::new(mock_model()), ModelConfig::default())
}
