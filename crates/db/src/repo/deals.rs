//! Deals, lines, weighings, jobs, job events, disputes, ratings, calls and receipts.

use super::offers::{Pointer, REF};
use super::outbox::{Notification, notify_op, outbox_op};
use super::{Db, of_type, ts_key};
use crate::keys;
use crate::record::{Meta, Record};
use crate::store::*;
use kabados_domain::deal::*;
use kabados_domain::events::DomainEvent;
use kabados_domain::ids::*;
use kabados_domain::job::*;
use serde::{Deserialize, Serialize};

pub fn deal_meta(d: &Deal) -> Meta {
    Meta::new(keys::deal(&d.id), keys::META, "deal").status(d.status.as_str())
}

/// Per-party index item so each party can list "my deals" (PLAN.md §16.4).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DealIndex {
    pub deal_id: DealId,
    pub role: String,
    pub status: DealStatus,
    pub listing_id: ListingId,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

pub fn deal_index_ops(d: &Deal) -> DbResult<Vec<TxOp>> {
    let mut ops = Vec::new();
    let mut parties = vec![("SELLER", &d.seller_id), ("VENDOR", &d.vendor_id)];
    if let Some(t) = &d.transporter_id {
        parties.push(("TRANSPORTER", t));
    }
    for (role, user) in parties {
        let idx = DealIndex {
            deal_id: d.id.clone(),
            role: role.into(),
            status: d.status,
            listing_id: d.listing_id.clone(),
            created_at: d.created_at,
        };
        let meta = Meta::new(keys::deal(&d.id), keys::deal_index(&d.id, role), "deal_index")
            .status(d.status.as_str())
            .gsi2(keys::user(user), keys::deal_by_status(d.status.as_str(), &ts_key(d.created_at)));
        ops.push(Record::new(meta, idx).put_op(Table::Core)?);
    }
    Ok(ops)
}

pub fn job_meta(j: &Job) -> Meta {
    let mut m = Meta::new(keys::deal(&j.deal_id), keys::job(&j.id), "job").status(j.status.as_str());
    if let Some(t) = &j.transporter_id {
        m = m.gsi2(keys::user(t), keys::job_by_status(j.status.as_str(), &ts_key(j.created_at)));
    }
    m
}

/// Offer of a job to one transporter (indexed for their incoming list).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JobOfferIndex {
    pub job_id: JobId,
    pub deal_id: DealId,
    pub transporter_id: UserId,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

pub fn dispute_meta(d: &Dispute) -> Meta {
    let status = format!("{:?}", d.status).to_uppercase();
    Meta::new(keys::deal(&d.deal_id), keys::dispute(&d.id), "dispute")
        .status(status.clone())
        .gsi1(keys::dispute_queue(&status), ts_key(d.sla_due_at))
}

impl Db {
    pub async fn deal(&self, id: &DealId) -> DbResult<Option<Record<Deal>>> {
        self.get_rec(Table::Core, &keys::deal(id), keys::META).await
    }

    /// Saves deal META and its index items with events and notifications, atomically.
    pub async fn save_deal_with(
        &self,
        rec: &mut Record<Deal>,
        mut extra: Vec<TxOp>,
        events: Vec<DomainEvent>,
        notes: Vec<Notification>,
    ) -> DbResult<()> {
        rec.data.updated_at = chrono::Utc::now();
        rec.meta =
            Meta { version: rec.meta.version, created_at: rec.meta.created_at, updated_at: rec.meta.updated_at, ..deal_meta(&rec.data) };
        let pk = keys::deal(&rec.data.id);
        let mut ops = vec![rec.update_op(Table::Core)?];
        ops.extend(deal_index_ops(&rec.data)?);
        ops.append(&mut extra);
        for e in events {
            ops.push(outbox_op(&pk, e, None)?);
        }
        for n in notes {
            ops.push(notify_op(&pk, n)?);
        }
        self.store.transact(ops).await.inspect_err(|_| rec.meta.version -= 1)
    }

