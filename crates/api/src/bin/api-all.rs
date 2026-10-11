//! Combined API Lambda: serves every bounded-context router behind one
//! API Gateway `$default` route (approved packaging; code stays split).

#[tokio::main]
async fn main() -> Result<(), lambda_http::Error> {
    kabados_api::run_lambda_combined().await
}
