//! Out-of-band notification fan-out (PLAN.md §19.1 channel matrix, §19.2
//! delivery mechanics).
//!
//! The in-app real-time push (AppSync Events / Web Push) is published at the
//! point the state change happens, so this module handles only the *out-of-band*
//! channels — WhatsApp/SMS utility templates, SNS→FCM high-priority data
//! messages for transporter job offers, and SES email with a WhatsApp document
//! for receipts.
//!
//! The channel order per [`NotificationKind`] follows the §19.1 matrix:
//!
//! | Kind(s) | Out-of-band channels |
//! |---|---|
//! | `AnalysisDone`, `ProofRequested` | WhatsApp utility template |
//! | `NewMatch` | WhatsApp utility template (Web Push is in-app) |
//! | `NewOffer`, `CounterOffer`, `OfferAccepted` | WhatsApp utility template |
//! | `JobOffer` | SNS→FCM high-priority data message to the transporter's devices |
//! | `PickupEta`, `Arrived` | WhatsApp utility template |
//! | `WeightsToConfirm`, `PaymentRequested`, `PaymentReceived` | WhatsApp → SMS |
//! | `HandoverDone`, `JobAccepted`, `DriveReminder`, `Dispute`, `ChatMessage`, `KycDecision` | WhatsApp utility template |
//! | `Receipt` | SES email + WhatsApp document template |
//!
//! Recipients' phone numbers are stored only as a salted hash plus the last
//! four digits ([`UserProfile::phone_last4`]), so the out-of-band messengers are
//! called with a best-effort `phone_last4` recipient string. Under the mock
//! adapters this simply lands the message in the dev inbox, which is the
//! intended non-OTP behaviour; the live adapters resolve the real E.164 number
//! from the hash before sending.
//!
//! Delivery is idempotent (workers may be redelivered): a [`Db::first_time`]
//! guard keyed by the notification id ensures each notification is fanned out
//! at most once. Every channel is best-effort — a failing channel logs a
//! warning and never aborts the others — and a delivery audit line is always
//! recorded on the first pass.

use crate::WorkerCtx;
use chrono::Duration;
use kabados_db::repo::{Notification, NotificationKind};
use kabados_domain::user::{Device, UserProfile};

/// Idempotency scope for the once-only delivery guard.
const DELIVER_SCOPE: &str = "notify-deliver";

/// How long the delivery guard remembers a notification id. Comfortably longer
/// than the Streams/SQS retry and DLQ-replay windows.
const GUARD_TTL_DAYS: i64 = 7;

/// High-priority FCM time-to-live (seconds) for a transporter job offer.
const JOB_OFFER_PUSH_TTL: u32 = 60;

/// Deliver one notification over its out-of-band channels (PLAN.md §19.1).
///
/// Looks up the recipient, dispatches on [`Notification::kind`], records a
/// delivery audit line and returns `Ok(())`. Channel failures are logged and
/// swallowed so a single degraded provider never blocks the rest; only an
/// infrastructure failure of the idempotency guard itself is propagated.
pub async fn deliver(ctx: &WorkerCtx, n: &Notification) -> anyhow::Result<()> {
    // Exactly-once guard: a redelivered notification is a no-op. Keyed by the
    // stable notification id so retries across Streams/SQS collapse to one.
    if !ctx.db.first_time(DELIVER_SCOPE, n.id.as_str(), Duration::days(GUARD_TTL_DAYS)).await? {
        tracing::debug!(notification = %n.id, "notification already delivered; skipping fan-out");
        return Ok(());
    }

    // Quiet hours (PLAN.md §19.2): user preferences and a nightly quiet window
    // suppress non-urgent kinds (`NotificationKind::urgent() == false`), while
    // security and active-job kinds always deliver. There is no per-user
    // quiet-hours / preference store yet, so every kind is delivered for now;
    // when that store lands, a `!n.kind.urgent()` check against it would gate
    // (defer) delivery here. Intentionally not a `todo!()` — delivery proceeds.

    // Best-effort recipient string: the plaintext number is never stored, only
    // its hash and last four digits.
    let recipient = recipient_hint(ctx, n).await;

    match n.kind {
        // Transporter job offers go out-of-band only as a high-priority FCM data
        // message; the on-duty AppSync Events channel is the in-app path.
        NotificationKind::JobOffer => {
            deliver_job_offer_push(ctx, n).await;
        }
        // Receipts and invoices: SES email plus a WhatsApp document template.
        NotificationKind::Receipt => {
            deliver_email(ctx, n, recipient.as_deref()).await;
            deliver_whatsapp(ctx, n, recipient.as_deref()).await;
        }
        // Everything else: a WhatsApp utility template, with an SMS fallback for
        // payment-critical kinds.
        kind => {
            deliver_whatsapp(ctx, n, recipient.as_deref()).await;
            if is_payment_critical(kind) {
                deliver_sms(ctx, n, recipient.as_deref()).await;
            }
        }
    }

    // Delivery audit line (best-effort; a failed audit must not fail delivery).
    if let Err(e) = ctx.db.audit(n.user_id.as_str(), "system", "NOTIFY_DELIVERED", serde_json::json!({ "kind": n.kind, "id": n.id })).await
    {
        tracing::warn!(notification = %n.id, error = %e, "failed to record NOTIFY_DELIVERED audit");
    }

    Ok(())
}

