//! Offers and offer acceptance (PLAN.md §4.3, §16.5).

use super::deals::{deal_index_ops, deal_meta};
use super::outbox::{Notification, notify_op, outbox_op};
use super::{Db, ts_key};
use crate::keys;
use crate::record::{Meta, Record};
use crate::store::*;
use kabados_domain::deal::Deal;
use kabados_domain::events::DomainEvent;
use kabados_domain::ids::*;
use kabados_domain::listing::Listing;
use kabados_domain::offer::{Offer, OfferStatus};
use serde::{Deserialize, Serialize};

pub const REF: &str = "REF";

/// Pointer from an ID-only route to the owning item collection.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pointer {
    pub parent_pk: String,
    pub sk: String,
}

pub fn offer_meta(o: &Offer) -> Meta {
    Meta::new(keys::listing(&o.listing_id), keys::offer(&o.id), "offer")
        .status(o.status.as_str())
        .gsi2(keys::user(&o.vendor_id), keys::offer_by_status(o.status.as_str(), &ts_key(o.created_at)))
}

impl Db {
    /// Creates an offer (optionally superseding a previous one) with its
    /// pointer, `OfferMade` event and a seller notification.
    pub async fn create_offer(
        &self,
        offer: Offer,
        supersede: Option<&mut Record<Offer>>,
        notes: Vec<Notification>,
    ) -> DbResult<Record<Offer>> {
        let rec = Record::new(offer_meta(&offer), offer);
        let lpk = keys::listing(&rec.data.listing_id);
        let ptr =
            Record::new(Meta::new(keys::offer(&rec.data.id), REF, "pointer"), Pointer { parent_pk: lpk.clone(), sk: rec.meta.sk.clone() });
        let mut ops = vec![rec.insert_op(Table::Core)?, ptr.insert_op(Table::Core)?];
        let mut bumped = None;
        if let Some(old) = supersede {
            old.data.status = OfferStatus::Superseded;
            ops.push(self.save_offer_op(old)?);
            bumped = Some(old);
        }
        ops.push(outbox_op(
            &lpk,
            DomainEvent::OfferMade {
                offer_id: rec.data.id.clone(),
                listing_id: rec.data.listing_id.clone(),
                vendor_id: rec.data.vendor_id.clone(),
            },
            None,
        )?);
        for n in notes {
            ops.push(notify_op(&lpk, n)?);
        }
        if let Err(e) = self.store.transact(ops).await {
            if let Some(old) = bumped {
                old.meta.version -= 1;
                old.data.status = OfferStatus::Active;
            }
            return Err(e);
        }
        Ok(rec)
    }

    pub fn save_offer_op(&self, rec: &mut Record<Offer>) -> DbResult<TxOp> {
        rec.meta =
            Meta { version: rec.meta.version, created_at: rec.meta.created_at, updated_at: rec.meta.updated_at, ..offer_meta(&rec.data) };
        rec.update_op(Table::Core)
    }

    pub async fn save_offer(&self, rec: &mut Record<Offer>, notes: Vec<Notification>) -> DbResult<()> {
        let lpk = keys::listing(&rec.data.listing_id);
        let mut ops = vec![self.save_offer_op(rec)?];
        for n in notes {
            ops.push(notify_op(&lpk, n)?);
        }
        self.store.transact(ops).await.inspect_err(|_| rec.meta.version -= 1)
    }

    pub async fn offer(&self, id: &OfferId) -> DbResult<Option<Record<Offer>>> {
        let Some(p) = self.get_rec::<Pointer>(Table::Core, &keys::offer(id), REF).await? else { return Ok(None) };
        self.get_rec(Table::Core, &p.data.parent_pk, &p.data.sk).await
    }

    pub async fn offers_for_listing(&self, listing: &ListingId) -> DbResult<Vec<Record<Offer>>> {
        self.query_all_recs(Query::new(Table::Core, keys::listing(listing)).begins_with(keys::OFFER_PREFIX)).await
    }

    pub async fn vendor_offers(
        &self,
        vendor: &UserId,
        status: Option<OfferStatus>,
        cursor: Option<String>,
    ) -> DbResult<(Vec<Record<Offer>>, Option<String>)> {
        let prefix = match status {
            Some(s) => format!("OFFER#{}#", s.as_str()),
            None => "OFFER#".into(),
        };
        self.query_recs(
            Query::new(Table::Core, keys::user(vendor)).index(Index::Gsi2).begins_with(prefix).reverse().limit(50).cursor(cursor),
        )
        .await
    }

    /// Accepts an offer in one transaction (PLAN.md §16.5): listing →
    /// RESERVED (conditioned on PUBLISHED + version), offer → ACCEPTED,
    /// deal META + per-party index items, outbox events and notifications.
    /// Other offers are superseded asynchronously by a Streams consumer.
    pub async fn accept_offer(
        &self,
        listing: &mut Record<Listing>,
        offer: &mut Record<Offer>,
        deal: Deal,
        notes: Vec<Notification>,
    ) -> DbResult<Record<Deal>> {
        let lpk = keys::listing(&listing.data.id);
        let deal_rec = Record::new(deal_meta(&deal), deal);
        // Guard on the stored listing still being PUBLISHED at the version we read.
        let mut lop = self.save_listing_op(listing)?;
        if let TxOp::Put { cond, .. } = &mut lop {
            *cond = Some(Cond::status("PUBLISHED").and(cond.take().unwrap_or(Cond::Exists)));
        }
        let mut ops = vec![lop];
        ops.push(self.save_offer_op(offer)?);
        ops.push(deal_rec.insert_op(Table::Core)?);
        ops.extend(deal_index_ops(&deal_rec.data)?);
        let dpk = keys::deal(&deal_rec.data.id);
        ops.push(outbox_op(
            &lpk,
            DomainEvent::OfferAccepted {
                offer_id: offer.data.id.clone(),
                listing_id: listing.data.id.clone(),
                deal_id: deal_rec.data.id.clone(),
            },
            None,
        )?);
        ops.push(outbox_op(
            &dpk,
            DomainEvent::DealCreated {
                deal_id: deal_rec.data.id.clone(),
                listing_id: listing.data.id.clone(),
                seller_id: deal_rec.data.seller_id.clone(),
                vendor_id: deal_rec.data.vendor_id.clone(),
            },
            None,
        )?);
        for n in notes {
            ops.push(notify_op(&dpk, n)?);
        }
        if let Err(e) = self.store.transact(ops).await {
            listing.meta.version -= 1;
            offer.meta.version -= 1;
            return Err(e);
        }
        Ok(deal_rec)
    }
}
