//! DynamoDB [`Store`] backend (aws-sdk-dynamodb).

use crate::store::*;
use async_trait::async_trait;
use aws_sdk_dynamodb::Client;
use aws_sdk_dynamodb::types::{
    AttributeValue, ConditionCheck, Delete, DeleteRequest, KeysAndAttributes, Put, PutRequest, TransactWriteItem, WriteRequest,
};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Map, Number, Value};
use std::collections::HashMap;

pub struct DynamoStore {
    client: Client,
    names: TableNames,
}

impl DynamoStore {
    pub fn new(client: Client, names: TableNames) -> Self {
        Self { client, names }
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    fn table(&self, t: Table) -> String {
        self.names.name(t).to_owned()
    }
}

/// JSON → DynamoDB attribute value.
pub fn to_attr(v: &Value) -> AttributeValue {
    match v {
        Value::Null => AttributeValue::Null(true),
        Value::Bool(b) => AttributeValue::Bool(*b),
        Value::Number(n) => AttributeValue::N(n.to_string()),
        Value::String(s) => AttributeValue::S(s.clone()),
        Value::Array(a) => AttributeValue::L(a.iter().map(to_attr).collect()),
        Value::Object(o) => AttributeValue::M(o.iter().map(|(k, v)| (k.clone(), to_attr(v))).collect()),
    }
}

/// DynamoDB attribute value → JSON.
pub fn from_attr(a: &AttributeValue) -> Value {
    match a {
        AttributeValue::Null(_) => Value::Null,
        AttributeValue::Bool(b) => Value::Bool(*b),
        AttributeValue::N(n) => n
            .parse::<i64>()
            .map(Value::from)
            .or_else(|_| n.parse::<u64>().map(Value::from))
            .ok()
            .or_else(|| n.parse::<f64>().ok().and_then(Number::from_f64).map(Value::Number))
            .unwrap_or(Value::Null),
        AttributeValue::S(s) => Value::String(s.clone()),
        AttributeValue::L(l) => Value::Array(l.iter().map(from_attr).collect()),
        AttributeValue::M(m) => Value::Object(m.iter().map(|(k, v)| (k.clone(), from_attr(v))).collect()),
        AttributeValue::Ss(v) => Value::Array(v.iter().cloned().map(Value::String).collect()),
        AttributeValue::Ns(v) => {
            Value::Array(v.iter().filter_map(|n| n.parse::<f64>().ok()).filter_map(Number::from_f64).map(Value::Number).collect())
        }
        _ => Value::Null,
    }
}

pub fn item_to_attrs(item: &Item) -> HashMap<String, AttributeValue> {
    item.iter().map(|(k, v)| (k.clone(), to_attr(v))).collect()
}

pub fn attrs_to_item(m: &HashMap<String, AttributeValue>) -> Item {
    m.iter().map(|(k, v)| (k.clone(), from_attr(v))).collect::<Map<_, _>>()
}

fn key(pk: &str, sk: &str) -> HashMap<String, AttributeValue> {
    HashMap::from([("PK".to_owned(), AttributeValue::S(pk.into())), ("SK".to_owned(), AttributeValue::S(sk.into()))])
}

#[derive(Default)]
struct Expr {
    names: HashMap<String, String>,
    values: HashMap<String, AttributeValue>,
    n: usize,
}

impl Expr {
    fn name(&mut self, attr: &str) -> String {
        let k = format!("#n{}", self.names.len());
        if let Some((existing, _)) = self.names.iter().find(|(_, v)| v.as_str() == attr) {
            return existing.clone();
        }
        self.names.insert(k.clone(), attr.to_owned());
        k
    }

    fn value(&mut self, v: AttributeValue) -> String {
        self.n += 1;
        let k = format!(":v{}", self.n);
        self.values.insert(k.clone(), v);
        k
    }

