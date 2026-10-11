//! Shared runtime plumbing for every KabadOS Lambda and the local dev server.

pub mod config;
pub mod crypto;
pub mod secrets;
pub mod telemetry;

pub use config::{Config, Mode, ProviderModes};

use kabados_db::Store;
use std::sync::Arc;

/// Loads the AWS SDK configuration for the configured region.
pub async fn aws_config(cfg: &Config) -> aws_config::SdkConfig {
    aws_config::defaults(aws_config::BehaviorVersion::latest()).region(aws_config::Region::new(cfg.region.clone())).load().await
}

/// Builds the storage backend: in-memory for `local`/tests unless
/// `DYNAMODB_ENDPOINT` points at DynamoDB Local, else real DynamoDB.
pub async fn store(cfg: &Config, sdk: Option<&aws_config::SdkConfig>) -> anyhow::Result<Arc<dyn Store>> {
    if cfg.use_memory_store {
        return Ok(Arc::new(kabados_db::memory::MemoryStore::new()));
    }
    let base = match sdk {
        Some(s) => s.clone(),
        None => aws_config(cfg).await,
    };
    let mut b = aws_sdk_dynamodb::config::Builder::from(&base);
    if let Some(ep) = &cfg.dynamodb_endpoint {
        b = b.endpoint_url(ep);
    }
    let client = aws_sdk_dynamodb::Client::from_conf(b.build());
    if cfg.dynamodb_endpoint.is_some() {
        kabados_db::schema::ensure_tables(&client, &cfg.tables).await.map_err(anyhow::Error::msg)?;
    }
    Ok(Arc::new(kabados_db::dynamo::DynamoStore::new(client, cfg.tables.clone())))
}
