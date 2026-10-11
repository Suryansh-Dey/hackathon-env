//! Typed KabadOS API client used by the transporter app and tooling.
//!
//! Request and response bodies are the `kabados-domain` types, so the client
//! can never drift from the server's wire format. Only the surface the
//! transporter app needs is modelled with named methods; [`Client::post`] /
//! [`Client::get`] cover anything else.

use kabados_domain::ids::{DealId, JobId};
use kabados_domain::job::{Job, JobEventKind, JobStatus};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::time::Duration;

pub mod models;
pub use models::*;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("network error: {0}")]
    Network(String),
    #[error("api error {status}: {title}{}", detail.as_deref().map(|d| format!(" — {d}")).unwrap_or_default())]
    Api { status: u16, title: String, detail: Option<String> },
    #[error("decode error: {0}")]
    Decode(String),
}

impl ClientError {
    /// Text to show the user: the problem's `detail` when the API gave one.
    pub fn message(&self) -> String {
        match self {
            ClientError::Api { detail: Some(d), .. } => d.clone(),
            ClientError::Api { status, title, .. } => format!("{title} ({status})"),
            ClientError::Network(e) => format!("Network error: {e}"),
            ClientError::Decode(e) => format!("Unexpected response: {e}"),
        }
    }
}

pub type Result<T> = std::result::Result<T, ClientError>;

/// A new `Idempotency-Key` for one user action. Keys must be fresh per
/// attempt: the API replays the stored response (errors included) for a
/// reused key.
pub fn fresh_key(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default();
    format!("{prefix}-{nanos}-{}", SEQ.fetch_add(1, Ordering::Relaxed))
}

/// Authentication token store (refresh handled by the caller/bridge).
#[derive(Clone, Default)]
pub struct Auth {
    pub bearer: Option<String>,
}

/// A KabadOS API client bound to one base URL.
#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base: String,
    auth: Auth,
}

impl Client {
    pub fn new(base_url: impl Into<String>) -> Self {
        let http = reqwest::Client::builder().timeout(Duration::from_secs(30)).build().unwrap_or_default();
        Self { http, base: base_url.into().trim_end_matches('/').to_owned(), auth: Auth::default() }
    }

    pub fn with_token(mut self, token: impl Into<String>) -> Self {
        self.auth.bearer = Some(token.into());
        self
    }

    pub fn set_token(&mut self, token: Option<String>) {
        self.auth.bearer = token;
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    async fn send<T: DeserializeOwned>(&self, req: reqwest::RequestBuilder) -> Result<T> {
        let mut req = req;
        if let Some(t) = &self.auth.bearer {
            req = req.bearer_auth(t);
        }
        let res = req.send().await.map_err(|e| ClientError::Network(e.to_string()))?;
        let status = res.status();
        let bytes = res.bytes().await.map_err(|e| ClientError::Network(e.to_string()))?;
        if status.is_success() {
            if bytes.is_empty() {
                return serde_json::from_slice(b"null").map_err(|e| ClientError::Decode(e.to_string()));
            }
            serde_json::from_slice(&bytes).map_err(|e| ClientError::Decode(e.to_string()))
        } else {
            let problem: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
            Err(ClientError::Api {
                status: status.as_u16(),
                title: problem["title"].as_str().unwrap_or("Error").to_owned(),
                detail: problem["detail"].as_str().map(str::to_owned),
            })
        }
    }

    /// Generic GET for any endpoint returning `T`.
    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        self.send(self.http.get(self.url(path))).await
    }