/// Resolves a best-effort recipient string (`phone_last4`) for the messengers.
///
/// Returns `None` when the user is missing or has no last-four on file; the
/// phone/SMS channels then log and skip. A repository error is logged (not
/// propagated) so the FCM/email paths can still run.
async fn recipient_hint(ctx: &WorkerCtx, n: &Notification) -> Option<String> {
    match ctx.db.user(&n.user_id).await {
        Ok(Some(rec)) => {
            let UserProfile { phone_last4, .. } = &rec.data;
            if phone_last4.is_empty() { None } else { Some(phone_last4.clone()) }
        }
        Ok(None) => {
            tracing::warn!(notification = %n.id, user = %n.user_id, "recipient profile not found");
            None
        }
        Err(e) => {
            tracing::warn!(notification = %n.id, user = %n.user_id, error = %e, "failed to load recipient profile");
            None
        }
    }
}

/// Sends the WhatsApp utility (or, for receipts, document) template.
async fn deliver_whatsapp(ctx: &WorkerCtx, n: &Notification, recipient: Option<&str>) {
    let Some(to) = recipient else {
        tracing::warn!(notification = %n.id, kind = ?n.kind, "no recipient hint; skipping WhatsApp");
        return;
    };
    let template = whatsapp_template(n.kind);
    // Positional template params: title, body, and the deep link if present.
    let params = vec![n.title.clone(), n.body.clone(), n.link.clone().unwrap_or_default()];
    match ctx.ix.whatsapp.send_template(to, template, &params).await {
        Ok(id) => tracing::info!(notification = %n.id, template, message = %id, "WhatsApp dispatched"),
        Err(e) => tracing::warn!(notification = %n.id, template, error = %e, "WhatsApp delivery failed"),
    }
}

/// Sends the SMS fallback for payment-critical kinds (DLT-registered template).
async fn deliver_sms(ctx: &WorkerCtx, n: &Notification, recipient: Option<&str>) {
    let Some(to) = recipient else {
        tracing::warn!(notification = %n.id, kind = ?n.kind, "no recipient hint; skipping SMS");
        return;
    };
    let template_key = sms_template(n.kind);
    let body = format!("{}: {}", n.title, n.body);
    match ctx.ix.sms.send(to, template_key, &body).await {
        Ok(id) => tracing::info!(notification = %n.id, template = template_key, message = %id, "SMS dispatched"),
        Err(e) => tracing::warn!(notification = %n.id, template = template_key, error = %e, "SMS delivery failed"),
    }
}

/// Sends a receipt as an HTML email via SES (best-effort recipient).
async fn deliver_email(ctx: &WorkerCtx, n: &Notification, recipient: Option<&str>) {
    // No email address is stored on the profile; fall back to the phone hint so
    // the mock inbox still captures it. The live SES adapter resolves the real
    // address from the account before sending.
    let to = recipient.unwrap_or("");
    let html = receipt_html(n);
    match ctx.ix.email.send(to, &n.title, &html).await {
        Ok(id) => tracing::info!(notification = %n.id, message = %id, "receipt email dispatched"),
        Err(e) => tracing::warn!(notification = %n.id, error = %e, "receipt email delivery failed"),
    }
}