    fn cond(&mut self, c: &Cond) -> String {
        match c {
            Cond::NotExists => format!("attribute_not_exists({})", self.name("PK")),
            Cond::Exists => format!("attribute_exists({})", self.name("PK")),
            Cond::Eq(a, v) => {
                let (n, v) = (self.name(a), self.value(to_attr(v)));
                format!("{n} = {v}")
            }
            Cond::Ne(a, v) => {
                let (n, v) = (self.name(a), self.value(to_attr(v)));
                format!("(attribute_not_exists({n}) OR {n} <> {v})")
            }
            Cond::In(a, vs) => {
                let n = self.name(a);
                let list: Vec<String> = vs.iter().map(|v| self.value(to_attr(v))).collect();
                format!("{n} IN ({})", list.join(", "))
            }
            Cond::And(cs) => cs.iter().map(|c| format!("({})", self.cond(c))).collect::<Vec<_>>().join(" AND "),
        }
    }

    fn names(&self) -> Option<HashMap<String, String>> {
        (!self.names.is_empty()).then(|| self.names.clone())
    }

    fn values(&self) -> Option<HashMap<String, AttributeValue>> {
        (!self.values.is_empty()).then(|| self.values.clone())
    }
}

fn backend<E: std::fmt::Display>(e: E) -> DbError {
    DbError::Backend(e.to_string())
}

#[async_trait]
impl Store for DynamoStore {
    async fn get(&self, table: Table, pk: &str, sk: &str) -> DbResult<Option<Item>> {
        let out = self
            .client
            .get_item()
            .table_name(self.table(table))
            .set_key(Some(key(pk, sk)))
            .consistent_read(true)
            .send()
            .await
            .map_err(|e| backend(e.into_service_error()))?;
        Ok(out.item().map(attrs_to_item))
    }

    async fn put(&self, table: Table, item: Item, cond: Option<Cond>) -> DbResult<()> {
        let mut e = Expr::default();
        let ce = cond.as_ref().map(|c| e.cond(c));
        let res = self
            .client
            .put_item()
            .table_name(self.table(table))
            .set_item(Some(item_to_attrs(&item)))
            .set_condition_expression(ce)
            .set_expression_attribute_names(e.names())
            .set_expression_attribute_values(e.values())
            .send()
            .await;
        match res {
            Ok(_) => Ok(()),
            Err(err) => {
                let se = err.into_service_error();
                if se.is_conditional_check_failed_exception() { Err(DbError::ConditionFailed) } else { Err(backend(se)) }
            }
        }
    }

    async fn delete(&self, table: Table, pk: &str, sk: &str, cond: Option<Cond>) -> DbResult<()> {
        let mut e = Expr::default();
        let ce = cond.as_ref().map(|c| e.cond(c));
        let res = self
            .client
            .delete_item()
            .table_name(self.table(table))
            .set_key(Some(key(pk, sk)))
            .set_condition_expression(ce)
            .set_expression_attribute_names(e.names())
            .set_expression_attribute_values(e.values())
            .send()
            .await;
        match res {
            Ok(_) => Ok(()),
            Err(err) => {
                let se = err.into_service_error();
                if se.is_conditional_check_failed_exception() { Err(DbError::ConditionFailed) } else { Err(backend(se)) }
            }
        }
    }