    /// Deals where the user is a party, grouped by status (GSI2).
    pub async fn user_deals(
        &self,
        user: &UserId,
        status: Option<DealStatus>,
        cursor: Option<String>,
    ) -> DbResult<(Vec<DealIndex>, Option<String>)> {
        let prefix = match status {
            Some(s) => format!("DEAL#{}#", s.as_str()),
            None => "DEAL#".into(),
        };
        let (recs, next) = self
            .query_recs::<DealIndex>(
                Query::new(Table::Core, keys::user(user)).index(Index::Gsi2).begins_with(prefix).limit(100).cursor(cursor),
            )
            .await?;
        Ok((recs.into_iter().map(|r| r.data).collect(), next))
    }

    pub fn line_op(&self, deal: &DealId, line: &DealLine) -> DbResult<TxOp> {
        Record::new(Meta::new(keys::deal(deal), keys::line(line.n), "deal_line"), line.clone()).put_op(Table::Core)
    }

    pub fn weighing_op(&self, deal: &DealId, w: &Weighing) -> DbResult<TxOp> {
        Record::new(Meta::new(keys::deal(deal), keys::weighing(w.line_n, &ts_key(w.at)), "weighing"), w.clone()).insert_op(Table::Core)
    }

    /// Lines, weighings, jobs, disputes, ratings and calls of a deal.
    pub async fn deal_children(&self, id: &DealId) -> DbResult<DealChildren> {
        let items = self.store.query_all(Query::new(Table::Core, keys::deal(id))).await?;
        Ok(DealChildren {
            lines: of_type(&items, "deal_line")?,
            weighings: of_type(&items, "weighing")?,
            jobs: of_type(&items, "job")?,
            disputes: of_type(&items, "dispute")?,
            ratings: of_type(&items, "rating")?,
            calls: of_type(&items, "call")?,
            job_events: of_type(&items, "job_event")?,
        })
    }

    pub async fn create_job(
        &self,
        deal: &mut Record<Deal>,
        job: Job,
        events: Vec<DomainEvent>,
        notes: Vec<Notification>,
    ) -> DbResult<Record<Job>> {
        let rec = Record::new(job_meta(&job), job);
        let ptr = Record::new(
            Meta::new(keys::job(&rec.data.id), REF, "pointer"),
            Pointer { parent_pk: keys::deal(&rec.data.deal_id), sk: rec.meta.sk.clone() },
        );
        let mut extra = vec![rec.insert_op(Table::Core)?, ptr.insert_op(Table::Core)?];
        extra.extend(self.job_offer_index_ops(&rec.data)?);
        self.save_deal_with(deal, extra, events, notes).await?;
        Ok(rec)
    }

    pub fn job_offer_index_ops(&self, j: &Job) -> DbResult<Vec<TxOp>> {
        j.offered_to
            .iter()
            .map(|t| {
                let idx = JobOfferIndex {
                    job_id: j.id.clone(),
                    deal_id: j.deal_id.clone(),
                    transporter_id: t.clone(),
                    expires_at: j.offer_expires_at,
                };
                let meta = Meta::new(keys::deal(&j.deal_id), format!("{}#{t}", keys::job_offer_index(&j.id)), "job_offer")
                    .gsi2(keys::user(t), keys::job_by_status("OFFERED", &ts_key(j.created_at)))
                    .ttl(j.offer_expires_at + chrono::Duration::hours(1));
                Record::new(meta, idx).put_op(Table::Core)
            })
            .collect()
    }

    pub fn clear_job_offer_ops(&self, j: &Job) -> Vec<TxOp> {
        j.offered_to
            .iter()
            .map(|t| TxOp::Delete {
                table: Table::Core,
                pk: keys::deal(&j.deal_id),
                sk: format!("{}#{t}", keys::job_offer_index(&j.id)),
                cond: None,
            })
            .collect()
    }

    pub async fn job(&self, id: &JobId) -> DbResult<Option<Record<Job>>> {
        let Some(p) = self.get_rec::<Pointer>(Table::Core, &keys::job(id), REF).await? else { return Ok(None) };
        self.get_rec(Table::Core, &p.data.parent_pk, &p.data.sk).await
    }

    pub fn save_job_op(&self, rec: &mut Record<Job>) -> DbResult<TxOp> {
        rec.data.updated_at = chrono::Utc::now();
        rec.meta =
            Meta { version: rec.meta.version, created_at: rec.meta.created_at, updated_at: rec.meta.updated_at, ..job_meta(&rec.data) };
        rec.update_op(Table::Core)
    }

