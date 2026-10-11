//! Structured logging (PLAN.md §22.2): JSON logs on Lambda, compact locally.

use tracing_subscriber::EnvFilter;

pub fn init() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,aws_config=warn,aws_smithy_runtime=warn"));
    let on_lambda = std::env::var("AWS_LAMBDA_FUNCTION_NAME").is_ok();
    let res = if on_lambda {
        tracing_subscriber::fmt().json().with_env_filter(filter).with_current_span(false).without_time().try_init()
    } else {
        tracing_subscriber::fmt().compact().with_env_filter(filter).try_init()
    };
    let _ = res;
}
