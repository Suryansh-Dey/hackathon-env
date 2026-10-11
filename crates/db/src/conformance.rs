//! Backend conformance suite: the same assertions run against the memory
//! store (unit tests) and DynamoDB Local (integration tests), so the
//! emulation cannot drift from DynamoDB (PLAN.md §22.4 "Contract").

use crate::store::*;
use serde_json::{Value, json};

fn item(pk: &str, sk: &str, extra: Value) -> Item {
    let mut m = extra.as_object().cloned().unwrap_or_default();
    m.insert("PK".into(), pk.into());
    m.insert("SK".into(), sk.into());
    m
}

/// Runs every check; `ns` namespaces keys so runs don't collide.
pub async fn run(store: &dyn Store, ns: &str) {
    let pk = |s: &str| format!("{ns}#{s}");

    // put/get and NotExists uniqueness
    store.put(Table::Core, item(&pk("A"), "META", json!({"version": 1, "data": {"n": 1}})), Some(Cond::NotExists)).await.unwrap();
    let got = store.get(Table::Core, &pk("A"), "META").await.unwrap().unwrap();
    assert_eq!(got["data"]["n"], 1);
    assert!(store.put(Table::Core, item(&pk("A"), "META", json!({})), Some(Cond::NotExists)).await.unwrap_err().is_conflict());

    // optimistic locking
    store.put(Table::Core, item(&pk("A"), "META", json!({"version": 2})), Some(Cond::version(1))).await.unwrap();
    assert!(store.put(Table::Core, item(&pk("A"), "META", json!({"version": 3})), Some(Cond::version(1))).await.unwrap_err().is_conflict());

    // Ne / In
    store.put(Table::Core, item(&pk("S"), "META", json!({"status": "PUBLISHED"})), None).await.unwrap();
    store
        .put(
            Table::Core,
            item(&pk("S"), "META", json!({"status": "RESERVED"})),
            Some(Cond::In("status".into(), vec![json!("PUBLISHED"), json!("READY")])),
        )
        .await
        .unwrap();
    assert!(
        store
            .put(Table::Core, item(&pk("S"), "META", json!({})), Some(Cond::Ne("status".into(), json!("RESERVED"))))
            .await
            .unwrap_err()
            .is_conflict()
    );

    // transactions are all-or-nothing
    let err = store
        .transact(vec![
            TxOp::Put { table: Table::Core, item: item(&pk("T"), "1", json!({})), cond: Some(Cond::NotExists) },
            TxOp::Check { table: Table::Core, pk: pk("A"), sk: "META".into(), cond: Cond::version(99) },
        ])
        .await
        .unwrap_err();
    assert!(matches!(err, DbError::TransactionCancelled(_)), "{err:?}");
    assert!(store.get(Table::Core, &pk("T"), "1").await.unwrap().is_none());
    store
        .transact(vec![
            TxOp::Put { table: Table::Core, item: item(&pk("T"), "1", json!({})), cond: Some(Cond::NotExists) },
            TxOp::Put { table: Table::Ledger, item: item(&pk("T"), "2", json!({})), cond: None },
            TxOp::Check { table: Table::Core, pk: pk("A"), sk: "META".into(), cond: Cond::version(2) },
        ])
        .await
        .unwrap();
    assert!(store.get(Table::Ledger, &pk("T"), "2").await.unwrap().is_some());

    // queries: begins_with, ordering, pagination, reverse
    for i in 0..7 {
        store
            .put(Table::Core, item(&pk("Q"), &format!("V#{i:02}"), json!({"GSI2PK": pk("G"), "GSI2SK": format!("K#{:02}", 10 - i)})), None)
            .await
            .unwrap();
    }
    store.put(Table::Core, item(&pk("Q"), "W#1", json!({})), None).await.unwrap();
    let first = store.query(Query::new(Table::Core, pk("Q")).begins_with("V#").limit(3)).await.unwrap();
    assert_eq!(first.items.len(), 3);
    assert_eq!(first.items[0]["SK"], "V#00");
    let all = store.query_all(Query::new(Table::Core, pk("Q")).begins_with("V#").limit(3)).await.unwrap();
    assert_eq!(all.len(), 7);
    let between = store.query(Query::new(Table::Core, pk("Q")).sk(SkCond::Between("V#02".into(), "V#04".into()))).await.unwrap();
    assert_eq!(between.items.len(), 3);
    let last = store.query(Query::new(Table::Core, pk("Q")).begins_with("V#").reverse().limit(1)).await.unwrap();
    assert_eq!(last.items[0]["SK"], "V#06");

    // GSI query sorted by index sort key
    let g = store.query_all(Query::new(Table::Core, pk("G")).index(Index::Gsi2)).await.unwrap();
    assert_eq!(g.len(), 7);
    assert_eq!(g[0]["SK"], "V#06");

    // batch operations
    let keys: Vec<(String, String)> = (0..7).map(|i| (pk("Q"), format!("V#{i:02}"))).collect();
    assert_eq!(store.batch_get(Table::Core, &keys).await.unwrap().len(), 7);
    let puts: Vec<Item> = (0..30).map(|i| item(&pk("B"), &format!("{i:03}"), json!({}))).collect();
    store.batch_write(Table::Core, puts, vec![]).await.unwrap();
    assert_eq!(store.query_all(Query::new(Table::Core, pk("B"))).await.unwrap().len(), 30);
    let dels: Vec<(String, String)> = (0..30).map(|i| (pk("B"), format!("{i:03}"))).collect();
    store.batch_write(Table::Core, vec![], dels).await.unwrap();
    assert!(store.query_all(Query::new(Table::Core, pk("B"))).await.unwrap().is_empty());

    // conditional delete
    assert!(store.delete(Table::Core, &pk("A"), "META", Some(Cond::version(1))).await.unwrap_err().is_conflict());
    store.delete(Table::Core, &pk("A"), "META", Some(Cond::version(2))).await.unwrap();
    assert!(store.get(Table::Core, &pk("A"), "META").await.unwrap().is_none());
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn memory_store_conforms() {
        super::run(&crate::memory::MemoryStore::new(), "mem").await;
    }
}
