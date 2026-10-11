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
        let app = AppState::from_env().await?;
        // Agents read GEMINI_API_KEY from the environment; load it from Secrets
        // Manager once at init (Lambda init is single-threaded) when AI is live.
        if app.cfg.modes.ai != kabados_runtime::Mode::Mock && std::env::var("GEMINI_API_KEY").is_err() {
            if let Ok(key) = app.secrets.get("gemini/api_key", "GEMINI_API_KEY").await {
                // SAFETY: set during single-threaded Lambda init, before any agent runs.
                unsafe { std::env::set_var("GEMINI_API_KEY", key) };
            }
        }
        Ok(Self { app })
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