/// Fans a job offer out to the transporter's registered devices as a
/// high-priority FCM data message (SNS platform endpoints).
async fn deliver_job_offer_push(ctx: &WorkerCtx, n: &Notification) {
    let devices = match ctx.db.devices(&n.user_id).await {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(notification = %n.id, user = %n.user_id, error = %e, "failed to load devices for job offer push");
            return;
        }
    };
    if devices.is_empty() {
        tracing::warn!(notification = %n.id, user = %n.user_id, "transporter has no registered devices for job offer push");
        return;
    }

    // Data-only payload so the Android client can raise a full-screen intent.
    let payload = serde_json::json!({
        "kind": n.kind,
        "notificationId": n.id,
        "title": n.title,
        "body": n.body,
        "link": n.link,
        "priority": "high",
        "data": n.data,
    });

    for rec in &devices {
        let Device { push_endpoint_arn, device_id, .. } = &rec.data;
        let Some(arn) = push_endpoint_arn.as_deref() else {
            tracing::debug!(notification = %n.id, device = %device_id, "device has no push endpoint; skipping");
            continue;
        };
        match ctx.ix.push.send(arn, &payload, JOB_OFFER_PUSH_TTL).await {
            Ok(()) => tracing::info!(notification = %n.id, device = %device_id, "job offer push dispatched"),
            Err(e) => tracing::warn!(notification = %n.id, device = %device_id, error = %e, "job offer push failed"),
        }
    }
}

/// Whether a kind is payment-critical and so earns the SMS fallback (§19.1:
/// "Weights to confirm / payment received" → WhatsApp → SMS).
fn is_payment_critical(kind: NotificationKind) -> bool {
    matches!(kind, NotificationKind::WeightsToConfirm | NotificationKind::PaymentRequested | NotificationKind::PaymentReceived)
}

/// Approved WhatsApp template name for a kind (versioned per PLAN.md §19.2).
fn whatsapp_template(kind: NotificationKind) -> &'static str {
    use NotificationKind::*;
    match kind {
        AnalysisDone => "analysis_done_v1",
        ProofRequested => "proof_requested_v1",
        NewMatch => "new_match_v1",
        NewOffer => "new_offer_v1",
        CounterOffer => "counter_offer_v1",
        OfferAccepted => "offer_accepted_v1",
        JobOffer => "job_offer_v1",
        JobAccepted => "job_accepted_v1",
        PickupEta => "pickup_eta_v1",
        Arrived => "arrived_v1",
        WeightsToConfirm => "weights_to_confirm_v1",
        PaymentRequested => "payment_requested_v1",
        PaymentReceived => "payment_received_v1",
        HandoverDone => "handover_done_v1",
        Receipt => "receipt_document_v1",
        Dispute => "dispute_v1",
        ChatMessage => "chat_message_v1",
        KycDecision => "kyc_decision_v1",
        DriveReminder => "drive_reminder_v1",
    }
}

/// DLT-registered SMS template key for a payment-critical kind.
fn sms_template(kind: NotificationKind) -> &'static str {
    use NotificationKind::*;
    match kind {
        WeightsToConfirm => "sms_weights_to_confirm_v1",
        PaymentRequested => "sms_payment_requested_v1",
        PaymentReceived => "sms_payment_received_v1",
        // Not expected (only payment-critical kinds reach SMS), but keep a safe
        // generic template rather than panicking on a future kind.
        _ => "sms_notification_v1",
    }
}

/// Minimal HTML body for a receipt email. The receipt payload travels in
/// `n.data`; this renders a human-readable summary with the deep link.
fn receipt_html(n: &Notification) -> String {
    let title = html_escape(&n.title);
    let body = html_escape(&n.body);
    let link = n.link.as_deref().map(|l| format!("<p><a href=\"{}\">View receipt</a></p>", html_escape(l))).unwrap_or_default();
    format!(
        "<!doctype html><html><body style=\"font-family:system-ui,sans-serif\">\
         <h2>{title}</h2><p>{body}</p>{link}\
         <hr><p style=\"color:#666;font-size:12px\">This is a KabadOS platform record, not an EPR certificate.</p>\
         </body></html>"
    )
}

/// Escapes the five HTML-significant characters for safe interpolation.
fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}
