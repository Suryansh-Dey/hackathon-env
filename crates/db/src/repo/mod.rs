//! Typed repositories over [`Store`]. Grouped per aggregate as `impl Db`
//! blocks; every key comes from [`crate::keys`].

mod chat;
mod deals;
mod drives;
mod ephemeral;
mod ledger;
mod listings;
mod offers;
mod outbox;
mod reference;
mod rulesets;
mod users;

pub use chat::*;
pub use deals::*;
pub use drives::*;
pub use ephemeral::*;
pub use ledger::*;
pub use listings::*;
pub use offers::*;
pub use outbox::*;
pub use reference::*;
pub use rulesets::*;
pub use users::*;

use crate::record::Record;
use crate::store::*;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::sync::Arc;

/// Repository facade shared by API handlers and workers.
#[derive(Clone)]
pub struct Db {
    pub store: Arc<dyn Store>,
}

impl Db {
    pub fn new(store: Arc<dyn Store>) -> Self {
        Self { store }
    }

    pub async fn get_rec<T: DeserializeOwned>(&self, table: Table, pk: &str, sk: &str) -> DbResult<Option<Record<T>>> {
        match self.store.get(table, pk, sk).await? {
            Some(i) => Ok(Some(Record::from_item(i)?)),
            None => Ok(None),
        }
    }

    pub async fn insert_rec<T: Serialize>(&self, table: Table, rec: &Record<T>) -> DbResult<()> {
        self.store.put(table, rec.to_item()?, Some(Cond::NotExists)).await
    }

    /// Optimistic-lock update: bumps `version`, conditioned on the old one.
    pub async fn update_rec<T: Serialize>(&self, table: Table, rec: &mut Record<T>) -> DbResult<()> {
        let TxOp::Put { item, cond, .. } = rec.update_op(table)? else { unreachable!() };
        let res = self.store.put(table, item, cond).await;
        if res.is_err() {
            rec.meta.version -= 1;
        }
        res
    }

    pub async fn put_rec<T: Serialize>(&self, table: Table, rec: &Record<T>) -> DbResult<()> {
        self.store.put(table, rec.to_item()?, None).await
    }

    pub async fn query_recs<T: DeserializeOwned>(&self, q: Query) -> DbResult<(Vec<Record<T>>, Option<String>)> {
        let page = self.store.query(q).await?;
        let recs = page.items.into_iter().map(Record::from_item).collect::<DbResult<Vec<_>>>()?;
        Ok((recs, page.cursor))
    }

    pub async fn query_all_recs<T: DeserializeOwned>(&self, q: Query) -> DbResult<Vec<Record<T>>> {
        self.store.query_all(q).await?.into_iter().map(Record::from_item).collect()
    }
}

/// Splits items of one item collection by their `type` attribute.
pub(crate) fn of_type<T: DeserializeOwned>(items: &[Item], ty: &str) -> DbResult<Vec<Record<T>>> {
    items.iter().filter(|i| str_attr(i, "type") == Some(ty)).map(|i| Record::from_item(i.clone())).collect()
}

pub(crate) fn ts_key(t: chrono::DateTime<chrono::Utc>) -> String {
    t.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()
}
