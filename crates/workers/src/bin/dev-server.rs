//! Local all-in-one dev server: the full HTTP API plus in-process workers,
//! backed by the in-memory store. One command runs the whole system with no
//! AWS — tasks are drained locally and domain events are dispatched to the
//! matching, coverage, rate-index and notification workers.
//!
//! Run: `cargo run -p kabados-workers --bin dev-server`  (listens on :8080)

use async_trait::async_trait;
use kabados_api::AppState;
use kabados_api::tasks::{Task, TaskQueue};
use kabados_db::memory::MemoryStore;
use kabados_db::repo::Db;
use kabados_domain::events::{DomainEvent, EventEnvelope};
use kabados_workers::{StreamItem, WorkerCtx, analysis, coverage, matching, outbox, rate_index};
use std::sync::Arc;
use tokio::sync::mpsc;

/// Task queue that hands work to an in-process channel.
struct LocalQueue(mpsc::UnboundedSender<Task>);

#[async_trait]
impl TaskQueue for LocalQueue {
    async fn enqueue(&self, task: Task) -> anyhow::Result<()> {
        let _ = self.0.send(task);
        Ok(())
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // SAFETY: set before anything reads config; single-threaded at startup.
    unsafe {
        if std::env::var("KABADOS_ENV").is_err() {
            std::env::set_var("KABADOS_ENV", "local");
        }
        if std::env::var("AI_MODE").is_err() {
            std::env::set_var("AI_MODE", "mock");
        }
    }
    kabados_runtime::telemetry::init();
    let cfg = kabados_runtime::Config::from_env();

    let store = Arc::new(MemoryStore::new());
    let db = Db::new(store.clone());
    let blob_root = std::env::temp_dir().join("kabados-dev-blobs");
    let ix = kabados_integrations::Integrations::local(db.clone(), &cfg.env, blob_root, &cfg.public_base_url);

    let (tx, mut rx) = mpsc::unbounded_channel::<Task>();
    let app_state = AppState::local(cfg, db.clone(), ix, Arc::new(LocalQueue(tx)))?;
    let worker = WorkerCtx::new(app_state.clone());

    // Drain the task queue into the analysis worker.
    {
        let worker = worker.clone();
        tokio::spawn(async move {
            while let Some(task) = rx.recv().await {
                if let Err(e) = analysis::handle_task(&worker, task).await {
                    tracing::error!(error = %e, "task failed");
                }
            }
        });
    }

    // Pump the store's change feed: outbox → events + notifications, and
    // dispatch each domain event to the matching/coverage/rate-index workers.
    {
        let worker = worker.clone();
        let mut changes = store.subscribe();
        tokio::spawn(async move {
            while let Ok(change) = changes.recv().await {
                let Some(new) = change.new.clone() else { continue };
                let ty = new.get("type").and_then(|v| v.as_str()).unwrap_or("");
                match ty {
                    "outbox" | "notify" => {
                        let item = StreamItem { event_name: "INSERT".into(), new_image: Some(new.clone()), old_image: None };
                        if let Err(e) = outbox::handle_records(&worker, &[item]).await {
                            tracing::warn!(error = %e, "outbox pump");
                        }
                        if ty == "outbox"
                            && let Some(env) = new.get("data").cloned().and_then(|d| serde_json::from_value::<EventEnvelope>(d).ok())
                        {
                            dispatch_event(&worker, &env).await;
                        }
                    }
                    _ => {}
                }
            }
        });
    }

    let router = kabados_api::app(app_state);
    let addr = std::env::var("BIND").unwrap_or_else(|_| "0.0.0.0:8080".into());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(%addr, "KabadOS dev server: API + in-process workers (in-memory store)");
    eprintln!("KabadOS dev server listening on http://{addr}  (POST /dev/seed to load demo vendors, POST /dev/token to sign in)");
    axum::serve(listener, router).await?;
    Ok(())
}

async fn dispatch_event(worker: &WorkerCtx, env: &EventEnvelope) {
    if let Err(e) = matching::handle_event(worker, env).await {
        tracing::warn!(error = %e, "matching");
    }
    if let Err(e) = coverage::handle_event(worker, env).await {
        tracing::warn!(error = %e, "coverage");
    }
    if let DomainEvent::DealSettled { deal_id } = &env.event
        && let Err(e) = rate_index::on_deal_settled(worker, deal_id).await
    {
        tracing::warn!(error = %e, "rate index");
    }
    // Notifications are delivered through the outbox `notify` path above.
}
