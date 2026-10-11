//! Shared worker context and helpers.

use kabados_api::AppState;
use kabados_db::store::Item;

/// Worker runtime context — the same `AppState` the API uses (Deref to it).
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

/// A DynamoDB Streams change, normalized to JSON items (see `bin/worker-stream`).
#[derive(Clone, Debug)]
pub struct StreamItem {
    pub event_name: String,
    pub new_image: Option<Item>,
    pub old_image: Option<Item>,
}

impl StreamItem {
    /// The `type` attribute of the new image, if any.
    pub fn item_type(&self) -> Option<&str> {
        self.new_image.as_ref().and_then(|i| i.get("type")).and_then(|v| v.as_str())
    }

    pub fn is_insert(&self) -> bool {
        self.event_name == "INSERT"
    }
}

pub fn parse_task(body: &str) -> anyhow::Result<kabados_api::tasks::Task> {
    Ok(serde_json::from_str(body)?)
}
