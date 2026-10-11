//! WhatsApp, SMS and email (PLAN.md §19). OTP delivery is currently off
//! (sign-in is phone-number only); these carry utility notifications.

use async_trait::async_trait;
use chrono::Utc;
use kabados_db::repo::{Db, InboxMessage};

#[async_trait]
pub trait WhatsAppPort: Send + Sync {
    /// Sends an approved template with positional parameters.
    async fn send_template(&self, to_e164: &str, template: &str, params: &[String]) -> anyhow::Result<String>;
}

#[async_trait]
pub trait SmsPort: Send + Sync {
    /// `template_key` maps to a DLT Template ID; `body` must match the registered text.
    async fn send(&self, to_e164: &str, template_key: &str, body: &str) -> anyhow::Result<String>;
}

#[async_trait]
pub trait EmailPort: Send + Sync {
    async fn send(&self, to: &str, subject: &str, html: &str) -> anyhow::Result<String>;
}

/// Mock adapter: every message lands in the dev inbox (non-prod).
pub struct InboxMessenger {
    db: Db,
    env: String,
}

impl InboxMessenger {
    pub fn new(db: Db, env: &str) -> Self {
        Self { db, env: env.to_owned() }
    }

    async fn record(&self, channel: &str, to: &str, template: &str, body: String) -> anyhow::Result<String> {
        let id = kabados_domain::ids::NotificationId::new().to_string();
        let masked = if to.len() > 4 { format!("{}******{}", &to[..3.min(to.len())], &to[to.len() - 4..]) } else { to.to_owned() };
        self.db
            .inbox_put(
                &self.env,
                InboxMessage { id: id.clone(), channel: channel.into(), to: masked, template: template.into(), body, at: Utc::now() },
            )
            .await?;
        Ok(id)
    }
}

#[async_trait]
impl WhatsAppPort for InboxMessenger {
    async fn send_template(&self, to: &str, template: &str, params: &[String]) -> anyhow::Result<String> {
        self.record("WHATSAPP", to, template, params.join(" | ")).await
    }
}

#[async_trait]
impl SmsPort for InboxMessenger {
    async fn send(&self, to: &str, template_key: &str, body: &str) -> anyhow::Result<String> {
        self.record("SMS", to, template_key, body.to_owned()).await
    }
}

#[async_trait]
impl EmailPort for InboxMessenger {
    async fn send(&self, to: &str, subject: &str, html: &str) -> anyhow::Result<String> {
        self.record("EMAIL", to, subject, html.to_owned()).await
    }
}

/// AWS End User Messaging Social (WhatsApp) live adapter.
pub struct AwsWhatsApp {
    client: aws_sdk_socialmessaging::Client,
    phone_number_id: String,
}

impl AwsWhatsApp {
    pub fn new(client: aws_sdk_socialmessaging::Client, phone_number_id: String) -> Self {
        Self { client, phone_number_id }
    }
}

#[async_trait]
impl WhatsAppPort for AwsWhatsApp {
    async fn send_template(&self, to: &str, template: &str, params: &[String]) -> anyhow::Result<String> {
        let body = serde_json::json!({
            "messaging_product": "whatsapp",
            "to": to,
            "type": "template",
            "template": {
                "name": template,
                "language": {"code": "en"},
                "components": [{"type": "body", "parameters": params.iter().map(|p| serde_json::json!({"type": "text", "text": p})).collect::<Vec<_>>()}]
            }
        });
        let out = self
            .client
            .send_whats_app_message()
            .origination_phone_number_id(&self.phone_number_id)
            .meta_api_version("v20.0")
            .message(aws_sdk_socialmessaging::primitives::Blob::new(serde_json::to_vec(&body)?))
            .send()
            .await?;
        Ok(out.message_id().unwrap_or_default().to_owned())
    }
}

/// AWS End User Messaging SMS live adapter (India DLT entity + template IDs).
pub struct AwsSms {
    client: aws_sdk_pinpointsmsvoicev2::Client,
    /// `{"entity_id": "...", "origination_identity": "...", "templates": {"key": "templateId"}}`
    dlt: serde_json::Value,
}

impl AwsSms {
    pub fn new(client: aws_sdk_pinpointsmsvoicev2::Client, dlt: serde_json::Value) -> Self {
        Self { client, dlt }
    }
}

#[async_trait]
impl SmsPort for AwsSms {
    async fn send(&self, to: &str, template_key: &str, body: &str) -> anyhow::Result<String> {
        use aws_sdk_pinpointsmsvoicev2::types::DestinationCountryParameterKey as K;
        let template = self.dlt["templates"][template_key].as_str().ok_or_else(|| anyhow::anyhow!("no DLT template for {template_key}"))?;
        let out = self
            .client
            .send_text_message()
            .destination_phone_number(to)
            .message_body(body)
            .set_origination_identity(self.dlt["origination_identity"].as_str().map(str::to_owned))
            .destination_country_parameters(K::InEntityId, self.dlt["entity_id"].as_str().unwrap_or_default())
            .destination_country_parameters(K::InTemplateId, template)
            .send()
            .await?;
        Ok(out.message_id().unwrap_or_default().to_owned())
    }
}

/// Amazon SES live adapter.
pub struct SesEmail {
    client: aws_sdk_sesv2::Client,
    from: String,
}

impl SesEmail {
    pub fn new(client: aws_sdk_sesv2::Client, from: String) -> Self {
        Self { client, from }
    }
}

#[async_trait]
impl EmailPort for SesEmail {
    async fn send(&self, to: &str, subject: &str, html: &str) -> anyhow::Result<String> {
        use aws_sdk_sesv2::types::{Body, Content, Destination, EmailContent, Message};
        let msg = Message::builder()
            .subject(Content::builder().data(subject).charset("UTF-8").build()?)
            .body(Body::builder().html(Content::builder().data(html).charset("UTF-8").build()?).build())
            .build();
        let out = self
            .client
            .send_email()
            .from_email_address(&self.from)
            .destination(Destination::builder().to_addresses(to).build())
            .content(EmailContent::builder().simple(msg).build())
            .send()
            .await?;
        Ok(out.message_id().unwrap_or_default().to_owned())
    }
}
