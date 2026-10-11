//! In-memory [`Store`] for unit tests and the local dev server. Emulates
//! keys, GSIs, conditions, transactions, TTL expiry and Streams.

use crate::store::*;
use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Mutex;
use tokio::sync::broadcast;

type Key = (String, String);

#[derive(Default)]
struct State {
    tables: HashMap<Table, BTreeMap<Key, Item>>,
}

pub struct MemoryStore {
    state: Mutex<State>,
    changes: broadcast::Sender<Change>,
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

fn expired(item: &Item, now: i64) -> bool {
    item.get("ttl").and_then(Value::as_i64).is_some_and(|t| t <= now)
}

fn key_of(item: &Item) -> DbResult<Key> {
    match (str_attr(item, "PK"), str_attr(item, "SK")) {
        (Some(pk), Some(sk)) => Ok((pk.to_owned(), sk.to_owned())),
        _ => Err(DbError::Serde("item needs string PK and SK".into())),
    }
}

impl MemoryStore {
    pub fn new() -> Self {
        let (changes, _) = broadcast::channel(4096);
        Self { state: Mutex::new(State::default()), changes }
    }

    /// Streams emulation: receives every change after it is applied.
    pub fn subscribe(&self) -> broadcast::Receiver<Change> {
        self.changes.subscribe()
    }

    /// Snapshot of all live items in a table (tests, dev inspection).
    pub fn dump(&self, table: Table) -> Vec<Item> {
        let now = now_secs();
        let st = self.state.lock().unwrap();
        st.tables.get(&table).map(|t| t.values().filter(|i| !expired(i, now)).cloned().collect()).unwrap_or_default()
    }

    fn emit(&self, changes: Vec<Change>) {
        for c in changes {
            let _ = self.changes.send(c);
        }
    }

    fn live<'a>(t: &'a BTreeMap<Key, Item>, k: &Key, now: i64) -> Option<&'a Item> {
        t.get(k).filter(|i| !expired(i, now))
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Cursor {
    s: String,
    pk: String,
    sk: String,
}

#[async_trait]
impl Store for MemoryStore {
    async fn get(&self, table: Table, pk: &str, sk: &str) -> DbResult<Option<Item>> {
        let st = self.state.lock().unwrap();
        Ok(st.tables.get(&table).and_then(|t| Self::live(t, &(pk.into(), sk.into()), now_secs())).cloned())
    }

    async fn put(&self, table: Table, item: Item, cond: Option<Cond>) -> DbResult<()> {
        self.transact(vec![TxOp::Put { table, item, cond }]).await.map_err(|e| match e {
            DbError::TransactionCancelled(_) => DbError::ConditionFailed,
            e => e,
        })
    }

    async fn delete(&self, table: Table, pk: &str, sk: &str, cond: Option<Cond>) -> DbResult<()> {
        self.transact(vec![TxOp::Delete { table, pk: pk.into(), sk: sk.into(), cond }]).await.map_err(|e| match e {
            DbError::TransactionCancelled(_) => DbError::ConditionFailed,
            e => e,
        })
    }

