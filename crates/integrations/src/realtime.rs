//! Real-time channels via AWS AppSync Events, published from Rust with the
//! HTTP publish API signed by SigV4 (PLAN.md §17.4).

use async_trait::async_trait;
use aws_credential_types::provider::ProvideCredentials;
use serde_json::Value;
use tokio::sync::broadcast;

/// Channel namespace configured in the AppSync Events API.
pub const NAMESPACE: &str = "kabados";

#[async_trait]
pub trait RealtimePort: Send + Sync {
    /// Publishes events to `channel` (e.g. `listing/lst_…`).
    async fn publish(&self, channel: &str, events: &[Value]) -> anyhow::Result<()>;
}

pub fn channel_path(channel: &str) -> String {
    format!("/{NAMESPACE}/{}", channel.trim_start_matches('/'))
}

pub struct AppSyncEvents {
    http_domain: String,
    sdk: aws_config::SdkConfig,
    http: reqwest::Client,
}

impl AppSyncEvents {
    pub fn new(http_domain: String, sdk: aws_config::SdkConfig) -> Self {
        Self { http_domain, sdk, http: reqwest::Client::new() }
    }
}

#[async_trait]
impl RealtimePort for AppSyncEvents {
    async fn publish(&self, channel: &str, events: &[Value]) -> anyhow::Result<()> {
        use aws_sigv4::http_request::{SignableBody, SignableRequest, SigningSettings, sign};
        use aws_sigv4::sign::v4;
        if events.is_empty() {
            return Ok(());
        }
        let url = format!("https://{}/event", self.http_domain);
        let body = serde_json::to_vec(&serde_json::json!({
            "channel": channel_path(channel),
            "events": events.iter().map(|e| e.to_string()).collect::<Vec<_>>(),
        }))?;
        let creds = self.sdk.credentials_provider().ok_or_else(|| anyhow::anyhow!("no AWS credentials"))?.provide_credentials().await?;
        let identity = creds.into();
        let region = self.sdk.region().map(|r| r.to_string()).unwrap_or_else(|| "ap-south-1".into());
        let params = v4::SigningParams::builder()
            .identity(&identity)
            .region(&region)
            .name("appsync")
            .time(std::time::SystemTime::now())
            .settings(SigningSettings::default())
            .build()?
            .into();
        let headers = [("content-type", "application/json"), ("host", self.http_domain.as_str())];
        let signable = SignableRequest::new("POST", &url, headers.iter().copied(), SignableBody::Bytes(&body))?;
        let (instructions, _) = sign(signable, &params)?.into_parts();
        let mut req = self.http.post(&url).header("content-type", "application/json");
        for (k, v) in instructions.headers() {
            req = req.header(k, v);
        }
        let res = req.body(body).send().await?;
        if !res.status().is_success() {
            anyhow::bail!("appsync publish failed: {} {}", res.status(), res.text().await.unwrap_or_default());
        }
        Ok(())
    }
}

/// In-process hub for local development; the dev server streams it as SSE.
pub struct LocalHub {
    tx: broadcast::Sender<(String, Value)>,
}

impl Default for LocalHub {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalHub {
    pub fn new() -> Self {
        Self { tx: broadcast::channel(1024).0 }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<(String, Value)> {
        self.tx.subscribe()
    }
}

#[async_trait]
impl RealtimePort for LocalHub {
    async fn publish(&self, channel: &str, events: &[Value]) -> anyhow::Result<()> {
        for e in events {
            let _ = self.tx.send((channel_path(channel), e.clone()));
        }
        Ok(())
    }
}
