//! TTL items in the `ephemeral` table (PLAN.md §16.7).

use super::{Db, ts_key};
use crate::keys;
use crate::record::{Meta, Record};
use crate::store::*;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IdemRecord {
    pub request_hash: String,
    pub status: Option<u16>,
    pub body: Option<serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum IdemState {
    /// First time: proceed and call [`Db::idem_complete`].
    New,
    /// Another request with this key is being processed.
    InProgress,
    /// Completed before; replay the stored response.
    Done { status: u16, body: serde_json::Value },
    /// Same key reused with a different request body.
    Mismatch,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OtpChannel {
    Whatsapp,
    Sms,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct InboxMessage {
    pub id: String,
    pub channel: String,
    pub to: String,
    pub template: String,
    pub body: String,
    pub at: DateTime<Utc>,
}

impl Db {
    /// Claims an idempotency key (PLAN.md §17.1, §13.4 webhooks).
    pub async fn idem_begin(&self, scope: &str, key: &str, request_hash: &str, ttl: Duration) -> DbResult<IdemState> {
        let rec = Record::new(
            Meta::new(keys::idem(scope, key), keys::IDEM, "idempotency").ttl(Utc::now() + ttl),
            IdemRecord { request_hash: request_hash.into(), status: None, body: None },
        );
        match self.insert_rec(Table::Ephemeral, &rec).await {
            Ok(()) => Ok(IdemState::New),
            Err(e) if e.is_conflict() => {
                let Some(existing) = self.get_rec::<IdemRecord>(Table::Ephemeral, &keys::idem(scope, key), keys::IDEM).await? else {
                    return Ok(IdemState::InProgress);
                };
                if existing.data.request_hash != request_hash {
                    return Ok(IdemState::Mismatch);
                }
                Ok(match (existing.data.status, existing.data.body) {
                    (Some(status), Some(body)) => IdemState::Done { status, body },
                    _ => IdemState::InProgress,
                })
            }
            Err(e) => Err(e),
        }
    }

    pub async fn idem_complete(
        &self,
        scope: &str,
        key: &str,
        request_hash: &str,
        status: u16,
        body: serde_json::Value,
        ttl: Duration,
    ) -> DbResult<()> {
        let rec = Record::new(
            Meta::new(keys::idem(scope, key), keys::IDEM, "idempotency").ttl(Utc::now() + ttl),
            IdemRecord { request_hash: request_hash.into(), status: Some(status), body: Some(body) },
        );
        self.put_rec(Table::Ephemeral, &rec).await
    }

    /// Releases a key after a failed request so the client can retry.
    pub async fn idem_abort(&self, scope: &str, key: &str) -> DbResult<()> {
        self.store.delete(Table::Ephemeral, &keys::idem(scope, key), keys::IDEM, None).await
    }

    /// Webhook/event exactly-once guard; returns false if already seen.
    pub async fn first_time(&self, scope: &str, key: &str, ttl: Duration) -> DbResult<bool> {
        let rec = Record::new(Meta::new(keys::idem(scope, key), keys::IDEM, "seen").ttl(Utc::now() + ttl), serde_json::json!({}));
        match self.insert_rec(Table::Ephemeral, &rec).await {
            Ok(()) => Ok(true),
            Err(e) if e.is_conflict() => Ok(false),
            Err(e) => Err(e),
        }
    }

    pub async fn set_otp_channel(&self, phone_hash: &str, ch: OtpChannel) -> DbResult<()> {
        let rec =
            Record::new(Meta::new(keys::otp_channel(phone_hash), keys::OTPCH, "otp_channel").ttl(Utc::now() + Duration::minutes(10)), ch);
        self.put_rec(Table::Ephemeral, &rec).await
    }

    pub async fn otp_channel(&self, phone_hash: &str) -> DbResult<Option<OtpChannel>> {
        Ok(self.get_rec::<OtpChannel>(Table::Ephemeral, &keys::otp_channel(phone_hash), keys::OTPCH).await?.map(|r| r.data))
    }

    pub async fn match_cache_get(&self, key: &str) -> DbResult<Option<serde_json::Value>> {
        Ok(self.get_rec::<serde_json::Value>(Table::Ephemeral, &keys::match_cache(key), keys::MATCH).await?.map(|r| r.data))
    }

    pub async fn match_cache_put(&self, key: &str, value: &serde_json::Value, ttl: Duration) -> DbResult<()> {
        let rec = Record::new(Meta::new(keys::match_cache(key), keys::MATCH, "match_cache").ttl(Utc::now() + ttl), value.clone());
        self.put_rec(Table::Ephemeral, &rec).await
    }

    /// Fixed-window rate limit; returns true if the call is allowed.
    pub async fn rate_limit(&self, scope: &str, subject: &str, window_secs: i64, max: i64) -> DbResult<bool> {
        let now = Utc::now().timestamp();
        let window = now - now.rem_euclid(window_secs);
        let n = self
            .store
            .increment(
                Table::Ephemeral,
                &keys::rate_limit(scope, subject, &window.to_string()),
                keys::RL,
                "n",
                1,
                Some(window + window_secs * 2),
            )
            .await?;
        Ok(n <= max)
    }

    /// Non-prod dev inbox for mocked WhatsApp/SMS/email (PLAN.md §15.4).
    pub async fn inbox_put(&self, env: &str, m: InboxMessage) -> DbResult<()> {
        let rec =
            Record::new(Meta::new(keys::inbox(env), format!("{}#{}", ts_key(m.at), m.id), "inbox").ttl(Utc::now() + Duration::days(7)), m);
        self.put_rec(Table::Ephemeral, &rec).await
    }

    pub async fn inbox(&self, env: &str, limit: u32) -> DbResult<Vec<InboxMessage>> {
        Ok(self
            .query_recs::<InboxMessage>(Query::new(Table::Ephemeral, keys::inbox(env)).reverse().limit(limit))
            .await?
            .0
            .into_iter()
            .map(|r| r.data)
            .collect())
    }
}
