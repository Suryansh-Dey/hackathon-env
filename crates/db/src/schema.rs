//! Table definitions for local development and integration tests.
//! Deployed tables are defined in CDK (`infra/lib/data-stack.ts`) with the same shape.

use crate::store::{Table, TableNames};
use aws_sdk_dynamodb::Client;
use aws_sdk_dynamodb::types::{
    AttributeDefinition, BillingMode, GlobalSecondaryIndex, KeySchemaElement, KeyType, Projection, ProjectionType, ScalarAttributeType,
};

fn attr(name: &str) -> AttributeDefinition {
    AttributeDefinition::builder().attribute_name(name).attribute_type(ScalarAttributeType::S).build().expect("attr")
}

fn key(name: &str, t: KeyType) -> KeySchemaElement {
    KeySchemaElement::builder().attribute_name(name).key_type(t).build().expect("key")
}

/// Creates any missing tables (idempotent).
pub async fn ensure_tables(client: &Client, names: &TableNames) -> Result<(), String> {
    let existing = client.list_tables().send().await.map_err(|e| e.to_string())?.table_names().to_vec();
    for t in Table::ALL {
        let name = names.name(t);
        if existing.iter().any(|n| n == name) {
            continue;
        }
        let mut req = client
            .create_table()
            .table_name(name)
            .billing_mode(BillingMode::PayPerRequest)
            .attribute_definitions(attr("PK"))
            .attribute_definitions(attr("SK"))
            .key_schema(key("PK", KeyType::Hash))
            .key_schema(key("SK", KeyType::Range));
        if t.has_gsis() {
            for n in 1..=3 {
                let (pk, sk) = (format!("GSI{n}PK"), format!("GSI{n}SK"));
                req = req.attribute_definitions(attr(&pk)).attribute_definitions(attr(&sk)).global_secondary_indexes(
                    GlobalSecondaryIndex::builder()
                        .index_name(format!("GSI{n}"))
                        .key_schema(key(&pk, KeyType::Hash))
                        .key_schema(key(&sk, KeyType::Range))
                        .projection(Projection::builder().projection_type(ProjectionType::All).build())
                        .build()
                        .map_err(|e| e.to_string())?,
                );
            }
        }
        req.send().await.map_err(|e| format!("create {name}: {}", e.into_service_error()))?;
    }
    Ok(())
}
