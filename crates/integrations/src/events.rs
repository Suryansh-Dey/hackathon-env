//! Domain event publishing to Amazon EventBridge (PLAN.md §17.3).

use async_trait::async_trait;
use kabados_domain::events::{EVENT_SOURCE, EventEnvelope};
use std::sync::Mutex;

#[async_trait]
pub trait EventBusPort: Send + Sync {
    async fn publish(&self, events: &[EventEnvelope]) -> anyhow::Result<()>;
}

pub struct EventBridgeBus {
    client: aws_sdk_eventbridge::Client,
    bus: String,
}

impl EventBridgeBus {
    pub fn new(client: aws_sdk_eventbridge::Client, bus: String) -> Self {
        Self { client, bus }
    }
}

#[async_trait]
impl EventBusPort for EventBridgeBus {
    async fn publish(&self, events: &[EventEnvelope]) -> anyhow::Result<()> {
        use aws_sdk_eventbridge::types::PutEventsRequestEntry;
        for chunk in events.chunks(10) {
            let entries = chunk
                .iter()
                .map(|e| {
                    Ok(PutEventsRequestEntry::builder()
                        .event_bus_name(&self.bus)
                        .source(EVENT_SOURCE)
                        .detail_type(e.event.detail_type())
                        .detail(serde_json::to_string(e)?)
                        .build())
                })
                .collect::<anyhow::Result<Vec<_>>>()?;
            let out = self.client.put_events().set_entries(Some(entries)).send().await?;
            if out.failed_entry_count() > 0 {
                anyhow::bail!("{} events failed to publish", out.failed_entry_count());
            }
        }
        Ok(())
    }
}

/// Local bus: records events (the local dispatcher consumes the store's change feed directly).
#[derive(Default)]
pub struct MemoryBus {
    pub published: Mutex<Vec<EventEnvelope>>,
}

#[async_trait]
impl EventBusPort for MemoryBus {
    async fn publish(&self, events: &[EventEnvelope]) -> anyhow::Result<()> {
        self.published.lock().unwrap().extend_from_slice(events);
        Ok(())
    }
}
