//! Environment configuration and provider modes (PLAN.md §15.4, external_api.md §21).

use chrono::{DateTime, Utc};
use kabados_db::TableNames;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Mock,
    Sandbox,
    Live,
}

impl FromStr for Mode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "mock" => Ok(Mode::Mock),
            "sandbox" => Ok(Mode::Sandbox),
            "live" => Ok(Mode::Live),
            other => Err(format!("unknown provider mode {other:?}")),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderModes {
    pub whatsapp: Mode,
    pub sms: Mode,
    pub payments: Mode,
    pub route: Mode,
    pub payouts: Mode,
    pub kyc: Mode,
    pub integrity: Mode,
    pub ai: Mode,
}

impl ProviderModes {
    /// True when any user-facing flow is backed by a mock or sandbox, which
    /// makes clients show the "Test mode" banner (PLAN.md §15.4).
    pub fn test_mode(&self) -> bool {
        [self.whatsapp, self.sms, self.payments, self.route, self.payouts, self.kyc, self.integrity].iter().any(|m| *m != Mode::Live)
    }

    /// Money-moving, identity and AI integrations.
    fn critical(&self) -> [(&'static str, Mode); 7] {
        [
            ("payments", self.payments),
            ("route", self.route),
            ("payouts", self.payouts),
            ("kyc", self.kyc),
            ("sms", self.sms),
            ("whatsapp", self.whatsapp),
            ("ai", self.ai),
        ]
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    /// `local`, `ci`, `dev`, `staging` or `prod`.
    pub env: String,
    pub region: String,
    pub tables: TableNames,
    pub use_memory_store: bool,
    pub dynamodb_endpoint: Option<String>,
    pub media_bucket: String,
    pub documents_bucket: String,
    pub receipts_bucket: String,
    pub event_bus: Option<String>,
    pub appsync_http_domain: Option<String>,
    pub tracker_name: Option<String>,
    pub cognito_user_pool_id: Option<String>,
    pub cognito_client_id: Option<String>,
    pub staff_user_pool_id: Option<String>,
    pub avp_policy_store_id: Option<String>,
    pub kms_key_id: Option<String>,
    pub modes: ProviderModes,
    /// Phone numbers allowed fixed OTPs outside prod (PLAN.md §15.4).
    pub demo_numbers: Vec<String>,
    pub public_base_url: String,
    pub allowed_origins: Vec<String>,
    pub mock_override_until: Option<DateTime<Utc>>,
    pub first_look_minutes: i64,
    pub platform_fee_bps: i64,
}

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.is_empty())
}

fn mode(k: &str, default: Mode) -> Mode {
    env(k).and_then(|v| v.parse().ok()).unwrap_or(default)
}

impl Config {
    pub fn from_env() -> Self {
        let env_name = env("KABADOS_ENV").unwrap_or_else(|| "local".into());
        let local = env_name == "local" || env_name == "ci";
        let default = if local { Mode::Mock } else { Mode::Live };
        let prefix = env("TABLE_PREFIX").unwrap_or_else(|| format!("kabados-{env_name}"));
        // Explicit TABLE_* names (set by CDK) win over the prefix convention.
        let tables = if env("TABLE_CORE").is_some() { TableNames::from_env() } else { TableNames::with_prefix(&prefix) };
        let dynamodb_endpoint = env("DYNAMODB_ENDPOINT");
        Config {
            use_memory_store: local && dynamodb_endpoint.is_none() && env("KABADOS_STORE").as_deref() != Some("dynamodb"),
            dynamodb_endpoint,
            region: env("AWS_REGION").unwrap_or_else(|| "ap-south-1".into()),
            tables,
            media_bucket: env("MEDIA_BUCKET").unwrap_or_else(|| format!("kabados-{env_name}-media")),
            documents_bucket: env("DOCUMENTS_BUCKET").unwrap_or_else(|| format!("kabados-{env_name}-documents")),
            receipts_bucket: env("RECEIPTS_BUCKET").unwrap_or_else(|| format!("kabados-{env_name}-receipts")),
            event_bus: env("EVENT_BUS_NAME"),
            appsync_http_domain: env("APPSYNC_HTTP_DOMAIN"),
            tracker_name: env("LOCATION_TRACKER"),
            cognito_user_pool_id: env("COGNITO_USER_POOL_ID"),
            cognito_client_id: env("COGNITO_CLIENT_ID"),
            staff_user_pool_id: env("COGNITO_STAFF_POOL_ID"),
            avp_policy_store_id: env("AVP_POLICY_STORE_ID"),
            kms_key_id: env("KMS_KEY_ID"),
            modes: ProviderModes {
                whatsapp: mode("WHATSAPP_MODE", default),
                sms: mode("SMS_MODE", default),
                payments: mode("PAYMENTS_MODE", default),
                route: mode("ROUTE_MODE", default),
                payouts: mode("PAYOUTS_MODE", default),
                kyc: mode("KYC_MODE", default),
                integrity: mode("INTEGRITY_MODE", default),
                ai: mode("AI_MODE", default),
            },
            demo_numbers: env("DEMO_NUMBERS").map(|v| v.split(',').map(|s| s.trim().to_owned()).collect()).unwrap_or_default(),
            public_base_url: env("PUBLIC_BASE_URL").unwrap_or_else(|| "http://localhost:3000".into()),
            allowed_origins: env("ALLOWED_ORIGINS").map(|v| v.split(',').map(|s| s.trim().to_owned()).collect()).unwrap_or_default(),
            mock_override_until: env("MOCK_OVERRIDE_UNTIL")
                .and_then(|v| DateTime::parse_from_rfc3339(&v).ok())
                .map(|d| d.with_timezone(&Utc)),
            first_look_minutes: env("FIRST_LOOK_MINUTES").and_then(|v| v.parse().ok()).unwrap_or(25),
            platform_fee_bps: env("PLATFORM_FEE_BPS").and_then(|v| v.parse().ok()).unwrap_or(200),
            env: env_name,
        }
    }