    /// Generic POST with a JSON body and an `Idempotency-Key`.
    pub async fn post<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B, idempotency_key: &str) -> Result<T> {
        self.send(self.http.post(self.url(path)).header("idempotency-key", idempotency_key).json(body)).await
    }

    async fn post_empty<T: DeserializeOwned>(&self, path: &str, idempotency_key: &str) -> Result<T> {
        self.send(self.http.post(self.url(path)).header("idempotency-key", idempotency_key)).await
    }

    // ---- sign-in and profile ----

    /// Dev sign-in (`POST /dev/token`): phone only, no OTP. Prod uses Cognito.
    pub async fn dev_token(&self, phone: &str) -> Result<DevToken> {
        self.send(self.http.post(self.url("/dev/token")).json(&serde_json::json!({ "phone": phone }))).await
    }

    pub async fn me(&self) -> Result<MeView> {
        self.get("/v1/me").await
    }

    pub async fn patch_me(&self, body: &PatchMe, key: &str) -> Result<MeView> {
        self.send(self.http.patch(self.url("/v1/me")).header("idempotency-key", key).json(body)).await
    }

    pub async fn apply_transporter(&self, body: &ApplyTransporter, key: &str) -> Result<MeView> {
        self.post("/v1/me/roles/TRANSPORTER:apply", body, key).await
    }

    // ---- transporter surface ----

    pub async fn set_availability(&self, on_duty: bool) -> Result<Availability> {
        let req = self.http.put(self.url("/v1/transporter/availability")).header("idempotency-key", fresh_key("duty"));
        self.send(req.json(&Availability { on_duty })).await
    }

    pub async fn register_device(&self, d: &DeviceReg) -> Result<kabados_domain::user::Device> {
        self.send(self.http.put(self.url("/v1/transporter/device")).json(d)).await
    }

    pub async fn job_offers(&self) -> Result<Vec<JobOfferView>> {
        self.get("/v1/transporter/job-offers").await
    }

    pub async fn jobs(&self, status: Option<JobStatus>) -> Result<Vec<Job>> {
        let q = status.map(|s| format!("?status={}", s.as_str())).unwrap_or_default();
        self.get(&format!("/v1/transporter/jobs{q}")).await
    }

    pub async fn job(&self, id: &JobId) -> Result<JobView> {
        self.get(&format!("/v1/jobs/{id}")).await
    }

    pub async fn accept_job(&self, id: &JobId, key: &str) -> Result<Job> {
        self.post_empty(&format!("/v1/jobs/{id}:accept"), key).await
    }

    pub async fn decline_job(&self, id: &JobId, key: &str) -> Result<Job> {
        self.post_empty(&format!("/v1/jobs/{id}:decline"), key).await
    }

    pub async fn post_job_event(&self, id: &JobId, kind: JobEventKind, lat: Option<f64>, lng: Option<f64>, key: &str) -> Result<Job> {
        self.post(&format!("/v1/jobs/{id}/events"), &serde_json::json!({"kind": kind, "lat": lat, "lng": lng}), key).await
    }

    pub async fn post_positions(&self, id: &JobId, positions: &[Position], key: &str) -> Result<()> {
        let _: serde_json::Value =
            self.post(&format!("/v1/jobs/{id}/positions"), &serde_json::json!({"positions": positions}), key).await?;
        Ok(())
    }

    pub async fn deal(&self, id: &DealId) -> Result<DealView> {
        self.get(&format!("/v1/deals/{id}")).await
    }

    /// Records a scale reading; returns the deal with server-computed line amounts.
    pub async fn weigh(&self, deal: &DealId, line_n: u32, reading_g: Grams, image_id: Option<&str>, key: &str) -> Result<DealView> {
        self.post(
            &format!("/v1/deals/{deal}/weighings"),
            &serde_json::json!({"line_n": line_n, "reading_g": reading_g, "image_id": image_id}),
            key,
        )
        .await
    }

    /// Verifies the seller's handover code; the job moves to LOADED.
    pub async fn handover(&self, deal: &DealId, otp: &str, key: &str) -> Result<DealView> {
        self.post(&format!("/v1/deals/{deal}:handover"), &serde_json::json!({"otp": otp}), key).await
    }
}

/// Domain modules re-exported so callers need only depend on the client.
pub use kabados_domain::{Grams, Paise, deal, geo, ids, job, offer, user};
