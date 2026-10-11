//! DynamoDB Streams consumer: outbox → EventBridge + notification dispatch.

use aws_lambda_events::event::dynamodb::Event;
use kabados_workers::{StreamItem, WorkerCtx, outbox};
use lambda_runtime::{Error, LambdaEvent, service_fn};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Error> {
    kabados_runtime::telemetry::init();
    let ctx = Arc::new(WorkerCtx::from_env().await.map_err(|e| Error::from(e.to_string()))?);
    lambda_runtime::run(service_fn(move |e: LambdaEvent<Event>| {
        let ctx = ctx.clone();
        async move {
            let items: Vec<StreamItem> = e
                .payload
                .records
                .into_iter()
                .map(|r| StreamItem {
                    event_name: r.event_name,
                    new_image: to_item(r.change.new_image),
                    old_image: to_item(r.change.old_image),
                })
                .collect();
            outbox::handle_records(&ctx, &items).await.map_err(|e| Error::from(e.to_string()))
        }
    }))
    .await
}

/// DynamoDB stream image → our JSON item (empty image → None).
fn to_item(image: serde_dynamo::Item) -> Option<kabados_db::store::Item> {
    if image.is_empty() {
        return None;
    }
    match serde_dynamo::from_item::<_, serde_json::Value>(image) {
        Ok(serde_json::Value::Object(m)) => Some(m),
        _ => None,
    }
}
