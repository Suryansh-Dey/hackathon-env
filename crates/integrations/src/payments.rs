//! Razorpay payments, Route transfers and RazorpayX payouts (PLAN.md §13).
//!
//! `Razorpay` talks to the real REST API (test keys in dev, never live keys
//! for now). `MockRazorpay` produces the same order objects and signed
//! webhook payloads, which go through the same webhook receiver, ledger and
//! deal workflow as real ones.

use async_trait::async_trait;
use kabados_domain::Paise;
use kabados_runtime::crypto::{hmac_sha256_hex, verify_hmac_sha256_hex};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Mutex;

pub const API: &str = "https://api.razorpay.com/v1";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Order {
    pub id: String,
    pub amount: Paise,
    pub currency: String,
    pub receipt: String,
    pub status: String,
    /// Public key for Razorpay Checkout on the client.
    pub key_id: String,
    /// True when created by the mock adapter (clients complete it via the dev endpoint).
    pub simulated: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SignedWebhook {
    pub event_id: String,
    pub body: Vec<u8>,
    pub signature: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum WebhookEvent {
    PaymentCaptured { payment_id: String, order_id: String, amount: Paise, notes: BTreeMap<String, String> },
    PaymentFailed { payment_id: String, order_id: String, reason: Option<String> },
    RefundProcessed { refund_id: String, payment_id: String, amount: Paise },
    PayoutProcessed { payout_id: String, amount: Paise, reference_id: Option<String> },
    PayoutFailed { payout_id: String, reason: Option<String>, reference_id: Option<String> },
    Other(String),
}

fn notes_of(v: &serde_json::Value) -> BTreeMap<String, String> {
    v.as_object()
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.as_str().map(str::to_owned).unwrap_or_else(|| v.to_string()))).collect())
        .unwrap_or_default()
}

/// Parses a Razorpay webhook body (after signature verification).
pub fn parse_webhook(body: &[u8]) -> anyhow::Result<WebhookEvent> {
    let v: serde_json::Value = serde_json::from_slice(body)?;
    let event = v["event"].as_str().unwrap_or_default().to_owned();
    let pay = &v["payload"]["payment"]["entity"];
    Ok(match event.as_str() {
        "payment.captured" | "order.paid" => WebhookEvent::PaymentCaptured {
            payment_id: pay["id"].as_str().unwrap_or_default().into(),
            order_id: pay["order_id"].as_str().unwrap_or_default().into(),
            amount: Paise(pay["amount"].as_i64().unwrap_or(0)),
            notes: notes_of(&pay["notes"]),
        },
        "payment.failed" => WebhookEvent::PaymentFailed {
            payment_id: pay["id"].as_str().unwrap_or_default().into(),
            order_id: pay["order_id"].as_str().unwrap_or_default().into(),
            reason: pay["error_description"].as_str().map(str::to_owned),
        },
        "refund.processed" => {
            let r = &v["payload"]["refund"]["entity"];
            WebhookEvent::RefundProcessed {
                refund_id: r["id"].as_str().unwrap_or_default().into(),
                payment_id: r["payment_id"].as_str().unwrap_or_default().into(),
                amount: Paise(r["amount"].as_i64().unwrap_or(0)),
            }
        }
        "payout.processed" | "payout.failed" | "payout.reversed" => {
            let p = &v["payload"]["payout"]["entity"];
            let (id, reference) = (p["id"].as_str().unwrap_or_default().to_owned(), p["reference_id"].as_str().map(str::to_owned));
            if event == "payout.processed" {
                WebhookEvent::PayoutProcessed { payout_id: id, amount: Paise(p["amount"].as_i64().unwrap_or(0)), reference_id: reference }
            } else {
                WebhookEvent::PayoutFailed {
                    payout_id: id,
                    reason: p["failure_reason"].as_str().map(str::to_owned),
                    reference_id: reference,
                }
            }
        }
        other => WebhookEvent::Other(other.to_owned()),
    })
}

#[async_trait]
pub trait PaymentsPort: Send + Sync {
    async fn create_order(&self, amount: Paise, receipt: &str, notes: &BTreeMap<String, String>) -> anyhow::Result<Order>;
    /// `X-Razorpay-Signature` check over the raw body.
    fn verify_webhook(&self, body: &[u8], signature: &str) -> bool;
    /// Checkout handler check: HMAC(order_id|payment_id, key_secret).
    fn verify_checkout(&self, order_id: &str, payment_id: &str, signature: &str) -> bool;
    /// Mock only: a signed `payment.captured` webhook for an order.
    fn simulate_capture(&self, _order_id: &str, _amount: Paise, _notes: &BTreeMap<String, String>) -> Option<SignedWebhook> {
        None
    }
}

