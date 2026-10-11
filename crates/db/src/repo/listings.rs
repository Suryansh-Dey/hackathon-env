//! Listings and their versions, items, claims, proofs, images and match results.

use super::outbox::{Notification, notify_op, outbox_op};
use super::{Db, of_type, ts_key};
use crate::keys;
use crate::record::{Meta, Record};
use crate::store::*;
use kabados_domain::events::DomainEvent;
use kabados_domain::ids::*;
use kabados_domain::listing::{self as dl, *};
use serde::{Deserialize, Serialize};

pub fn listing_meta(l: &Listing) -> Meta {
    let status = l.status.as_str();
    let mut m = Meta::new(keys::listing(&l.id), keys::META, "listing")
        .status(status)
        .gsi2(keys::user(&l.seller_id), keys::listing_by_status(status, &ts_key(l.created_at)));
    // Sparse feed index: only while PUBLISHED (PLAN.md §16.4).
    if l.status == ListingStatus::Published {
        let published = l.published_at.map(ts_key).unwrap_or_default();
        m = m.gsi1(keys::feed(&l.approx.h3_r7), format!("{published}#{}", l.id));
    }
    m
}

/// Everything about one listing version, read with a single query.
#[derive(Clone, Debug)]
pub struct ListingBundle {
    pub listing: Record<Listing>,
    pub version: Record<ListingVersion>,
    pub items: Vec<Record<dl::Item>>,
    pub claims: Vec<Record<Claim>>,
    pub proofs: Vec<Record<ProofRequest>>,
    pub images: Vec<Record<ImageMeta>>,
    pub match_result: Option<Record<serde_json::Value>>,
}

impl ListingBundle {
    pub fn item_data(&self) -> Vec<dl::Item> {
        self.items.iter().map(|r| r.data.clone()).collect()
    }
}

/// Last computed match result for a version (PLAN.md §16.4).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredMatch {
    pub result: serde_json::Value,
    pub cache_key: String,
    pub computed_at: chrono::DateTime<chrono::Utc>,
}

impl Db {
    pub async fn create_listing(&self, listing: Listing, version: ListingVersion) -> DbResult<(Record<Listing>, Record<ListingVersion>)> {
        let l = Record::new(listing_meta(&listing), listing);
        let v = Record::new(Meta::new(keys::listing(&l.data.id), keys::listing_version(version.ver), "listing_version"), version);
        self.store.transact(vec![l.insert_op(Table::Core)?, v.insert_op(Table::Core)?]).await?;
        Ok((l, v))
    }

    pub async fn listing(&self, id: &ListingId) -> DbResult<Option<Record<Listing>>> {
        self.get_rec(Table::Core, &keys::listing(id), keys::META).await
    }

    pub async fn listing_bundle(&self, id: &ListingId, ver: Option<u32>) -> DbResult<Option<ListingBundle>> {
        let Some(listing) = self.listing(id).await? else { return Ok(None) };
        let ver = ver.unwrap_or(listing.data.current_version);
        let items = self.store.query_all(Query::new(Table::Core, keys::listing(id)).begins_with(keys::listing_version(ver))).await?;
        let version = match items.iter().find(|i| str_attr(i, "SK") == Some(keys::listing_version(ver).as_str())) {
            Some(v) => Record::from_item(v.clone())?,
            None => return Ok(None),
        };
        Ok(Some(ListingBundle {
            listing,
            version,
            items: of_type(&items, "item")?,
            claims: of_type(&items, "claim")?,
            proofs: of_type(&items, "proof")?,
            images: of_type(&items, "image")?,
            match_result: of_type(&items, "match")?.pop(),
        }))
    }

    /// Saves listing META (recomputing index keys) with optimistic locking.
    pub fn save_listing_op(&self, rec: &mut Record<Listing>) -> DbResult<TxOp> {
        rec.meta =
            Meta { version: rec.meta.version, created_at: rec.meta.created_at, updated_at: rec.meta.updated_at, ..listing_meta(&rec.data) };
        rec.update_op(Table::Core)
    }

    pub async fn save_listing(&self, rec: &mut Record<Listing>) -> DbResult<()> {
        let op = self.save_listing_op(rec)?;
        self.store.transact(vec![op]).await.inspect_err(|_| rec.meta.version -= 1)
    }

    /// Saves listing META plus extra ops, events and notifications atomically.
    pub async fn save_listing_with(
        &self,
        rec: &mut Record<Listing>,
        mut extra: Vec<TxOp>,
        events: Vec<DomainEvent>,
        notes: Vec<Notification>,
    ) -> DbResult<()> {
        let pk = keys::listing(&rec.data.id);
        let mut ops = vec![self.save_listing_op(rec)?];
        ops.append(&mut extra);
        for e in events {
            ops.push(outbox_op(&pk, e, None)?);
        }
        for n in notes {
            ops.push(notify_op(&pk, n)?);
        }
        self.store.transact(ops).await.inspect_err(|_| rec.meta.version -= 1)
    }

    pub fn version_op(&self, v: &Record<ListingVersion>) -> DbResult<TxOp> {
        let rec =
            Record::new(Meta::new(keys::listing(&v.data.listing_id), keys::listing_version(v.data.ver), "listing_version"), v.data.clone());
        rec.put_op(Table::Core)
    }

