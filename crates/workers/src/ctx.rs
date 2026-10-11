//! Shared worker context: the same `AppState` the API uses, so workers reuse
//! the pricing, matching, settlement and repository code without duplication.

use kabados_api::AppState;
use std::sync::Arc;

/// Worker runtime context. Thin wrapper over [`AppState`] for now; add
/// worker-only clients here as needed.
#[derive(Clone)]
pub struct WorkerCtx {
    pub app: AppState,
}

impl WorkerCtx {
    pub async fn from_env() -> anyhow::Result<Self> {
        Ok(Self { app: AppState::from_env().await? })
    }

    pub fn new(app: AppState) -> Self {
        Self { app }
    }
}

impl std::ops::Deref for WorkerCtx {
    type Target = AppState;
    fn deref(&self) -> &AppState {
        &self.app
    }
}

/// Parses an SQS record body as a [`kabados_api::tasks::Task`].
pub fn parse_task(body: &str) -> anyhow::Result<kabados_api::tasks::Task> {
    Ok(serde_json::from_str(body)?)
}

pub fn shared() -> Arc<()> {
    Arc::new(())
}
