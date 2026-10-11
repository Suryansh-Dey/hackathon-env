//! Rate Index worker: per-deal settlement aggregation and the weekly pass.

use kabados_domain::events::{DomainEvent, EventEnvelope};
use kabados_workers::{WorkerCtx, rate_index};
use lambda_runtime::{Error, LambdaEvent, service_fn};
use serde_json::Value;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Error> {
    kabados_runtime::telemetry::init();
    let ctx = Arc::new(WorkerCtx::from_env().await.map_err(|e| Error::from(e.to_string()))?);
    lambda_runtime::run(service_fn(move |e: LambdaEvent<Value>| {
        let ctx = ctx.clone();
        async move {
            // Scheduled weekly pass: {"job":"weekly"}.
            if e.payload.get("job").and_then(|v| v.as_str()) == Some("weekly") {
                let week = current_iso_week();
                let run = rate_index::run(&ctx, &week).await.map_err(|e| Error::from(e.to_string()))?;
                tracing::info!(?run, %week, "weekly rate index pass");
                return Ok::<_, Error>(());
            }
            // Otherwise a DealSettled EventBridge event.
            if let Some(detail) = e.payload.get("detail").cloned() {
                let env: EventEnvelope = serde_json::from_value(detail).map_err(|e| Error::from(format!("bad event detail: {e}")))?;
                if let DomainEvent::DealSettled { deal_id } = env.event {
                    rate_index::on_deal_settled(&ctx, &deal_id).await.map_err(|e| Error::from(e.to_string()))?;
                }
            }
            Ok::<_, Error>(())
        }
    }))
    .await
}

fn current_iso_week() -> String {
    let now = chrono::Utc::now();
    let w = chrono::Datelike::iso_week(&now);
    format!("{}-W{:02}", w.year(), w.week())
}
