//! Integration tests against DynamoDB Local (`DYNAMODB_ENDPOINT`, e.g. http://localhost:8000).
//! Run with `cargo test -p kabados-db -- --ignored`.

use kabados_db::dynamo::DynamoStore;
use kabados_db::{TableNames, conformance, schema};

async fn store() -> Option<DynamoStore> {
    let endpoint = std::env::var("DYNAMODB_ENDPOINT").ok()?;
    let creds = aws_sdk_dynamodb::config::Credentials::new("local", "local", None, None, "static");
    let conf = aws_sdk_dynamodb::Config::builder()
        .behavior_version(aws_sdk_dynamodb::config::BehaviorVersion::latest())
        .region(aws_sdk_dynamodb::config::Region::new("ap-south-1"))
        .credentials_provider(creds)
        .endpoint_url(endpoint)
        .build();
    let client = aws_sdk_dynamodb::Client::from_conf(conf);
    let names = TableNames::with_prefix("kabados-it");
    schema::ensure_tables(&client, &names).await.expect("create tables");
    Some(DynamoStore::new(client, names))
}

#[tokio::test]
#[ignore = "needs DynamoDB Local (DYNAMODB_ENDPOINT)"]
async fn dynamodb_conforms() {
    let Some(s) = store().await else {
        eprintln!("DYNAMODB_ENDPOINT not set; skipping");
        return;
    };
    let ns = format!("it-{}", chrono::Utc::now().timestamp_millis());
    conformance::run(&s, &ns).await;
}