    async fn query(&self, q: Query) -> DbResult<Page> {
        let mut e = Expr::default();
        let (pk_attr, sk_attr) = match q.index {
            Some(ix) => (ix.pk_attr(), ix.sk_attr()),
            None => ("PK", "SK"),
        };
        let pkn = e.name(pk_attr);
        let pkv = e.value(AttributeValue::S(q.pk.clone()));
        let mut kc = format!("{pkn} = {pkv}");
        if let Some(c) = &q.sk {
            let skn = e.name(sk_attr);
            let part = match c {
                SkCond::Eq(s) => format!("{skn} = {}", e.value(AttributeValue::S(s.clone()))),
                SkCond::BeginsWith(s) => format!("begins_with({skn}, {})", e.value(AttributeValue::S(s.clone()))),
                SkCond::Between(a, b) => {
                    let (a, b) = (e.value(AttributeValue::S(a.clone())), e.value(AttributeValue::S(b.clone())));
                    format!("{skn} BETWEEN {a} AND {b}")
                }
                SkCond::Gte(s) => format!("{skn} >= {}", e.value(AttributeValue::S(s.clone()))),
                SkCond::Lte(s) => format!("{skn} <= {}", e.value(AttributeValue::S(s.clone()))),
            };
            kc = format!("{kc} AND {part}");
        }
        let start = match &q.cursor {
            Some(c) => {
                let bytes = URL_SAFE_NO_PAD.decode(c).map_err(|_| DbError::BadCursor)?;
                let v: Item = serde_json::from_slice(&bytes).map_err(|_| DbError::BadCursor)?;
                Some(item_to_attrs(&v))
            }
            None => None,
        };
        let out = self
            .client
            .query()
            .table_name(self.table(q.table))
            .set_index_name(q.index.map(|i| i.name().to_owned()))
            .key_condition_expression(kc)
            .set_expression_attribute_names(e.names())
            .set_expression_attribute_values(e.values())
            .scan_index_forward(q.forward)
            .set_limit(q.limit.map(|l| l as i32))
            .set_exclusive_start_key(start)
            .send()
            .await
            .map_err(|e| backend(e.into_service_error()))?;
        let cursor = out.last_evaluated_key().map(|k| URL_SAFE_NO_PAD.encode(serde_json::to_vec(&attrs_to_item(k)).unwrap_or_default()));
        Ok(Page { items: out.items().iter().map(attrs_to_item).collect(), cursor })
    }

    async fn transact(&self, ops: Vec<TxOp>) -> DbResult<()> {
        if ops.len() > 100 {
            return Err(DbError::TooManyOps(ops.len()));
        }
        let mut items = Vec::with_capacity(ops.len());
        for op in ops {
            let mut e = Expr::default();
            let ti = match op {
                TxOp::Put { table, item, cond } => {
                    let ce = cond.as_ref().map(|c| e.cond(c));
                    let put = Put::builder()
                        .table_name(self.table(table))
                        .set_item(Some(item_to_attrs(&item)))
                        .set_condition_expression(ce)
                        .set_expression_attribute_names(e.names())
                        .set_expression_attribute_values(e.values())
                        .build()
                        .map_err(backend)?;
                    TransactWriteItem::builder().put(put).build()
                }
                TxOp::Delete { table, pk, sk, cond } => {
                    let ce = cond.as_ref().map(|c| e.cond(c));
                    let del = Delete::builder()
                        .table_name(self.table(table))
                        .set_key(Some(key(&pk, &sk)))
                        .set_condition_expression(ce)
                        .set_expression_attribute_names(e.names())
                        .set_expression_attribute_values(e.values())
                        .build()
                        .map_err(backend)?;
                    TransactWriteItem::builder().delete(del).build()
                }
                TxOp::Check { table, pk, sk, cond } => {
                    let ce = e.cond(&cond);
                    let chk = ConditionCheck::builder()
                        .table_name(self.table(table))
                        .set_key(Some(key(&pk, &sk)))
                        .condition_expression(ce)
                        .set_expression_attribute_names(e.names())
                        .set_expression_attribute_values(e.values())
                        .build()
                        .map_err(backend)?;
                    TransactWriteItem::builder().condition_check(chk).build()
                }
            };
            items.push(ti);
        }
        match self.client.transact_write_items().set_transact_items(Some(items)).send().await {
            Ok(_) => Ok(()),
            Err(err) => {
                let se = err.into_service_error();
                if let aws_sdk_dynamodb::operation::transact_write_items::TransactWriteItemsError::TransactionCanceledException(tc) = &se {
                    let reasons = tc.cancellation_reasons().iter().map(|r| r.code().filter(|c| *c != "None").map(str::to_owned)).collect();
                    return Err(DbError::TransactionCancelled(reasons));
                }
                Err(backend(se))
            }
        }
    }