    pub fn item_op(&self, listing: &ListingId, ver: u32, item: &dl::Item) -> DbResult<TxOp> {
        Record::new(Meta::new(keys::listing(listing), keys::item(ver, &item.id), "item"), item.clone()).put_op(Table::Core)
    }

    pub fn delete_item_op(&self, listing: &ListingId, ver: u32, item: &ItemId) -> TxOp {
        TxOp::Delete { table: Table::Core, pk: keys::listing(listing), sk: keys::item(ver, item), cond: None }
    }

    pub fn claim_op(&self, listing: &ListingId, ver: u32, c: &Claim) -> DbResult<TxOp> {
        Record::new(Meta::new(keys::listing(listing), keys::claim(ver, &c.id), "claim"), c.clone()).put_op(Table::Core)
    }

    pub fn proof_op(&self, listing: &ListingId, ver: u32, p: &ProofRequest) -> DbResult<TxOp> {
        Record::new(
            Meta::new(keys::listing(listing), keys::proof(ver, &p.id), "proof").status(format!("{:?}", p.status).to_uppercase()),
            p.clone(),
        )
        .put_op(Table::Core)
    }

    pub fn image_op(&self, listing: &ListingId, ver: u32, img: &ImageMeta) -> DbResult<TxOp> {
        Record::new(Meta::new(keys::listing(listing), keys::image(ver, &img.id), "image"), img.clone()).put_op(Table::Core)
    }

    pub async fn put_image(&self, listing: &ListingId, ver: u32, img: &ImageMeta) -> DbResult<()> {
        self.store.transact(vec![self.image_op(listing, ver, img)?]).await
    }

    pub async fn image(&self, listing: &ListingId, ver: u32, id: &ImageId) -> DbResult<Option<Record<ImageMeta>>> {
        self.get_rec(Table::Core, &keys::listing(listing), &keys::image(ver, id)).await
    }

    /// Writes many child items (chunked transactions of ≤ 100).
    pub async fn write_ops(&self, ops: Vec<TxOp>) -> DbResult<()> {
        let mut ops = ops;
        while !ops.is_empty() {
            let rest = ops.split_off(ops.len().min(100));
            self.store.transact(std::mem::replace(&mut ops, rest)).await?;
        }
        Ok(())
    }

    pub async fn put_match_result(&self, listing: &ListingId, ver: u32, m: StoredMatch) -> DbResult<()> {
        self.put_rec(Table::Core, &Record::new(Meta::new(keys::listing(listing), keys::match_result(ver), "match"), m)).await
    }

    pub async fn match_result(&self, listing: &ListingId, ver: u32) -> DbResult<Option<Record<StoredMatch>>> {
        self.get_rec(Table::Core, &keys::listing(listing), &keys::match_result(ver)).await
    }

    /// Seller's listings, newest first (GSI2).
    pub async fn seller_listings(
        &self,
        seller: &UserId,
        cursor: Option<String>,
        limit: u32,
    ) -> DbResult<(Vec<Record<Listing>>, Option<String>)> {
        // GSI2SK = LISTING#<status>#<ts>; fetch all statuses and sort by creation.
        let (mut recs, next) = self
            .query_recs::<Listing>(
                Query::new(Table::Core, keys::user(seller)).index(Index::Gsi2).begins_with("LISTING#").limit(limit).cursor(cursor),
            )
            .await?;
        recs.sort_by_key(|r| std::cmp::Reverse(r.data.created_at));
        Ok((recs, next))
    }

    /// Published listings in an r7 cell, newest first (GSI1, sparse).
    pub async fn feed(&self, cell_r7: &str, cursor: Option<String>, limit: u32) -> DbResult<(Vec<Record<Listing>>, Option<String>)> {
        self.query_recs(Query::new(Table::Core, keys::feed(cell_r7)).index(Index::Gsi1).reverse().limit(limit).cursor(cursor)).await
    }

    /// Records a perceptual-hash band entry for near-duplicate lookup (PLAN.md §14).
    pub async fn put_phash_bands(&self, image: &ImageId, listing: &ListingId, seller: &UserId, bands: &[(u8, u16)]) -> DbResult<()> {
        #[derive(Serialize)]
        struct Band<'a> {
            image_id: &'a ImageId,
            listing_id: &'a ListingId,
            seller_id: &'a UserId,
        }
        let mut puts = Vec::new();
        for (b, v) in bands {
            puts.push(
                Record::new(
                    Meta::new(keys::phash(*b, *v), keys::img_sk(image), "phash"),
                    Band { image_id: image, listing_id: listing, seller_id: seller },
                )
                .to_item()?,
            );
        }
        self.store.batch_write(Table::Core, puts, vec![]).await
    }

    /// Candidate near-duplicates sharing any band.
    pub async fn phash_candidates(&self, bands: &[(u8, u16)]) -> DbResult<Vec<crate::store::Item>> {
        let mut out = Vec::new();
        for (b, v) in bands {
            out.extend(self.store.query_all(Query::new(Table::Core, keys::phash(*b, *v))).await?);
        }
        Ok(out)
    }
}