    pub fn is_prod(&self) -> bool {
        self.env == "prod"
    }

    pub fn is_local(&self) -> bool {
        self.env == "local" || self.env == "ci"
    }

    /// Startup guard (external_api.md §21): prod refuses to start with a
    /// money-moving, identity or AI integration in `mock`, unless a
    /// time-limited override is set.
    pub fn guard(&self, now: DateTime<Utc>) -> Result<(), String> {
        if !self.is_prod() {
            return Ok(());
        }
        let mocked: Vec<&str> = self.modes.critical().iter().filter(|(_, m)| *m == Mode::Mock).map(|(n, _)| *n).collect();
        if mocked.is_empty() {
            return Ok(());
        }
        match self.mock_override_until {
            Some(until) if now < until => {
                tracing::error!(?mocked, %until, "prod running with mocked integrations under a time-limited override");
                Ok(())
            }
            _ => Err(format!("prod refuses to start with mocked integrations: {}", mocked.join(", "))),
        }
    }

    /// Fixed OTPs only for whitelisted demo numbers, never in prod.
    pub fn fixed_otp_allowed(&self, phone_e164: &str) -> bool {
        !self.is_prod() && self.demo_numbers.iter().any(|n| n == phone_e164)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(env: &str, m: Mode) -> Config {
        let mut c = Config::from_env();
        c.env = env.into();
        c.modes = ProviderModes { whatsapp: m, sms: m, payments: m, route: m, payouts: m, kyc: m, integrity: m, ai: m };
        c.demo_numbers = vec!["+919999999999".into()];
        c
    }

    #[test]
    fn prod_refuses_mocks_without_override() {
        let now = Utc::now();
        assert!(cfg("prod", Mode::Mock).guard(now).is_err());
        assert!(cfg("prod", Mode::Live).guard(now).is_ok());
        assert!(cfg("dev", Mode::Mock).guard(now).is_ok());
        let mut c = cfg("prod", Mode::Mock);
        c.mock_override_until = Some(now + chrono::Duration::hours(1));
        assert!(c.guard(now).is_ok());
        c.mock_override_until = Some(now - chrono::Duration::hours(1));
        assert!(c.guard(now).is_err());
    }

    #[test]
    fn fixed_otp_never_in_prod() {
        assert!(cfg("dev", Mode::Mock).fixed_otp_allowed("+919999999999"));
        assert!(!cfg("dev", Mode::Mock).fixed_otp_allowed("+919876543210"));
        assert!(!cfg("prod", Mode::Live).fixed_otp_allowed("+919999999999"));
    }

    #[test]
    fn test_mode_banner() {
        assert!(cfg("dev", Mode::Sandbox).modes.test_mode());
        assert!(!cfg("prod", Mode::Live).modes.test_mode());
    }
}