    async fn query(&self, q: Query) -> DbResult<Page> {
        if q.index.is_some() && !q.table.has_gsis() {
            return Err(DbError::Backend(format!("{:?} has no GSIs", q.table)));
        }
        let now = now_secs();
        let st = self.state.lock().unwrap();
        let empty = BTreeMap::new();
        let t = st.tables.get(&q.table).unwrap_or(&empty);
        // (sort value, pk, sk, item)
        let mut rows: Vec<(String, String, String, &Item)> = match q.index {
            None => t
                .range((q.pk.clone(), String::new())..)
                .take_while(|((pk, _), _)| *pk == q.pk)
                .filter(|(_, i)| !expired(i, now))
                .map(|((pk, sk), i)| (sk.clone(), pk.clone(), sk.clone(), i))
                .collect(),
            Some(ix) => {
                let mut v: Vec<_> = t
                    .iter()
                    .filter(|(_, i)| !expired(i, now) && str_attr(i, ix.pk_attr()) == Some(q.pk.as_str()))
                    .filter_map(|((pk, sk), i)| str_attr(i, ix.sk_attr()).map(|s| (s.to_owned(), pk.clone(), sk.clone(), i)))
                    .collect();
                v.sort_by(|a, b| (&a.0, &a.1, &a.2).cmp(&(&b.0, &b.1, &b.2)));
                v
            }
        };
        if let Some(c) = &q.sk {
            rows.retain(|r| c.matches(&r.0));
        }
        if !q.forward {
            rows.reverse();
        }
        if let Some(cur) = &q.cursor {
            let bytes = URL_SAFE_NO_PAD.decode(cur).map_err(|_| DbError::BadCursor)?;
            let c: Cursor = serde_json::from_slice(&bytes).map_err(|_| DbError::BadCursor)?;
            let pos = rows.iter().position(|r| r.0 == c.s && r.1 == c.pk && r.2 == c.sk).ok_or(DbError::BadCursor)?;
            rows.drain(..=pos);
        }
        let limit = q.limit.unwrap_or(1000) as usize;
        let more = rows.len() > limit;
        rows.truncate(limit);
        let cursor = if more {
            rows.last()
                .map(|r| URL_SAFE_NO_PAD.encode(serde_json::to_vec(&Cursor { s: r.0.clone(), pk: r.1.clone(), sk: r.2.clone() }).unwrap()))
        } else {
            None
        };
        Ok(Page { items: rows.into_iter().map(|r| r.3.clone()).collect(), cursor })
    }

    async fn increment(&self, table: Table, pk: &str, sk: &str, attr: &str, by: i64, ttl: Option<i64>) -> DbResult<i64> {
        let now = now_secs();
        let (value, change) = {
            let mut st = self.state.lock().unwrap();
            let t = st.tables.entry(table).or_default();
            let key = (pk.to_owned(), sk.to_owned());
            let old = t.get(&key).filter(|i| !expired(i, now)).cloned();
            let mut item = old.clone().unwrap_or_else(|| {
                let mut m = Item::new();
                m.insert("PK".into(), pk.into());
                m.insert("SK".into(), sk.into());
                m
            });
            let v = item.get(attr).and_then(Value::as_i64).unwrap_or(0) + by;
            item.insert(attr.into(), v.into());
            if let Some(t) = ttl {
                item.insert("ttl".into(), t.into());
            }
            t.insert(key, item.clone());
            (v, Change { table, old, new: Some(item) })
        };
        self.emit(vec![change]);
        Ok(value)
    }

