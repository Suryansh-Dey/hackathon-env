//! Chat threads (core table) and messages (`chat` table), PLAN.md §16.3.

use super::offers::{Pointer, REF};
use super::{Db, ts_key};
use crate::keys;
use crate::record::{Meta, Record};
use crate::store::*;
use kabados_domain::chat::*;
use kabados_domain::ids::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ThreadIndex {
    pub thread_id: ThreadId,
    pub listing_id: ListingId,
    pub role: String,
}

fn thread_index_ops(t: &Thread) -> DbResult<Vec<TxOp>> {
    let last = ts_key(t.last_message_at.unwrap_or(t.created_at));
    [("SELLER", &t.seller_id), ("VENDOR", &t.vendor_id)]
        .into_iter()
        .map(|(role, user)| {
            let meta =
                Meta::new(keys::thread(&t.id), format!("IDX#{role}"), "thread_index").gsi2(keys::user(user), format!("THREAD#{last}"));
            Record::new(meta, ThreadIndex { thread_id: t.id.clone(), listing_id: t.listing_id.clone(), role: role.into() })
                .put_op(Table::Core)
        })
        .collect()
}

impl Db {
    /// Returns the existing thread for (listing, vendor) or creates it.
    pub async fn thread_for(&self, t: Thread) -> DbResult<Record<Thread>> {
        let ptr_pk = keys::thread_for(&t.listing_id, &t.vendor_id);
        if let Some(p) = self.get_rec::<Pointer>(Table::Core, &ptr_pk, REF).await?
            && let Some(existing) = self.get_rec::<Thread>(Table::Core, &p.data.parent_pk, &p.data.sk).await?
        {
            return Ok(existing);
        }
        let rec = Record::new(Meta::new(keys::thread(&t.id), keys::META, "thread"), t);
        let ptr = Record::new(Meta::new(ptr_pk, REF, "pointer"), Pointer { parent_pk: rec.meta.pk.clone(), sk: keys::META.into() });
        let mut ops = vec![rec.insert_op(Table::Core)?, ptr.insert_op(Table::Core)?];
        ops.extend(thread_index_ops(&rec.data)?);
        self.store.transact(ops).await?;
        Ok(rec)
    }

    pub async fn thread(&self, id: &ThreadId) -> DbResult<Option<Record<Thread>>> {
        self.get_rec(Table::Core, &keys::thread(id), keys::META).await
    }

    pub async fn user_threads(&self, user: &UserId) -> DbResult<Vec<ThreadIndex>> {
        let recs = self
            .query_all_recs::<ThreadIndex>(Query::new(Table::Core, keys::user(user)).index(Index::Gsi2).begins_with("THREAD#").reverse())
            .await?;
        Ok(recs.into_iter().map(|r| r.data).collect())
    }

    pub async fn post_message(&self, thread: &mut Record<Thread>, m: Message) -> DbResult<Record<Message>> {
        let rec = Record::new(Meta::new(keys::thread(&m.thread_id), keys::message_sk(&ts_key(m.at), &m.id), "message"), m);
        self.store.put(Table::Chat, rec.to_item()?, Some(Cond::NotExists)).await?;
        thread.data.last_message_at = Some(rec.data.at);
        let mut ops = vec![thread.update_op(Table::Core)?];
        ops.extend(thread_index_ops(&thread.data)?);
        self.store.transact(ops).await.inspect_err(|_| thread.meta.version -= 1)?;
        Ok(rec)
    }

    pub async fn messages(
        &self,
        thread: &ThreadId,
        cursor: Option<String>,
        limit: u32,
    ) -> DbResult<(Vec<Record<Message>>, Option<String>)> {
        self.query_recs(Query::new(Table::Chat, keys::thread(thread)).reverse().limit(limit).cursor(cursor)).await
    }
}
