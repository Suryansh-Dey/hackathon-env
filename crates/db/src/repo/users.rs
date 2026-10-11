//! Users, roles, addresses, payouts, consents, devices, vendor and
//! transporter profiles, KYC documents and trusted transporter lists.

use super::{Db, ts_key};
use crate::keys;
use crate::record::{Meta, Record};
use crate::store::*;
use kabados_domain::ids::*;
use kabados_domain::user::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PhoneClaim {
    pub user_id: UserId,
}

pub fn user_meta(p: &UserProfile) -> Meta {
    Meta::new(keys::user(&p.id), keys::PROFILE, "user").gsi3(keys::cognito(&p.cognito_sub), keys::PROFILE)
}

pub fn vendor_meta(v: &VendorProfile) -> Meta {
    let status = serde_json::to_value(v.kyc_status).ok().and_then(|s| s.as_str().map(str::to_owned)).unwrap_or_default();
    let submitted = v.kyc_submitted_at.map(ts_key).unwrap_or_else(|| ts_key(v.created_at));
    Meta::new(keys::user(&v.user_id), keys::VENDOR, "vendor")
        .gsi1(keys::kyc_queue(&status), format!("{submitted}#VENDOR#{}", v.user_id))
        .status(status)
}

pub fn transporter_meta(t: &TransporterProfile) -> Meta {
    let status = serde_json::to_value(t.kyc_status).ok().and_then(|s| s.as_str().map(str::to_owned)).unwrap_or_default();
    let submitted = t.kyc_submitted_at.map(ts_key).unwrap_or_else(|| ts_key(t.created_at));
    Meta::new(keys::user(&t.user_id), keys::TRANSPORTER, "transporter")
        .gsi1(keys::kyc_queue(&status), format!("{submitted}#TRANSPORTER#{}", t.user_id))
        .status(status)
}

/// Item in the KYC queue (vendor or transporter).
#[derive(Clone, Debug, PartialEq)]
pub enum KycSubject {
    Vendor(Box<Record<VendorProfile>>),
    Transporter(Box<Record<TransporterProfile>>),
}

impl Db {
    /// Creates a profile and claims the phone number in one transaction (PLAN.md §16.5).
    pub async fn create_user(&self, profile: UserProfile) -> DbResult<Record<UserProfile>> {
        let claim =
            Record::new(Meta::new(keys::phone(&profile.phone_hash), keys::PHONE, "phone"), PhoneClaim { user_id: profile.id.clone() });
        let rec = Record::new(user_meta(&profile), profile);
        self.store.transact(vec![rec.insert_op(Table::Core)?, claim.insert_op(Table::Core)?]).await?;
        Ok(rec)
    }

    pub async fn user(&self, id: &UserId) -> DbResult<Option<Record<UserProfile>>> {
        self.get_rec(Table::Core, &keys::user(id), keys::PROFILE).await
    }

    pub async fn user_by_sub(&self, sub: &str) -> DbResult<Option<Record<UserProfile>>> {
        let (mut recs, _) = self.query_recs(Query::new(Table::Core, keys::cognito(sub)).index(Index::Gsi3).limit(1)).await?;
        Ok(recs.pop())
    }

    pub async fn user_by_phone_hash(&self, hash: &str) -> DbResult<Option<UserId>> {
        Ok(self.get_rec::<PhoneClaim>(Table::Core, &keys::phone(hash), keys::PHONE).await?.map(|r| r.data.user_id))
    }

    pub async fn save_user(&self, rec: &mut Record<UserProfile>) -> DbResult<()> {
        rec.meta =
            Meta { version: rec.meta.version, created_at: rec.meta.created_at, updated_at: rec.meta.updated_at, ..user_meta(&rec.data) };
        self.update_rec(Table::Core, rec).await
    }

    pub async fn put_address(&self, user: &UserId, a: Address) -> DbResult<Record<Address>> {
        let rec = Record::new(Meta::new(keys::user(user), keys::addr(&a.id), "address"), a);
        self.put_rec(Table::Core, &rec).await?;
        Ok(rec)
    }

    pub async fn address(&self, user: &UserId, id: &AddressId) -> DbResult<Option<Record<Address>>> {
        self.get_rec(Table::Core, &keys::user(user), &keys::addr(id)).await
    }

    pub async fn addresses(&self, user: &UserId) -> DbResult<Vec<Record<Address>>> {
        self.query_all_recs(Query::new(Table::Core, keys::user(user)).begins_with("ADDR#")).await
    }

    pub async fn delete_address(&self, user: &UserId, id: &AddressId) -> DbResult<()> {
        self.store.delete(Table::Core, &keys::user(user), &keys::addr(id), None).await
    }

    pub async fn put_payout_method(&self, user: &UserId, p: PayoutMethod) -> DbResult<Record<PayoutMethod>> {
        let rec = Record::new(Meta::new(keys::user(user), keys::payout(&p.id), "payout_method"), p);
        self.put_rec(Table::Core, &rec).await?;
        Ok(rec)
    }

    pub async fn payout_methods(&self, user: &UserId) -> DbResult<Vec<Record<PayoutMethod>>> {
        self.query_all_recs(Query::new(Table::Core, keys::user(user)).begins_with("PAYOUT#")).await
    }

