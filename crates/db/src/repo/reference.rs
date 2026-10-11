//! Reference data (taxonomy overrides, authorized recyclers, Rate Index,
//! city impact) and the append-only audit log.

use super::{Db, ts_key};
use crate::keys;
use crate::record::{Meta, Record};
use crate::store::*;
use chrono::{DateTime, Utc};
use kabados_domain::taxonomy::{Category, Taxonomy};
use kabados_domain::{Grams, Paise};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct AuthorizedRecycler {
    pub authorization_no: String,
    pub state: String,
    pub name: String,
    pub valid_until: Option<DateTime<Utc>>,
    pub source_url: String,
    pub imported_at: DateTime<Utc>,
    /// Dev seed entries are clearly fake and never shipped to prod.
    pub dev_seed: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct RateIndexEntry {
    pub city: String,
    pub category_id: String,
    pub grade: String,
    pub iso_week: String,
    pub median_paise_per_kg: Paise,
    pub p25_paise_per_kg: Paise,
    pub p75_paise_per_kg: Paise,
    pub deals: u32,
    pub vendors: u32,
    pub published: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct CityImpact {
    pub city: String,
    pub kg_by_category: Vec<(String, Grams)>,
    pub hazardous_to_authorized: Grams,
    pub co2e_avoided_g: i64,
    pub deals: u32,
    pub emission_factor_source: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct AuditEntry {
    pub subject: String,
    pub actor: String,
    pub action: String,
    pub detail: serde_json::Value,
    pub at: DateTime<Utc>,
}

impl Db {
    /// Embedded seed taxonomy merged with ops overrides stored as `TAXONOMY/CAT#…`.
    pub async fn taxonomy(&self) -> DbResult<Taxonomy> {
        let overrides = self.query_all_recs::<Category>(Query::new(Table::Core, keys::TAXONOMY).begins_with("CAT#")).await?;
        let seed = Taxonomy::seed();
        if overrides.is_empty() {
            return Ok(seed.clone());
        }
        let mut cats = seed.categories.clone();
        for o in overrides {
            match cats.iter_mut().find(|c| c.id == o.data.id) {
                Some(c) => *c = o.data,
                None => cats.push(o.data),
            }
        }
        Taxonomy::new(format!("{}+ops", seed.version), cats).map_err(|e| DbError::Serde(e.to_string()))
    }

    pub async fn put_category(&self, c: Category) -> DbResult<()> {
        self.put_rec(Table::Core, &Record::new(Meta::new(keys::TAXONOMY, keys::category(&c.id), "category"), c)).await
    }

    pub async fn put_recycler(&self, r: AuthorizedRecycler) -> DbResult<()> {
        self.put_rec(
            Table::Core,
            &Record::new(Meta::new(keys::recycler(&r.state), keys::recycler_auth(&r.authorization_no), "recycler"), r),
        )
        .await
    }

    pub async fn recycler(&self, state: &str, no: &str) -> DbResult<Option<AuthorizedRecycler>> {
        Ok(self.get_rec::<AuthorizedRecycler>(Table::Core, &keys::recycler(state), &keys::recycler_auth(no)).await?.map(|r| r.data))
    }

    pub async fn put_rate(&self, e: RateIndexEntry) -> DbResult<()> {
        let meta = Meta::new(keys::rate(&e.city, &e.category_id, &e.grade), keys::week(&e.iso_week), "rate_index")
            .gsi1(keys::rate_city(&e.city), format!("{}#{}#{}", e.iso_week, e.category_id, e.grade));
        self.put_rec(Table::Core, &Record::new(meta, e)).await
    }

    /// Latest published Rate Index entries for a city (GSI1, newest week first).
    pub async fn rate_index(&self, city: &str, limit: u32) -> DbResult<Vec<RateIndexEntry>> {
        let recs = self
            .query_recs::<RateIndexEntry>(Query::new(Table::Core, keys::rate_city(city)).index(Index::Gsi1).reverse().limit(limit))
            .await?
            .0;
        Ok(recs.into_iter().map(|r| r.data).filter(|e| e.published).collect())
    }

    pub async fn rate_history(&self, city: &str, category: &str, grade: &str) -> DbResult<Vec<RateIndexEntry>> {
        let recs =
            self.query_all_recs::<RateIndexEntry>(Query::new(Table::Core, keys::rate(city, category, grade)).begins_with("WEEK#")).await?;
        Ok(recs.into_iter().map(|r| r.data).collect())
    }

    pub async fn put_city_impact(&self, i: CityImpact) -> DbResult<()> {
        self.put_rec(Table::Core, &Record::new(Meta::new(keys::impact_city(&i.city), keys::META, "impact"), i)).await
    }

    pub async fn city_impact(&self, city: &str) -> DbResult<Option<CityImpact>> {
        Ok(self.get_rec::<CityImpact>(Table::Core, &keys::impact_city(city), keys::META).await?.map(|r| r.data))
    }

    /// Append-only audit entry (PLAN.md §16.3 `audit`).
    pub async fn audit(&self, subject: &str, actor: &str, action: &str, detail: serde_json::Value) -> DbResult<()> {
        let e = AuditEntry { subject: subject.into(), actor: actor.into(), action: action.into(), detail, at: Utc::now() };
        let rec = Record::new(
            Meta::new(keys::subject(subject), format!("{}#{actor}#{}", ts_key(e.at), kabados_domain::ids::EventId::new()), "audit"),
            e,
        );
        self.insert_rec(Table::Audit, &rec).await
    }

    pub async fn audit_log(&self, subject: &str) -> DbResult<Vec<AuditEntry>> {
        Ok(self
            .query_all_recs::<AuditEntry>(Query::new(Table::Audit, keys::subject(subject)).reverse())
            .await?
            .into_iter()
            .map(|r| r.data)
            .collect())
    }
}
