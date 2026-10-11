//! Pooled collection drives (PLAN.md §9).

use super::Db;
use super::offers::{Pointer, REF};
use crate::keys;
use crate::record::{Meta, Record};
use crate::store::*;
use kabados_domain::drive::*;
use kabados_domain::ids::*;

pub fn drive_meta(d: &Drive) -> Meta {
    Meta::new(keys::drive(&d.id), keys::META, "drive")
        .status(format!("{:?}", d.status).to_uppercase())
        .gsi2(keys::user(&d.admin_id), keys::drive_by_date(&d.date.to_string()))
}

impl Db {
    pub async fn create_drive(&self, d: Drive) -> DbResult<Record<Drive>> {
        let rec = Record::new(drive_meta(&d), d);
        let ptr = Record::new(
            Meta::new(keys::drive_code(&rec.data.join_code), REF, "pointer"),
            Pointer { parent_pk: rec.meta.pk.clone(), sk: keys::META.into() },
        );
        self.store.transact(vec![rec.insert_op(Table::Core)?, ptr.insert_op(Table::Core)?]).await?;
        Ok(rec)
    }

    pub async fn drive(&self, id: &DriveId) -> DbResult<Option<Record<Drive>>> {
        self.get_rec(Table::Core, &keys::drive(id), keys::META).await
    }

    pub async fn drive_by_code(&self, code: &str) -> DbResult<Option<Record<Drive>>> {
        let Some(p) = self.get_rec::<Pointer>(Table::Core, &keys::drive_code(code), REF).await? else { return Ok(None) };
        self.get_rec(Table::Core, &p.data.parent_pk, &p.data.sk).await
    }

    pub fn save_drive_op(&self, rec: &mut Record<Drive>) -> DbResult<TxOp> {
        rec.meta =
            Meta { version: rec.meta.version, created_at: rec.meta.created_at, updated_at: rec.meta.updated_at, ..drive_meta(&rec.data) };
        rec.update_op(Table::Core)
    }

    pub fn drive_member_op(&self, m: &DriveMember) -> DbResult<TxOp> {
        Record::new(Meta::new(keys::drive(&m.drive_id), keys::drive_member(&m.listing_id), "drive_member"), m.clone())
            .insert_op(Table::Core)
    }

    pub async fn drive_members(&self, id: &DriveId) -> DbResult<Vec<Record<DriveMember>>> {
        self.query_all_recs(Query::new(Table::Core, keys::drive(id)).begins_with("MEMBER#")).await
    }

    pub async fn admin_drives(&self, admin: &UserId) -> DbResult<Vec<Record<Drive>>> {
        self.query_all_recs(Query::new(Table::Core, keys::user(admin)).index(Index::Gsi2).begins_with("DRIVE#")).await
    }
}
