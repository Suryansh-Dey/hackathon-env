//! Play Integrity verdicts (PLAN.md §14). Mock until the app is on Play.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IntegrityVerdict {
    pub device_integrity: Vec<String>,
    pub app_recognized: bool,
}

impl IntegrityVerdict {
    pub fn meets_device_integrity(&self) -> bool {
        self.device_integrity.iter().any(|v| v == "MEETS_DEVICE_INTEGRITY" || v == "MEETS_STRONG_INTEGRITY")
    }
}

#[async_trait]
pub trait IntegrityPort: Send + Sync {
    async fn verify(&self, token: &str) -> anyhow::Result<IntegrityVerdict>;
}

/// Dev builds return a configurable verdict (`MEETS_DEVICE_INTEGRITY` by default).
pub struct MockIntegrity {
    verdict: String,
}

impl Default for MockIntegrity {
    fn default() -> Self {
        Self { verdict: std::env::var("MOCK_INTEGRITY_VERDICT").unwrap_or_else(|_| "MEETS_DEVICE_INTEGRITY".into()) }
    }
}

#[async_trait]
impl IntegrityPort for MockIntegrity {
    async fn verify(&self, _token: &str) -> anyhow::Result<IntegrityVerdict> {
        Ok(IntegrityVerdict { device_integrity: vec![self.verdict.clone()], app_recognized: true })
    }
}
