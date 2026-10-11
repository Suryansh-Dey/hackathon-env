//! In-app audio calls via the Amazon Chime SDK (PLAN.md §19.3). Responses use
//! the Chime JSON field names the web and Android SDKs expect.

use async_trait::async_trait;
use serde_json::{Value, json};

#[async_trait]
pub trait CallsPort: Send + Sync {
    /// Creates a meeting (media in the configured region) and returns `{Meeting: {...}}`.
    async fn create_meeting(&self, external_id: &str) -> anyhow::Result<Value>;
    /// Adds an attendee and returns `{Attendee: {...}}`.
    async fn create_attendee(&self, meeting_id: &str, external_user_id: &str) -> anyhow::Result<Value>;
    async fn end_meeting(&self, meeting_id: &str) -> anyhow::Result<()>;
}

pub struct ChimeCalls {
    client: aws_sdk_chimesdkmeetings::Client,
    media_region: String,
}

impl ChimeCalls {
    pub fn new(client: aws_sdk_chimesdkmeetings::Client, media_region: String) -> Self {
        Self { client, media_region }
    }
}

#[async_trait]
impl CallsPort for ChimeCalls {
    async fn create_meeting(&self, external_id: &str) -> anyhow::Result<Value> {
        let out = self
            .client
            .create_meeting()
            .client_request_token(kabados_domain::ids::CallId::new().to_string())
            .media_region(&self.media_region)
            .external_meeting_id(external_id)
            .send()
            .await?;
        let m = out.meeting().ok_or_else(|| anyhow::anyhow!("no meeting returned"))?;
        let mp = m.media_placement();
        Ok(json!({"Meeting": {
            "MeetingId": m.meeting_id(),
            "ExternalMeetingId": m.external_meeting_id(),
            "MediaRegion": m.media_region(),
            "MediaPlacement": {
                "AudioHostUrl": mp.and_then(|p| p.audio_host_url()),
                "AudioFallbackUrl": mp.and_then(|p| p.audio_fallback_url()),
                "SignalingUrl": mp.and_then(|p| p.signaling_url()),
                "TurnControlUrl": mp.and_then(|p| p.turn_control_url()),
                "ScreenDataUrl": mp.and_then(|p| p.screen_data_url()),
                "ScreenViewingUrl": mp.and_then(|p| p.screen_viewing_url()),
                "ScreenSharingUrl": mp.and_then(|p| p.screen_sharing_url()),
                "EventIngestionUrl": mp.and_then(|p| p.event_ingestion_url()),
            }
        }}))
    }

    async fn create_attendee(&self, meeting_id: &str, external_user_id: &str) -> anyhow::Result<Value> {
        let out = self.client.create_attendee().meeting_id(meeting_id).external_user_id(external_user_id).send().await?;
        let a = out.attendee().ok_or_else(|| anyhow::anyhow!("no attendee returned"))?;
        Ok(json!({"Attendee": {"AttendeeId": a.attendee_id(), "ExternalUserId": a.external_user_id(), "JoinToken": a.join_token()}}))
    }

    async fn end_meeting(&self, meeting_id: &str) -> anyhow::Result<()> {
        self.client.delete_meeting().meeting_id(meeting_id).send().await?;
        Ok(())
    }
}

/// Local fake: well-formed objects so the flow (authorization, notifications) runs offline.
pub struct FakeCalls;

#[async_trait]
impl CallsPort for FakeCalls {
    async fn create_meeting(&self, external_id: &str) -> anyhow::Result<Value> {
        Ok(
            json!({"Meeting": {"MeetingId": format!("local-{external_id}"), "ExternalMeetingId": external_id, "MediaRegion": "local", "MediaPlacement": {}}}),
        )
    }

    async fn create_attendee(&self, meeting_id: &str, user: &str) -> anyhow::Result<Value> {
        Ok(json!({"Attendee": {"AttendeeId": format!("{meeting_id}-{user}"), "ExternalUserId": user, "JoinToken": "local"}}))
    }

    async fn end_meeting(&self, _meeting_id: &str) -> anyhow::Result<()> {
        Ok(())
    }
}