    async fn transact(&self, ops: Vec<TxOp>) -> DbResult<()> {
        if ops.len() > 100 {
            return Err(DbError::TooManyOps(ops.len()));
        }
        let now = now_secs();
        let changes = {
            let mut st = self.state.lock().unwrap();
            let mut seen: HashSet<(Table, Key)> = HashSet::new();
            let mut reasons: Vec<Option<String>> = Vec::with_capacity(ops.len());
            let mut failed = false;
            for op in &ops {
                let (table, key, cond) = match op {
                    TxOp::Put { table, item, cond } => (*table, key_of(item)?, cond.as_ref()),
                    TxOp::Delete { table, pk, sk, cond } => (*table, (pk.clone(), sk.clone()), cond.as_ref()),
                    TxOp::Check { table, pk, sk, cond } => (*table, (pk.clone(), sk.clone()), Some(cond)),
                };
                if !seen.insert((table, key.clone())) {
                    return Err(DbError::Backend("transaction touches the same item twice".into()));
                }
                let existing = st.tables.get(&table).and_then(|t| Self::live(t, &key, now));
                let ok = cond.is_none_or(|c| c.eval(existing));
                failed |= !ok;
                reasons.push((!ok).then(|| "ConditionalCheckFailed".to_string()));
            }
            if failed {
                return Err(DbError::TransactionCancelled(reasons));
            }
            let mut changes = Vec::new();
            for op in ops {
                match op {
                    TxOp::Put { table, item, .. } => {
                        let key = key_of(&item)?;
                        let t = st.tables.entry(table).or_default();
                        let old = t.insert(key, item.clone()).filter(|i| !expired(i, now));
                        changes.push(Change { table, old, new: Some(item) });
                    }
                    TxOp::Delete { table, pk, sk, .. } => {
                        let old = st.tables.entry(table).or_default().remove(&(pk, sk)).filter(|i| !expired(i, now));
                        if old.is_some() {
                            changes.push(Change { table, old, new: None });
                        }
                    }
                    TxOp::Check { .. } => {}
                }
            }
            changes
        };
        self.emit(changes);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn item(pk: &str, sk: &str, extra: Value) -> Item {
        let mut m = extra.as_object().cloned().unwrap_or_default();
        m.insert("PK".into(), pk.into());
        m.insert("SK".into(), sk.into());
        m
    }

    #[tokio::test]
    async fn conditions_and_transactions() {
        let s = MemoryStore::new();
        s.put(Table::Core, item("A", "1", json!({"version": 1})), Some(Cond::NotExists)).await.unwrap();
        assert!(s.put(Table::Core, item("A", "1", json!({})), Some(Cond::NotExists)).await.unwrap_err().is_conflict());
        let err = s
            .transact(vec![
                TxOp::Put { table: Table::Core, item: item("A", "2", json!({})), cond: None },
                TxOp::Check { table: Table::Core, pk: "A".into(), sk: "1".into(), cond: Cond::version(2) },
            ])
            .await
            .unwrap_err();
        assert!(matches!(err, DbError::TransactionCancelled(ref r) if r[1].is_some()));
        assert!(s.get(Table::Core, "A", "2").await.unwrap().is_none(), "nothing applied on failure");
    }

    #[tokio::test]
    async fn queries_gsis_pagination_and_ttl() {
        let s = MemoryStore::new();
        for i in 0..5 {
            s.put(Table::Core, item("P", &format!("S#{i}"), json!({"GSI1PK": "G", "GSI1SK": format!("{}", 9 - i)})), None).await.unwrap();
        }
        s.put(Table::Core, item("P", "X#1", json!({})), None).await.unwrap();
        let p = s.query(Query::new(Table::Core, "P").begins_with("S#").limit(2)).await.unwrap();
        assert_eq!(p.items.len(), 2);
        let all = s.query_all(Query::new(Table::Core, "P").begins_with("S#").limit(2)).await.unwrap();
        assert_eq!(all.len(), 5);
        let g = s.query(Query::new(Table::Core, "G").index(Index::Gsi1)).await.unwrap();
        assert_eq!(str_attr(&g.items[0], "SK"), Some("S#4"), "sorted by GSI1SK");
        let rev = s.query(Query::new(Table::Core, "P").reverse().limit(1)).await.unwrap();
        assert_eq!(str_attr(&rev.items[0], "SK"), Some("X#1"));

        s.put(Table::Ephemeral, item("T", "T", json!({"ttl": 1})), None).await.unwrap();
        assert!(s.get(Table::Ephemeral, "T", "T").await.unwrap().is_none(), "expired items are invisible");
        s.put(Table::Ephemeral, item("T", "T", json!({})), Some(Cond::NotExists)).await.unwrap();
    }

    #[tokio::test]
    async fn emits_changes() {
        let s = MemoryStore::new();
        let mut rx = s.subscribe();
        s.put(Table::Core, item("A", "1", json!({"v": 1})), None).await.unwrap();
        s.put(Table::Core, item("A", "1", json!({"v": 2})), None).await.unwrap();
        let c1 = rx.recv().await.unwrap();
        assert!(c1.old.is_none());
        let c2 = rx.recv().await.unwrap();
        assert_eq!(c2.old.unwrap()["v"], 1);
    }
}