    pub async fn put_consent(&self, user: &UserId, c: Consent) -> DbResult<()> {
        let purpose = serde_json::to_value(c.purpose)?.as_str().unwrap_or_default().to_owned();
        let rec = Record::new(Meta::new(keys::user(user), keys::consent(&purpose, &c.notice_version), "consent"), c);
        self.put_rec(Table::Core, &rec).await
    }

    pub async fn consents(&self, user: &UserId) -> DbResult<Vec<Record<Consent>>> {
        self.query_all_recs(Query::new(Table::Core, keys::user(user)).begins_with("CONSENT#")).await
    }

    pub async fn put_device(&self, user: &UserId, d: Device) -> DbResult<()> {
        let rec = Record::new(Meta::new(keys::user(user), keys::device(&d.device_id), "device"), d);
        self.put_rec(Table::Core, &rec).await
    }

    pub async fn devices(&self, user: &UserId) -> DbResult<Vec<Record<Device>>> {
        self.query_all_recs(Query::new(Table::Core, keys::user(user)).begins_with("DEVICE#")).await
    }

    pub async fn vendor(&self, user: &UserId) -> DbResult<Option<Record<VendorProfile>>> {
        self.get_rec(Table::Core, &keys::user(user), keys::VENDOR).await
    }

    pub async fn insert_vendor(&self, v: VendorProfile) -> DbResult<Record<VendorProfile>> {
        let rec = Record::new(vendor_meta(&v), v);
        self.insert_rec(Table::Core, &rec).await?;
        Ok(rec)
    }

    pub async fn save_vendor(&self, rec: &mut Record<VendorProfile>) -> DbResult<()> {
        rec.meta =
            Meta { version: rec.meta.version, created_at: rec.meta.created_at, updated_at: rec.meta.updated_at, ..vendor_meta(&rec.data) };
        self.update_rec(Table::Core, rec).await
    }

    pub async fn transporter(&self, user: &UserId) -> DbResult<Option<Record<TransporterProfile>>> {
        self.get_rec(Table::Core, &keys::user(user), keys::TRANSPORTER).await
    }

    pub async fn insert_transporter(&self, t: TransporterProfile) -> DbResult<Record<TransporterProfile>> {
        let rec = Record::new(transporter_meta(&t), t);
        self.insert_rec(Table::Core, &rec).await?;
        Ok(rec)
    }

    pub async fn save_transporter(&self, rec: &mut Record<TransporterProfile>) -> DbResult<()> {
        rec.meta = Meta {
            version: rec.meta.version,
            created_at: rec.meta.created_at,
            updated_at: rec.meta.updated_at,
            ..transporter_meta(&rec.data)
        };
        self.update_rec(Table::Core, rec).await
    }

    /// Ops KYC queue, oldest submission first (PLAN.md §10.1).
    pub async fn kyc_queue(&self, status: KycStatus, cursor: Option<String>, limit: u32) -> DbResult<(Vec<KycSubject>, Option<String>)> {
        let s = serde_json::to_value(status)?.as_str().unwrap_or_default().to_owned();
        let page = self.store.query(Query::new(Table::Core, keys::kyc_queue(&s)).index(Index::Gsi1).limit(limit).cursor(cursor)).await?;
        let mut out = Vec::new();
        for i in page.items {
            match str_attr(&i, "type") {
                Some("vendor") => out.push(KycSubject::Vendor(Box::new(Record::from_item(i)?))),
                Some("transporter") => out.push(KycSubject::Transporter(Box::new(Record::from_item(i)?))),
                _ => {}
            }
        }
        Ok((out, page.cursor))
    }

    pub async fn put_document(&self, user: &UserId, d: KycDocument) -> DbResult<Record<KycDocument>> {
        let rec = Record::new(Meta::new(keys::user(user), keys::doc(&d.id), "document"), d);
        self.put_rec(Table::Core, &rec).await?;
        Ok(rec)
    }

    pub async fn documents(&self, user: &UserId) -> DbResult<Vec<Record<KycDocument>>> {
        self.query_all_recs(Query::new(Table::Core, keys::user(user)).begins_with("DOC#")).await
    }

    pub async fn put_my_transporter(&self, m: MyTransporter) -> DbResult<()> {
        let meta = Meta::new(keys::user(&m.vendor_id), keys::my_transporter(&m.transporter_id), "my_transporter")
            .gsi2(keys::user(&m.transporter_id), keys::my_vendor(&m.vendor_id));
        self.put_rec(Table::Core, &Record::new(meta, m)).await
    }

    pub async fn my_transporters(&self, vendor: &UserId) -> DbResult<Vec<Record<MyTransporter>>> {
        self.query_all_recs(Query::new(Table::Core, keys::user(vendor)).begins_with("MYTRANSPORTER#")).await
    }

    pub async fn my_vendors(&self, transporter: &UserId) -> DbResult<Vec<Record<MyTransporter>>> {
        self.query_all_recs(Query::new(Table::Core, keys::user(transporter)).index(Index::Gsi2).begins_with("MYVENDOR#")).await
    }
}
