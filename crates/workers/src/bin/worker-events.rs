//! EventBridge consumer: matching and coverage index.

use kabados_domain::events::EventEnvelope;
use kabados_workers::{WorkerCtx, coverage, matching};
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
            let detail = e.payload.get("detail").cloned().unwrap_or(Value::Null);
            let env: EventEnvelope = serde_json::from_value(detail).map_err(|e| Error::from(format!("bad event detail: {e}")))?;
            matching::handle_event(&ctx, &env).await.map_err(|e| Error::from(e.to_string()))?;
            coverage::handle_event(&ctx, &env).await.map_err(|e| Error::from(e.to_string()))?;
            Ok::<_, Error>(())
        }
    }))
    .await
}
