//! Asynchronous work handed to workers (SQS in AWS, in-process locally).

use async_trait::async_trait;
use kabados_domain::ids::*;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "task", rename_all = "snake_case")]
pub enum Task {
    /// Image uploaded: strip EXIF, blur faces, thumbnail, pHash, triage (PLAN.md §11.3).
    ProcessImage { listing_id: ListingId, ver: u32, image_id: ImageId },
    /// Itemize then verify claims (PLAN.md §11.4–11.5).
    AnalyzeListing { listing_id: ListingId, ver: u32 },
    /// Re-run claim verification after proof photos (PLAN.md §11.5).
    VerifyListing { listing_id: ListingId, ver: u32 },
    /// Process a KYC document upload (Textract extraction).
    ProcessDocument { user_id: UserId, document_id: DocumentId },
}

impl Task {
    /// Queue the task belongs to.
    pub fn queue(&self) -> QueueKind {
        match self {
            Task::ProcessImage { .. } | Task::ProcessDocument { .. } => QueueKind::Media,
            Task::AnalyzeListing { .. } | Task::VerifyListing { .. } => QueueKind::Analysis,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum QueueKind {
    Media,
    Analysis,
}

#[async_trait]
pub trait TaskQueue: Send + Sync {
    async fn enqueue(&self, task: Task) -> anyhow::Result<()>;
}

pub struct SqsTaskQueue {
    client: aws_sdk_sqs::Client,
    media_url: Option<String>,
    analysis_url: Option<String>,
}

impl SqsTaskQueue {
    pub fn from_env(client: aws_sdk_sqs::Client) -> Self {
        Self { client, media_url: std::env::var("QUEUE_MEDIA_URL").ok(), analysis_url: std::env::var("QUEUE_ANALYSIS_URL").ok() }
    }
}

#[async_trait]
impl TaskQueue for SqsTaskQueue {
    async fn enqueue(&self, task: Task) -> anyhow::Result<()> {
        let url = match task.queue() {
            QueueKind::Media => self.media_url.as_ref(),
            QueueKind::Analysis => self.analysis_url.as_ref(),
        }
        .ok_or_else(|| anyhow::anyhow!("queue URL for {:?} not configured", task.queue()))?;
        self.client.send_message().queue_url(url).message_body(serde_json::to_string(&task)?).send().await?;
        Ok(())
    }
}

/// Records tasks (tests; the local server drains it into workers).
#[derive(Default)]
pub struct RecordingQueue {
    pub tasks: Mutex<Vec<Task>>,
}

#[async_trait]
impl TaskQueue for RecordingQueue {
    async fn enqueue(&self, task: Task) -> anyhow::Result<()> {
        self.tasks.lock().unwrap().push(task);
        Ok(())
    }
}
