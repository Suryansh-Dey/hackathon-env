//! Integrations (PLAN.md §15.4). Providers that need an external approval sit
//! behind ports with `mock` / `sandbox` / `live` adapters; AWS services that
//! need no approval are wrapped thinly with local fakes for development.

pub mod blob;
pub mod calls;
pub mod events;
pub mod integrity;
pub mod kyc;
pub mod messaging;
pub mod payments;
pub mod places;
pub mod push;
pub mod realtime;
pub mod routes;
pub mod vision;

use std::sync::Arc;

/// Every port the API and workers use, chosen from configuration.
#[derive(Clone)]
pub struct Integrations {
    pub whatsapp: Arc<dyn messaging::WhatsAppPort>,
    pub sms: Arc<dyn messaging::SmsPort>,
    pub email: Arc<dyn messaging::EmailPort>,
    pub payments: Arc<dyn payments::PaymentsPort>,
    pub route: Arc<dyn payments::RoutePort>,
    pub payouts: Arc<dyn payments::PayoutsPort>,
    pub kyc: Arc<dyn kyc::KycPort>,
    pub integrity: Arc<dyn integrity::IntegrityPort>,
    pub places: Arc<dyn places::PlacesPort>,
    pub routes: Arc<dyn routes::RoutesPort>,
    pub tracker: Arc<dyn routes::TrackerPort>,
    pub vision: Arc<dyn vision::VisionPort>,
    pub doc_text: Arc<dyn vision::DocTextPort>,
    pub calls: Arc<dyn calls::CallsPort>,
    pub push: Arc<dyn push::PushPort>,
    pub realtime: Arc<dyn realtime::RealtimePort>,
    pub blobs: Arc<dyn blob::BlobPort>,
    pub events: Arc<dyn events::EventBusPort>,
}

impl Integrations {
    /// All-local adapters (unit tests and the in-memory dev server).
    pub fn local(db: kabados_db::repo::Db, env: &str, blob_root: std::path::PathBuf, public_base_url: &str) -> Self {
        let inbox = Arc::new(messaging::InboxMessenger::new(db.clone(), env));
        Self {
            whatsapp: inbox.clone(),
            sms: inbox.clone(),
            email: inbox,
            payments: Arc::new(payments::MockRazorpay::new("mock_webhook_secret")),
            route: Arc::new(payments::MockRoute::default()),
            payouts: Arc::new(payments::MockPayouts::default()),
            kyc: Arc::new(kyc::SkipKyc),
            integrity: Arc::new(integrity::MockIntegrity::default()),
            places: Arc::new(places::FakePlaces),
            routes: Arc::new(routes::FakeRoutes),
            tracker: Arc::new(routes::FakeTracker::default()),
            vision: Arc::new(vision::FakeVision),
            doc_text: Arc::new(vision::FakeDocText),
            calls: Arc::new(calls::FakeCalls),
            push: Arc::new(push::FakePush::default()),
            realtime: Arc::new(realtime::LocalHub::new()),
            blobs: Arc::new(blob::LocalBlobs::new(blob_root, public_base_url)),
            events: Arc::new(events::MemoryBus::default()),
        }
    }

