//! Lambda for the `deals` bounded context (PLAN.md §15.1).

#[tokio::main]
async fn main() -> Result<(), lambda_http::Error> {
    kabados_api::run_lambda(kabados_api::routes::deals::router()).await
}
