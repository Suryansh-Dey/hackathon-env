//! Rule set versions and the coverage index (PLAN.md §7.2, §12.5).

use super::Db;
use super::outbox::outbox_op;
use crate::keys;
use crate::record::{Meta, Record};
use crate::store::*;
use kabados_domain::events::DomainEvent;
use kabados_domain::ids::UserId;
use kabados_domain::ruleset::{RuleSet, RuleSetStatus};
use kabados_domain::user::{RuleSetRef, VendorProfile};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// One coverage index entry: vendor V prices category C in cell X.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CoverageEntry {
    pub vendor_id: UserId,
    pub category_id: String,
    pub cell_r8: String,
    pub rule_set_version: u32,
}

/// The vendor's current coverage keys, so a rewrite can delete stale entries.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CoverageManifest {
    pub rule_set_version: u32,
    /// `cell#category` pairs.
    pub entries: Vec<String>,
}

impl Db {
    pub async fn rule_draft(&self, vendor: &UserId) -> DbResult<Option<Record<RuleSet>>> {
        self.get_rec(Table::Core, &keys::user(vendor), keys::RULESET_DRAFT).await
    }

    pub async fn put_rule_draft(&self, vendor: &UserId, draft: RuleSet) -> DbResult<Record<RuleSet>> {
        let rec = Record::new(Meta::new(keys::user(vendor), keys::RULESET_DRAFT, "ruleset_draft").status("DRAFT"), draft);
        self.put_rec(Table::Core, &rec).await?;
        Ok(rec)
    }

    pub async fn latest_rule_set(&self, vendor: &UserId) -> DbResult<Option<Record<RuleSet>>> {
        let (mut v, _) =
            self.query_recs(Query::new(Table::Core, keys::user(vendor)).begins_with(keys::RULESET_PREFIX).reverse().limit(1)).await?;
        Ok(v.pop())
    }

    pub async fn rule_set_version(&self, vendor: &UserId, version: u32) -> DbResult<Option<Record<RuleSet>>> {
        self.get_rec(Table::Core, &keys::user(vendor), &keys::ruleset(version)).await
    }

    pub async fn rule_set_history(&self, vendor: &UserId, limit: u32) -> DbResult<Vec<Record<RuleSet>>> {
        Ok(self.query_recs(Query::new(Table::Core, keys::user(vendor)).begins_with(keys::RULESET_PREFIX).reverse().limit(limit)).await?.0)
    }

    /// Publishes an immutable version, points the vendor at it, clears the
    /// draft and emits `RuleSetPublished`, all in one transaction.
    pub async fn publish_rule_set(&self, vendor: &mut Record<VendorProfile>, mut rules: RuleSet) -> DbResult<Record<RuleSet>> {
        rules.status = RuleSetStatus::Published;
        let id = vendor.data.user_id.clone();
        let rec = Record::new(Meta::new(keys::user(&id), keys::ruleset(rules.version), "ruleset").status("PUBLISHED"), rules);
        vendor.data.active_rule_set = Some(RuleSetRef { rule_set_id: rec.data.rule_set_id.clone(), version: rec.data.version });
        vendor.meta = Meta {
            version: vendor.meta.version,
            created_at: vendor.meta.created_at,
            updated_at: vendor.meta.updated_at,
            ..super::users::vendor_meta(&vendor.data)
        };
        let ops = vec![
            rec.insert_op(Table::Core)?,
            vendor.update_op(Table::Core)?,
            TxOp::Delete { table: Table::Core, pk: keys::user(&id), sk: keys::RULESET_DRAFT.into(), cond: None },
            outbox_op(
                &keys::user(&id),
                DomainEvent::RuleSetPublished {
                    vendor_id: id.clone(),
                    rule_set_id: rec.data.rule_set_id.clone(),
                    version: rec.data.version,
                },
                None,
            )?,
        ];
        if let Err(e) = self.store.transact(ops).await {
            vendor.meta.version -= 1;
            return Err(e);
        }
        Ok(rec)
    }

    pub async fn coverage_manifest(&self, vendor: &UserId) -> DbResult<CoverageManifest> {
        Ok(self
            .get_rec::<CoverageManifest>(Table::Core, &keys::user(vendor), keys::COVERAGE_MANIFEST)
            .await?
            .map(|r| r.data)
            .unwrap_or_default())
    }

    /// Rewrites a vendor's coverage entries (PLAN.md §12.5). `pairs` are
    /// `(cell_r8, category_id)`; an empty set removes the vendor from the index.
    pub async fn rewrite_coverage(
        &self,
        vendor: &UserId,
        rule_set_version: u32,
        pairs: &BTreeSet<(String, String)>,
    ) -> DbResult<(usize, usize)> {
        let old = self.coverage_manifest(vendor).await?;
        let new_keys: BTreeSet<String> = pairs.iter().map(|(c, k)| format!("{c}#{k}")).collect();
        let old_keys: BTreeSet<String> = old.entries.into_iter().collect();
        let deletes: Vec<(String, String)> = old_keys
            .difference(&new_keys)
            .filter_map(|k| k.split_once('#'))
            .map(|(c, cat)| (keys::coverage(c, cat), keys::vendor_sk(vendor)))
            .collect();
        let mut puts = Vec::with_capacity(pairs.len());
        for (cell, cat) in pairs {
            let e = CoverageEntry { vendor_id: vendor.clone(), category_id: cat.clone(), cell_r8: cell.clone(), rule_set_version };
            puts.push(Record::new(Meta::new(keys::coverage(cell, cat), keys::vendor_sk(vendor), "coverage"), e).to_item()?);
        }
        let (n_put, n_del) = (puts.len(), deletes.len());
        self.store.batch_write(Table::Core, puts, deletes).await?;
        let manifest = CoverageManifest { rule_set_version, entries: new_keys.into_iter().collect() };
        self.put_rec(Table::Core, &Record::new(Meta::new(keys::user(vendor), keys::COVERAGE_MANIFEST, "coverage_manifest"), manifest))
            .await?;
        Ok((n_put, n_del))
    }

    /// Vendors covering a category in a cell.
    pub async fn coverage(&self, cell_r8: &str, category: &str) -> DbResult<Vec<CoverageEntry>> {
        Ok(self
            .query_all_recs::<CoverageEntry>(Query::new(Table::Core, keys::coverage(cell_r8, category)))
            .await?
            .into_iter()
            .map(|r| r.data)
            .collect())
    }
}