    pub async fn save_job(
        &self,
        rec: &mut Record<Job>,
        extra: Vec<TxOp>,
        events: Vec<DomainEvent>,
        notes: Vec<Notification>,
    ) -> DbResult<()> {
        let pk = keys::deal(&rec.data.deal_id);
        let mut ops = vec![self.save_job_op(rec)?];
        ops.extend(extra);
        for e in events {
            ops.push(outbox_op(&pk, e, None)?);
        }
        for n in notes {
            ops.push(notify_op(&pk, n)?);
        }
        self.store.transact(ops).await.inspect_err(|_| rec.meta.version -= 1)
    }

    /// Jobs assigned to a transporter, optionally by status prefix.
    pub async fn transporter_jobs(&self, transporter: &UserId, status: Option<JobStatus>) -> DbResult<Vec<Record<Job>>> {
        let prefix = match status {
            Some(s) => format!("JOB#{}#", s.as_str()),
            None => "JOB#".into(),
        };
        let items = self.store.query_all(Query::new(Table::Core, keys::user(transporter)).index(Index::Gsi2).begins_with(prefix)).await?;
        of_type(&items, "job")
    }

    /// Open job offers for a transporter.
    pub async fn transporter_job_offers(&self, transporter: &UserId) -> DbResult<Vec<JobOfferIndex>> {
        let items =
            self.store.query_all(Query::new(Table::Core, keys::user(transporter)).index(Index::Gsi2).begins_with("JOB#OFFERED#")).await?;
        Ok(of_type::<JobOfferIndex>(&items, "job_offer")?.into_iter().map(|r| r.data).collect())
    }

    pub fn job_event_op(&self, deal: &DealId, job: &JobId, e: &JobEvent) -> DbResult<TxOp> {
        Record::new(Meta::new(keys::deal(deal), keys::job_event(job, &ts_key(e.at)), "job_event"), e.clone()).put_op(Table::Core)
    }

    pub fn dispute_op(&self, d: &Dispute) -> DbResult<TxOp> {
        Record::new(dispute_meta(d), d.clone()).put_op(Table::Core)
    }

    pub async fn dispute(&self, deal: &DealId, id: &DisputeId) -> DbResult<Option<Record<Dispute>>> {
        self.get_rec(Table::Core, &keys::deal(deal), &keys::dispute(id)).await
    }

    /// Ops dispute queue ordered by SLA due time (GSI1).
    pub async fn dispute_queue(&self, status: DisputeStatus, cursor: Option<String>) -> DbResult<(Vec<Record<Dispute>>, Option<String>)> {
        let s = format!("{status:?}").to_uppercase();
        self.query_recs(Query::new(Table::Core, keys::dispute_queue(&s)).index(Index::Gsi1).limit(50).cursor(cursor)).await
    }

    /// One rating per party per deal (uniqueness via the key).
    pub fn rating_op(&self, r: &Rating) -> DbResult<TxOp> {
        Record::new(Meta::new(keys::deal(&r.deal_id), keys::rating(&r.from_user), "rating"), r.clone()).insert_op(Table::Core)
    }

    pub fn call_op(&self, c: &CallSession) -> DbResult<TxOp> {
        Record::new(Meta::new(keys::deal(&c.deal_id), keys::call(&c.id), "call"), c.clone()).put_op(Table::Core)
    }

    pub fn receipt_op(&self, r: &GreenReceipt) -> DbResult<TxOp> {
        Record::new(Meta::new(keys::receipt(&r.id), keys::META, "receipt"), r.clone()).insert_op(Table::Core)
    }

    pub async fn receipt(&self, id: &ReceiptId) -> DbResult<Option<Record<GreenReceipt>>> {
        self.get_rec(Table::Core, &keys::receipt(id), keys::META).await
    }
}

#[derive(Clone, Debug, Default)]
pub struct DealChildren {
    pub lines: Vec<Record<DealLine>>,
    pub weighings: Vec<Record<Weighing>>,
    pub jobs: Vec<Record<Job>>,
    pub disputes: Vec<Record<Dispute>>,
    pub ratings: Vec<Record<Rating>>,
    pub calls: Vec<Record<CallSession>>,
    pub job_events: Vec<Record<JobEvent>>,
}
