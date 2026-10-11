//! Double-entry ledger in the `ledger` table (PLAN.md §13.4, §16.6).

use super::{Db, ts_key};
use crate::keys;
use crate::record::{Meta, Record};
use crate::store::*;
use kabados_domain::ledger::{Entry, Side, Transaction};
use kabados_domain::{DomainError, Paise};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Balance {
    pub account: String,
    /// Σ credits − Σ debits, paise.
    pub balance: Paise,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HistoryLine {
    pub txn_key: String,
    pub side: Side,
    pub amount: Paise,
    pub balance_after: Paise,
    pub at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PostOutcome {
    Posted,
    /// Same idempotency key already posted (exactly-once).
    AlreadyPosted,
}

impl Db {
    /// Posts a balanced transaction exactly once: the TXN item is created with
    /// `attribute_not_exists`, entries and history are written, and balances
    /// are updated under version conditions, all in one `TransactWriteItems`.
    pub async fn post_transaction(&self, txn: &Transaction) -> Result<PostOutcome, DbError> {
        txn.validate().map_err(|e: DomainError| DbError::Serde(e.to_string()))?;
        for attempt in 0..5 {
            if self.store.get(Table::Ledger, &keys::txn(&txn.idempotency_key), keys::META).await?.is_some() {
                return Ok(PostOutcome::AlreadyPosted);
            }
            let mut deltas: BTreeMap<String, i64> = BTreeMap::new();
            for e in &txn.entries {
                *deltas.entry(e.account.key()).or_default() += signed(e);
            }
            let mut ops =
                vec![Record::new(Meta::new(keys::txn(&txn.idempotency_key), keys::META, "txn"), txn.clone()).insert_op(Table::Ledger)?];
            for (n, e) in txn.entries.iter().enumerate() {
                ops.push(
                    Record::new(Meta::new(keys::txn(&txn.idempotency_key), keys::entry(n as u32), "entry"), e.clone())
                        .insert_op(Table::Ledger)?,
                );
            }
            for (acct, delta) in &deltas {
                let current = self.get_rec::<Balance>(Table::Ledger, &keys::account(acct), keys::BALANCE).await?;
                let (mut rec, op_new) = match current {
                    Some(r) => (r, false),
                    None => (
                        Record::new(
                            Meta::new(keys::account(acct), keys::BALANCE, "balance"),
                            Balance { account: acct.clone(), balance: Paise(0) },
                        ),
                        true,
                    ),
                };
                rec.data.balance = Paise(rec.data.balance.0 + delta);
                ops.push(if op_new { rec.insert_op(Table::Ledger)? } else { rec.update_op(Table::Ledger)? });
                let side = if *delta >= 0 { Side::Credit } else { Side::Debit };
                let line = HistoryLine {
                    txn_key: txn.idempotency_key.clone(),
                    side,
                    amount: Paise(delta.abs()),
                    balance_after: rec.data.balance,
                    at: txn.occurred_at,
                };
                ops.push(
                    Record::new(
                        Meta::new(
                            keys::account(acct),
                            keys::account_history(&ts_key(txn.occurred_at), &txn.idempotency_key),
                            "account_line",
                        ),
                        line,
                    )
                    .insert_op(Table::Ledger)?,
                );
            }
            match self.store.transact(ops).await {
                Ok(()) => return Ok(PostOutcome::Posted),
                Err(DbError::TransactionCancelled(reasons)) => {
                    if reasons.first().is_some_and(Option::is_some) {
                        return Ok(PostOutcome::AlreadyPosted);
                    }
                    // Lost a balance race; re-read and retry.
                    tokio::time::sleep(std::time::Duration::from_millis(20 << attempt)).await;
                }
                Err(e) => return Err(e),
            }
        }
        Err(DbError::Backend("ledger post: too much contention".into()))
    }

    pub async fn balance(&self, account: &str) -> DbResult<Paise> {
        Ok(self
            .get_rec::<Balance>(Table::Ledger, &keys::account(account), keys::BALANCE)
            .await?
            .map(|r| r.data.balance)
            .unwrap_or_default())
    }

    pub async fn account_history(&self, account: &str, cursor: Option<String>) -> DbResult<(Vec<HistoryLine>, Option<String>)> {
        let (recs, next) = self
            .query_recs::<HistoryLine>(
                Query::new(Table::Ledger, keys::account(account)).begins_with("TXN#").reverse().limit(100).cursor(cursor),
            )
            .await?;
        Ok((recs.into_iter().map(|r| r.data).collect(), next))
    }

    pub async fn ledger_transaction(&self, key: &str) -> DbResult<Option<Transaction>> {
        Ok(self.get_rec::<Transaction>(Table::Ledger, &keys::txn(key), keys::META).await?.map(|r| r.data))
    }
}

fn signed(e: &Entry) -> i64 {
    match e.side {
        Side::Credit => e.amount.0,
        Side::Debit => -e.amount.0,
    }
}
