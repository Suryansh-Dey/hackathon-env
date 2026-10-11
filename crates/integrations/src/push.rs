//! Android push via Amazon SNS mobile push → FCM (PLAN.md §8.2, §19.1).

use async_trait::async_trait;
use serde_json::Value;
use std::sync::Mutex;

#[async_trait]
pub trait PushPort: Send + Sync {
    /// Registers an FCM token; returns the SNS endpoint ARN.
    async fn register(&self, fcm_token: &str, user_data: &str) -> anyhow::Result<String>;
    /// High-priority data message (job offers use a full-screen intent on device).
    async fn send(&self, endpoint_arn: &str, data: &Value, ttl_secs: u32) -> anyhow::Result<()>;
}

pub struct SnsPush {
    client: aws_sdk_sns::Client,
    platform_app_arn: String,
}

impl SnsPush {
    pub fn new(client: aws_sdk_sns::Client, platform_app_arn: String) -> Self {
        Self { client, platform_app_arn }
    }
}

#[async_trait]
impl PushPort for SnsPush {
    async fn register(&self, fcm_token: &str, user_data: &str) -> anyhow::Result<String> {
        let out = self
            .client
            .create_platform_endpoint()
            .platform_application_arn(&self.platform_app_arn)
            .token(fcm_token)
            .custom_user_data(user_data)
            .send()
            .await?;
        out.endpoint_arn().map(str::to_owned).ok_or_else(|| anyhow::anyhow!("no endpoint ARN"))
    }

    async fn send(&self, endpoint_arn: &str, data: &Value, ttl_secs: u32) -> anyhow::Result<()> {
        let fcm = serde_json::json!({"fcmV1Message": {"message": {"data": data_strings(data), "android": {"priority": "high", "ttl": format!("{ttl_secs}s")}}}});
        let msg = serde_json::json!({"GCM": fcm.to_string()});
        self.client.publish().target_arn(endpoint_arn).message_structure("json").message(msg.to_string()).send().await?;
        Ok(())
    }
}

/// FCM data payloads must be string → string.
fn data_strings(v: &Value) -> Value {
    match v.as_object() {
        Some(m) => Value::Object(
            m.iter().map(|(k, v)| (k.clone(), Value::String(v.as_str().map(str::to_owned).unwrap_or_else(|| v.to_string())))).collect(),
        ),
        None => serde_json::json!({"payload": v.to_string()}),
    }
}

#[derive(Default)]
pub struct FakePush {
    pub sent: Mutex<Vec<(String, Value)>>,
}

#[async_trait]
impl PushPort for FakePush {
    async fn register(&self, fcm_token: &str, _user_data: &str) -> anyhow::Result<String> {
        Ok(format!("arn:local:sns:endpoint/{}", &fcm_token[..fcm_token.len().min(12)]))
    }

    async fn send(&self, endpoint_arn: &str, data: &Value, _ttl: u32) -> anyhow::Result<()> {
        self.sent.lock().unwrap().push((endpoint_arn.into(), data.clone()));
        Ok(())
    }
}
