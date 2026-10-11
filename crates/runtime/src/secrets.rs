//! Secrets Manager access with an in-memory cache (read once per container,
//! PLAN.md §11.2). Locally, secrets fall back to environment variables.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct Secrets {
    client: Option<aws_sdk_secretsmanager::Client>,
    env: String,
    cache: Arc<RwLock<HashMap<String, String>>>,
}

impl Secrets {
    pub fn new(client: Option<aws_sdk_secretsmanager::Client>, env: impl Into<String>) -> Self {
        Self { client, env: env.into(), cache: Arc::default() }
    }

    /// Secret at `/kabados/{env}/{name}`; locally from env var `fallback_env`.
    pub async fn get(&self, name: &str, fallback_env: &str) -> anyhow::Result<String> {
        let id = format!("/kabados/{}/{name}", self.env);
        if let Some(v) = self.cache.read().await.get(&id) {
            return Ok(v.clone());
        }
        let value = match &self.client {
            Some(c) => match c.get_secret_value().secret_id(&id).send().await {
                Ok(out) => out.secret_string().map(str::to_owned).ok_or_else(|| anyhow::anyhow!("secret {id} has no string value"))?,
                Err(e) => match std::env::var(fallback_env) {
                    Ok(v) if !v.is_empty() => v,
                    _ => return Err(anyhow::anyhow!("secret {id}: {}", e.into_service_error())),
                },
            },
            None => std::env::var(fallback_env).map_err(|_| anyhow::anyhow!("{fallback_env} not set (secret {id})"))?,
        };
        self.cache.write().await.insert(id, value.clone());
        Ok(value)
    }

    /// Optional secret: `None` when absent.
    pub async fn try_get(&self, name: &str, fallback_env: &str) -> Option<String> {
        self.get(name, fallback_env).await.ok()
    }
}