    async fn batch_get(&self, table: Table, keys: &[(String, String)]) -> DbResult<Vec<Item>> {
        let name = self.table(table);
        let mut out = Vec::new();
        for chunk in keys.chunks(100) {
            let mut pending: Vec<HashMap<String, AttributeValue>> = chunk.iter().map(|(pk, sk)| key(pk, sk)).collect();
            for attempt in 0..5 {
                if pending.is_empty() {
                    break;
                }
                let ka = KeysAndAttributes::builder().set_keys(Some(pending.clone())).consistent_read(true).build().map_err(backend)?;
                let res =
                    self.client.batch_get_item().request_items(&name, ka).send().await.map_err(|e| backend(e.into_service_error()))?;
                if let Some(rs) = res.responses().and_then(|r| r.get(&name)) {
                    out.extend(rs.iter().map(attrs_to_item));
                }
                pending = res.unprocessed_keys().and_then(|u| u.get(&name)).map(|k| k.keys().to_vec()).unwrap_or_default();
                if !pending.is_empty() {
                    tokio::time::sleep(std::time::Duration::from_millis(50 << attempt)).await;
                }
            }
            if !pending.is_empty() {
                return Err(DbError::Backend("batch_get: unprocessed keys after retries".into()));
            }
        }
        Ok(out)
    }

    async fn batch_write(&self, table: Table, puts: Vec<Item>, deletes: Vec<(String, String)>) -> DbResult<()> {
        let name = self.table(table);
        let mut reqs: Vec<WriteRequest> = Vec::with_capacity(puts.len() + deletes.len());
        for i in &puts {
            reqs.push(
                WriteRequest::builder()
                    .put_request(PutRequest::builder().set_item(Some(item_to_attrs(i))).build().map_err(backend)?)
                    .build(),
            );
        }
        for (pk, sk) in &deletes {
            reqs.push(
                WriteRequest::builder()
                    .delete_request(DeleteRequest::builder().set_key(Some(key(pk, sk))).build().map_err(backend)?)
                    .build(),
            );
        }
        for chunk in reqs.chunks(25) {
            let mut pending = chunk.to_vec();
            for attempt in 0..6 {
                if pending.is_empty() {
                    break;
                }
                let res = self
                    .client
                    .batch_write_item()
                    .request_items(&name, pending.clone())
                    .send()
                    .await
                    .map_err(|e| backend(e.into_service_error()))?;
                pending = res.unprocessed_items().and_then(|u| u.get(&name)).cloned().unwrap_or_default();
                if !pending.is_empty() {
                    tokio::time::sleep(std::time::Duration::from_millis(50 << attempt)).await;
                }
            }
            if !pending.is_empty() {
                return Err(DbError::Backend("batch_write: unprocessed items after retries".into()));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn attribute_round_trip() {
        let v = json!({"a": 1, "b": -2, "c": 1.5, "d": "x", "e": [true, null], "f": {"g": "h"}, "big": 18446744073709551615u64});
        let item: Item = v.as_object().unwrap().clone();
        assert_eq!(attrs_to_item(&item_to_attrs(&item)), item);
    }

    #[test]
    fn builds_condition_expressions() {
        let mut e = Expr::default();
        let s = e.cond(&Cond::NotExists.and(Cond::version(3)).and(Cond::In("status".into(), vec![json!("A"), json!("B")])));
        assert!(s.contains("attribute_not_exists(#n0)"), "{s}");
        assert!(s.contains("IN (:v2, :v3)"), "{s}");
        assert_eq!(e.names().unwrap().len(), 3);
    }
}