/// Razorpay REST adapter (sandbox = test keys; live keys are not used yet).
pub struct Razorpay {
    http: reqwest::Client,
    key_id: String,
    key_secret: String,
    webhook_secret: String,
}

impl Razorpay {
    pub fn new(key_id: String, key_secret: String, webhook_secret: String) -> Self {
        Self { http: reqwest::Client::new(), key_id, key_secret, webhook_secret }
    }
}

#[async_trait]
impl PaymentsPort for Razorpay {
    async fn create_order(&self, amount: Paise, receipt: &str, notes: &BTreeMap<String, String>) -> anyhow::Result<Order> {
        let res = self
            .http
            .post(format!("{API}/orders"))
            .basic_auth(&self.key_id, Some(&self.key_secret))
            .json(&serde_json::json!({"amount": amount.0, "currency": "INR", "receipt": receipt, "notes": notes}))
            .send()
            .await?;
        let status = res.status();
        let v: serde_json::Value = res.json().await?;
        if !status.is_success() {
            anyhow::bail!("razorpay order failed ({status}): {}", v["error"]["description"].as_str().unwrap_or("unknown"));
        }
        Ok(Order {
            id: v["id"].as_str().unwrap_or_default().into(),
            amount: Paise(v["amount"].as_i64().unwrap_or(amount.0)),
            currency: v["currency"].as_str().unwrap_or("INR").into(),
            receipt: receipt.into(),
            status: v["status"].as_str().unwrap_or("created").into(),
            key_id: self.key_id.clone(),
            simulated: false,
        })
    }

    fn verify_webhook(&self, body: &[u8], signature: &str) -> bool {
        verify_hmac_sha256_hex(self.webhook_secret.as_bytes(), body, signature)
    }

    fn verify_checkout(&self, order_id: &str, payment_id: &str, signature: &str) -> bool {
        verify_hmac_sha256_hex(self.key_secret.as_bytes(), format!("{order_id}|{payment_id}").as_bytes(), signature)
    }
}

/// Mock payments: realistic ids and signed, Razorpay-shaped webhook payloads.
pub struct MockRazorpay {
    webhook_secret: String,
}

impl MockRazorpay {
    pub fn new(webhook_secret: &str) -> Self {
        Self { webhook_secret: webhook_secret.into() }
    }
}

fn mock_id(prefix: &str) -> String {
    format!("{prefix}_{}", &kabados_domain::ids::EventId::new().as_str()[4..18])
}

#[async_trait]
impl PaymentsPort for MockRazorpay {
    async fn create_order(&self, amount: Paise, receipt: &str, _notes: &BTreeMap<String, String>) -> anyhow::Result<Order> {
        Ok(Order {
            id: mock_id("order"),
            amount,
            currency: "INR".into(),
            receipt: receipt.into(),
            status: "created".into(),
            key_id: "rzp_mock".into(),
            simulated: true,
        })
    }

    fn verify_webhook(&self, body: &[u8], signature: &str) -> bool {
        verify_hmac_sha256_hex(self.webhook_secret.as_bytes(), body, signature)
    }

    fn verify_checkout(&self, order_id: &str, payment_id: &str, signature: &str) -> bool {
        verify_hmac_sha256_hex(self.webhook_secret.as_bytes(), format!("{order_id}|{payment_id}").as_bytes(), signature)
    }

    fn simulate_capture(&self, order_id: &str, amount: Paise, notes: &BTreeMap<String, String>) -> Option<SignedWebhook> {
        let payment_id = mock_id("pay");
        let body = serde_json::json!({
            "entity": "event",
            "account_id": "acc_mock",
            "event": "payment.captured",
            "contains": ["payment"],
            "payload": {"payment": {"entity": {
                "id": payment_id, "entity": "payment", "amount": amount.0, "currency": "INR",
                "status": "captured", "order_id": order_id, "method": "upi", "notes": notes,
                "created_at": chrono::Utc::now().timestamp()
            }}},
            "created_at": chrono::Utc::now().timestamp()
        });
        let body = serde_json::to_vec(&body).ok()?;
        let signature = hmac_sha256_hex(self.webhook_secret.as_bytes(), &body);
        Some(SignedWebhook { event_id: mock_id("evt"), body, signature })
    }
}

#[async_trait]
pub trait RoutePort: Send + Sync {
    async fn create_linked_account(&self, user_ref: &str, name: &str) -> anyhow::Result<String>;
    async fn transfer(&self, payment_id: &str, account_id: &str, amount: Paise) -> anyhow::Result<String>;
}

/// Route mock: records linked accounts and transfers (Route needs activation).
#[derive(Default)]
pub struct MockRoute {
    transfers: Mutex<Vec<(String, String, Paise)>>,
}

