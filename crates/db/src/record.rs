//! Item envelope (PLAN.md §16.2): generic keys, `type`, `schema_v`,
//! timestamps and an optimistic-lock `version`; entity fields live under
//! `data` so they can never collide with envelope attributes.

use crate::store::{Cond, DbError, DbResult, Item, Table, TxOp};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub struct Meta {
    pub pk: String,
    pub sk: String,
    pub gsi1: Option<(String, String)>,
    pub gsi2: Option<(String, String)>,
    pub gsi3: Option<(String, String)>,
    pub ty: String,
    /// Mirrored top-level for condition expressions and analytics.
    pub status: Option<String>,
    /// Epoch seconds; DynamoDB TTL attribute.
    pub ttl: Option<i64>,
    pub version: u64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Meta {
    pub fn new(pk: impl Into<String>, sk: impl Into<String>, ty: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            pk: pk.into(),
            sk: sk.into(),
            gsi1: None,
            gsi2: None,
            gsi3: None,
            ty: ty.into(),
            status: None,
            ttl: None,
            version: 1,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn gsi1(mut self, pk: impl Into<String>, sk: impl Into<String>) -> Self {
        self.gsi1 = Some((pk.into(), sk.into()));
        self
    }

    pub fn gsi2(mut self, pk: impl Into<String>, sk: impl Into<String>) -> Self {
        self.gsi2 = Some((pk.into(), sk.into()));
        self
    }

    pub fn gsi3(mut self, pk: impl Into<String>, sk: impl Into<String>) -> Self {
        self.gsi3 = Some((pk.into(), sk.into()));
        self
    }

    pub fn status(mut self, s: impl Into<String>) -> Self {
        self.status = Some(s.into());
        self
    }

    pub fn ttl(mut self, at: DateTime<Utc>) -> Self {
        self.ttl = Some(at.timestamp());
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Record<T> {
    pub meta: Meta,
    pub data: T,
}

const ENVELOPE: [&str; 15] = [
    "PK",
    "SK",
    "GSI1PK",
    "GSI1SK",
    "GSI2PK",
    "GSI2SK",
    "GSI3PK",
    "GSI3SK",
    "type",
    "status",
    "ttl",
    "version",
    "created_at",
    "updated_at",
    "schema_v",
];

impl<T> Record<T> {
    pub fn new(meta: Meta, data: T) -> Self {
        Self { meta, data }
    }

    /// Condition that the stored item still has this record's version.
    pub fn version_cond(&self) -> Cond {
        Cond::version(self.meta.version)
    }
}

impl<T: Serialize> Record<T> {
    pub fn to_item(&self) -> DbResult<Item> {
        let m = &self.meta;
        let mut item = Item::new();
        item.insert("PK".into(), m.pk.clone().into());
        item.insert("SK".into(), m.sk.clone().into());
        for (n, g) in [(1, &m.gsi1), (2, &m.gsi2), (3, &m.gsi3)] {
            if let Some((pk, sk)) = g {
                item.insert(format!("GSI{n}PK"), pk.clone().into());
                item.insert(format!("GSI{n}SK"), sk.clone().into());
            }
        }
        item.insert("type".into(), m.ty.clone().into());
        if let Some(s) = &m.status {
            item.insert("status".into(), s.clone().into());
        }
        if let Some(t) = m.ttl {
            item.insert("ttl".into(), t.into());
        }
        item.insert("version".into(), m.version.into());
        item.insert("schema_v".into(), kabados_domain::SCHEMA_V.into());
        item.insert("created_at".into(), m.created_at.to_rfc3339().into());
        item.insert("updated_at".into(), m.updated_at.to_rfc3339().into());
        item.insert("data".into(), serde_json::to_value(&self.data)?);
        Ok(item)
    }

    /// Put that must not overwrite an existing item.
    pub fn insert_op(&self, table: Table) -> DbResult<TxOp> {
        Ok(TxOp::Put { table, item: self.to_item()?, cond: Some(Cond::NotExists) })
    }

    /// Bumps the version and returns a put conditioned on the previous version.
    pub fn update_op(&mut self, table: Table) -> DbResult<TxOp> {
        let cond = self.version_cond();
        self.meta.version += 1;
        self.meta.updated_at = Utc::now();
        Ok(TxOp::Put { table, item: self.to_item()?, cond: Some(cond) })
    }

    pub fn put_op(&self, table: Table) -> DbResult<TxOp> {
        Ok(TxOp::Put { table, item: self.to_item()?, cond: None })
    }
}

fn s(item: &Item, k: &str) -> Option<String> {
    item.get(k).and_then(Value::as_str).map(str::to_owned)
}

fn ts(item: &Item, k: &str) -> DateTime<Utc> {
    item.get(k)
        .and_then(Value::as_str)
        .and_then(|v| DateTime::parse_from_rfc3339(v).ok())
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_default()
}

impl<T: DeserializeOwned> Record<T> {
    pub fn from_item(mut item: Item) -> DbResult<Self> {
        let pair = |item: &Item, n: u8| match (s(item, &format!("GSI{n}PK")), s(item, &format!("GSI{n}SK"))) {
            (Some(a), Some(b)) => Some((a, b)),
            _ => None,
        };
        let meta = Meta {
            pk: s(&item, "PK").ok_or_else(|| DbError::Serde("item without PK".into()))?,
            sk: s(&item, "SK").ok_or_else(|| DbError::Serde("item without SK".into()))?,
            gsi1: pair(&item, 1),
            gsi2: pair(&item, 2),
            gsi3: pair(&item, 3),
            ty: s(&item, "type").unwrap_or_default(),
            status: s(&item, "status"),
            ttl: item.get("ttl").and_then(Value::as_i64),
            version: item.get("version").and_then(Value::as_u64).unwrap_or(0),
            created_at: ts(&item, "created_at"),
            updated_at: ts(&item, "updated_at"),
        };
        let data = match item.remove("data") {
            Some(d) => serde_json::from_value(d)?,
            None => {
                for k in ENVELOPE {
                    item.remove(k);
                }
                serde_json::from_value(Value::Object(item))?
            }
        };
        Ok(Self { meta, data })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Thing {
        version: u32,
        name: String,
    }

    #[test]
    fn round_trip_keeps_entity_version_separate() {
        let mut r =
            Record::new(Meta::new("A#1", "META", "thing").gsi2("U#1", "T#x").status("ACTIVE"), Thing { version: 7, name: "x".into() });
        let item = r.to_item().unwrap();
        assert_eq!(item["version"], 1);
        assert_eq!(item["data"]["version"], 7);
        let back: Record<Thing> = Record::from_item(item).unwrap();
        assert_eq!(back.data, r.data);
        assert_eq!(back.meta.gsi2, Some(("U#1".into(), "T#x".into())));
        let op = r.update_op(Table::Core).unwrap();
        match op {
            TxOp::Put { cond: Some(c), item, .. } => {
                assert_eq!(c, Cond::version(1));
                assert_eq!(item["version"], 2);
            }
            _ => panic!(),
        }
    }
}