    /// Adapters for a deployed environment, chosen by provider modes.
    pub async fn deployed(
        cfg: &kabados_runtime::Config,
        sdk: &aws_config::SdkConfig,
        db: kabados_db::repo::Db,
        secrets: &kabados_runtime::secrets::Secrets,
    ) -> anyhow::Result<Self> {
        use kabados_runtime::Mode;
        let inbox = Arc::new(messaging::InboxMessenger::new(db.clone(), &cfg.env));
        let whatsapp: Arc<dyn messaging::WhatsAppPort> = match cfg.modes.whatsapp {
            Mode::Mock => inbox.clone(),
            _ => Arc::new(messaging::AwsWhatsApp::new(
                aws_sdk_socialmessaging::Client::new(sdk),
                secrets.get("whatsapp/phone_number_id", "WHATSAPP_PHONE_NUMBER_ID").await?,
            )),
        };
        let sms: Arc<dyn messaging::SmsPort> = match cfg.modes.sms {
            Mode::Mock => inbox.clone(),
            _ => {
                let dlt: serde_json::Value = serde_json::from_str(&secrets.get("sms/dlt", "SMS_DLT_JSON").await?)?;
                Arc::new(messaging::AwsSms::new(aws_sdk_pinpointsmsvoicev2::Client::new(sdk), dlt))
            }
        };
        let email: Arc<dyn messaging::EmailPort> = match std::env::var("SES_FROM").ok() {
            Some(from) if cfg.is_prod() || std::env::var("SES_ENABLED").is_ok() => {
                Arc::new(messaging::SesEmail::new(aws_sdk_sesv2::Client::new(sdk), from))
            }
            _ => inbox.clone(),
        };
        let payments: Arc<dyn payments::PaymentsPort> = match cfg.modes.payments {
            Mode::Mock => Arc::new(payments::MockRazorpay::new(
                &secrets
                    .try_get("razorpay/webhook_secret", "RAZORPAY_WEBHOOK_SECRET")
                    .await
                    .unwrap_or_else(|| "mock_webhook_secret".into()),
            )),
            _ => Arc::new(payments::Razorpay::new(
                secrets.get("razorpay/key_id", "RAZORPAY_KEY_ID").await?,
                secrets.get("razorpay/key_secret", "RAZORPAY_KEY_SECRET").await?,
                secrets.get("razorpay/webhook_secret", "RAZORPAY_WEBHOOK_SECRET").await?,
            )),
        };
        let places: Arc<dyn places::PlacesPort> = Arc::new(places::AwsPlaces::new(aws_sdk_geoplaces::Client::new(sdk)));
        let routes: Arc<dyn routes::RoutesPort> = Arc::new(routes::AwsRoutes::new(aws_sdk_georoutes::Client::new(sdk)));
        let tracker: Arc<dyn routes::TrackerPort> = match &cfg.tracker_name {
            Some(t) => Arc::new(routes::AwsTracker::new(aws_sdk_location::Client::new(sdk), t.clone())),
            None => Arc::new(routes::FakeTracker::default()),
        };
        let realtime: Arc<dyn realtime::RealtimePort> = match &cfg.appsync_http_domain {
            Some(d) => Arc::new(realtime::AppSyncEvents::new(d.clone(), sdk.clone())),
            None => Arc::new(realtime::LocalHub::new()),
        };
        let push: Arc<dyn push::PushPort> = match std::env::var("SNS_PLATFORM_APP_ARN").ok() {
            Some(arn) => Arc::new(push::SnsPush::new(aws_sdk_sns::Client::new(sdk), arn)),
            None => Arc::new(push::FakePush::default()),
        };
        let events: Arc<dyn events::EventBusPort> = match &cfg.event_bus {
            Some(bus) => Arc::new(events::EventBridgeBus::new(aws_sdk_eventbridge::Client::new(sdk), bus.clone())),
            None => Arc::new(events::MemoryBus::default()),
        };
        Ok(Self {
            whatsapp,
            sms,
            email,
            payments,
            route: Arc::new(payments::MockRoute::default()),
            payouts: Arc::new(payments::MockPayouts::default()),
            kyc: Arc::new(kyc::SkipKyc),
            integrity: Arc::new(integrity::MockIntegrity::default()),
            places,
            routes,
            tracker,
            vision: Arc::new(vision::AwsVision::new(aws_sdk_rekognition::Client::new(sdk))),
            doc_text: Arc::new(vision::AwsDocText::new(aws_sdk_textract::Client::new(sdk))),
            calls: Arc::new(calls::ChimeCalls::new(aws_sdk_chimesdkmeetings::Client::new(sdk), cfg.region.clone())),
            push,
            realtime,
            blobs: Arc::new(blob::S3Blobs::new(aws_sdk_s3::Client::new(sdk))),
            events,
        })
    }
}