#[async_trait]
impl RoutePort for MockRoute {
    async fn create_linked_account(&self, _user_ref: &str, _name: &str) -> anyhow::Result<String> {
        Ok(mock_id("acc"))
    }

    async fn transfer(&self, payment_id: &str, account_id: &str, amount: Paise) -> anyhow::Result<String> {
        self.transfers.lock().unwrap().push((payment_id.into(), account_id.into(), amount));
        Ok(mock_id("trf"))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FavResult {
    pub valid: bool,
    pub registered_name: Option<String>,
    pub fund_account_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Payout {
    pub id: String,
    pub status: String,
}

#[async_trait]
pub trait PayoutsPort: Send + Sync {
    /// Fund Account Validation of a UPI ID (RazorpayX Lite).
    async fn validate_vpa(&self, vpa: &str, contact_ref: &str) -> anyhow::Result<FavResult>;
    async fn payout(&self, fund_account_id: &str, amount: Paise, idempotency_key: &str) -> anyhow::Result<Payout>;
}

/// Payouts mock: format-validates VPAs; payouts settle immediately.
#[derive(Default)]
pub struct MockPayouts {
    seen: Mutex<BTreeMap<String, Payout>>,
}

#[async_trait]
impl PayoutsPort for MockPayouts {
    async fn validate_vpa(&self, vpa: &str, _contact_ref: &str) -> anyhow::Result<FavResult> {
        let valid = kabados_domain::user::is_valid_vpa(vpa);
        Ok(FavResult { valid, registered_name: None, fund_account_id: valid.then(|| mock_id("fa")) })
    }

    async fn payout(&self, _fund_account_id: &str, _amount: Paise, idempotency_key: &str) -> anyhow::Result<Payout> {
        let mut seen = self.seen.lock().unwrap();
        Ok(seen.entry(idempotency_key.into()).or_insert_with(|| Payout { id: mock_id("pout"), status: "processed".into() }).clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_capture_is_signed_and_parses() {
        let m = MockRazorpay::new("sec");
        let order = m.create_order(Paise(12_345), "deal_1", &BTreeMap::new()).await.unwrap();
        assert!(order.id.starts_with("order_") && order.simulated);
        let notes = BTreeMap::from([("deal_id".to_string(), "del_x".to_string())]);
        let wh = m.simulate_capture(&order.id, order.amount, &notes).unwrap();
        assert!(m.verify_webhook(&wh.body, &wh.signature));
        assert!(!MockRazorpay::new("other").verify_webhook(&wh.body, &wh.signature));
        match parse_webhook(&wh.body).unwrap() {
            WebhookEvent::PaymentCaptured { order_id, amount, notes, .. } => {
                assert_eq!(order_id, order.id);
                assert_eq!(amount, Paise(12_345));
                assert_eq!(notes["deal_id"], "del_x");
            }
            e => panic!("{e:?}"),
        }
    }

    #[test]
    fn parses_real_razorpay_shapes() {
        let body = br#"{"event":"payment.failed","payload":{"payment":{"entity":{"id":"pay_1","order_id":"order_1","error_description":"Payment was cancelled"}}}}"#;
        assert_eq!(
            parse_webhook(body).unwrap(),
            WebhookEvent::PaymentFailed {
                payment_id: "pay_1".into(),
                order_id: "order_1".into(),
                reason: Some("Payment was cancelled".into())
            }
        );
        let rz = Razorpay::new("rzp_test_x".into(), "secret".into(), "wh".into());
        let sig = hmac_sha256_hex(b"secret", b"order_1|pay_1");
        assert!(rz.verify_checkout("order_1", "pay_1", &sig));
    }

    #[tokio::test]
    async fn payouts_are_idempotent() {
        let p = MockPayouts::default();
        assert!(!p.validate_vpa("bad", "c").await.unwrap().valid);
        let a = p.payout("fa_1", Paise(100), "k1").await.unwrap();
        let b = p.payout("fa_1", Paise(100), "k1").await.unwrap();
        assert_eq!(a, b);
    }

    /// Calls Razorpay test mode when RAZORPAY_KEY_ID/RAZORPAY_KEY_SECRET are set.
    #[tokio::test]
    #[ignore = "needs Razorpay test keys"]
    async fn live_razorpay_test_mode_order() {
        let (Ok(id), Ok(secret)) = (std::env::var("RAZORPAY_KEY_ID"), std::env::var("RAZORPAY_KEY_SECRET")) else { return };
        assert!(id.starts_with("rzp_test_"), "only test keys are allowed");
        let rz = Razorpay::new(id, secret, "unused".into());
        let o = rz.create_order(Paise(100), "kabados-it", &BTreeMap::new()).await.unwrap();
        assert!(o.id.starts_with("order_"));
    }
}
