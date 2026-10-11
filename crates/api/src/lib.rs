//! KabadOS REST API (PLAN.md §17). Each bounded context exposes a router; a
//! Lambda binary per context serves its router behind API Gateway, and the
//! local dev server mounts them all.

pub mod access;
pub mod ai;
pub mod auth;
pub mod coverage;
pub mod error;
pub mod idem;
pub mod matching;
pub mod openapi;
pub mod routes;
pub mod settle;
pub mod state;
pub mod tasks;
pub mod views;

pub use error::{ApiError, ApiResult};
pub use state::AppState;

use axum::Router;

/// Every context's routes (local dev server, tests, OpenAPI).
pub fn app(state: AppState) -> Router {
    Router::new()
        .merge(routes::identity::router())
        .merge(routes::listings::router())
        .merge(routes::offers::router())
        .merge(routes::vendor::router())
        .merge(routes::logistics::router())
        .merge(routes::deals::router())
        .merge(routes::payments::router())
        .merge(routes::chat::router())
        .merge(routes::calls::router())
        .merge(routes::drives::router())
        .merge(routes::public::router())
        .merge(routes::ops::router())
        .merge(routes::dev::router())
        .layer(axum::middleware::from_fn_with_state(state.clone(), idem::idempotency))
        .layer(tower_http::cors::CorsLayer::very_permissive())
        .with_state(state)
}

/// Serves one context router on Lambda.
pub async fn run_lambda(router: Router<AppState>) -> Result<(), lambda_http::Error> {
    kabados_runtime::telemetry::init();
    let state = AppState::from_env().await.map_err(|e| lambda_http::Error::from(e.to_string()))?;
    let app = router.layer(axum::middleware::from_fn_with_state(state.clone(), idem::idempotency)).with_state(state);
    lambda_http::run(app).await
}
