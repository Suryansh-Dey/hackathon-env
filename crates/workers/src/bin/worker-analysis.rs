//! SQS consumer for media and analysis tasks.

use aws_lambda_events::event::sqs::{SqsBatchResponse, SqsEvent};
use aws_lambda_events::sqs::BatchItemFailure;
use kabados_workers::{WorkerCtx, analysis, ctx::parse_task};
use lambda_runtime::{Error, LambdaEvent, service_fn};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Error> {
    kabados_runtime::telemetry::init();
    let ctx = Arc::new(WorkerCtx::from_env().await.map_err(|e| Error::from(e.to_string()))?);
    lambda_runtime::run(service_fn(move |e: LambdaEvent<SqsEvent>| {
        let ctx = ctx.clone();
        async move {
            let mut failures = Vec::new();
            for msg in e.payload.records {
                let id = msg.message_id.clone().unwrap_or_default();
                let body = msg.body.unwrap_or_default();
                match parse_task(&body) {
                    Ok(task) => {
                        if let Err(err) = analysis::handle_task(&ctx, task).await {
                            tracing::error!(error = %err, %id, "task failed");
                            let mut f = BatchItemFailure::default();
                            f.item_identifier = id;
                            failures.push(f);
                        }
                    }
                    Err(err) => tracing::error!(error = %err, %id, "undeserializable task; dropping"),
                }
            }
            let mut resp = SqsBatchResponse::default();
            resp.batch_item_failures = failures;
            Ok::<_, Error>(resp)
        }
    }))
    .await
}
