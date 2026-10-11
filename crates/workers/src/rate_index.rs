//! Weekly Kabad Rate Index and city impact aggregation (PLAN.md §12.8, §11.11).
//!
//! There is no global scan index over settled deals, so this module splits the
//! work in two:
//!
//! * [`on_deal_settled`] runs on each `DealSettled` event. It reads the deal's
//!   final lines, derives a `paise_per_kg` price sample for every KG-unit line,
//!   and writes one durable *price-sample* item per line plus a per-week *group
//!   registry* item. It also folds the deal's weighed tonnage into the city's
//!   [`CityImpact`] (kg by category family, hazardous-to-authorized tonnage and
//!   CO2e avoided). It is guarded by [`Db::first_time`] so a redelivered event
//!   is counted exactly once.
//! * [`run`] is the scheduled batch. For an ISO week it enumerates the groups
//!   that saw activity (via the registry), reads back the price samples per
//!   `(city, category, grade)`, drops IQR outliers, computes the median / p25 /
//!   p75 with [`kabados_pricing::stats`], and writes one unpublished
//!   [`RateIndexEntry`] per group. Rows carry deal/vendor counts so an ops
//!   action can publish only those above the k-anonymity threshold.
//!
//! [`Db::first_time`]: kabados_db::repo::Db::first_time

use crate::WorkerCtx;
use anyhow::Context;
use chrono::{DateTime, Datelike, Duration, Utc};
use kabados_db::repo::{CityImpact, RateIndexEntry};
use kabados_db::store::Query;
use kabados_db::{Meta, Record, Table, keys};
use kabados_domain::deal::LineStatus;
use kabados_domain::ids::{DealId, UserId};
use kabados_domain::{Grams, Paise};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Outlier fence used by the Rate Index: 1.5 × IQR (expressed in tenths).
const IQR_K_TENTHS: i64 = 15;

/// Grade key used when a line has no grade, so `(city, category, grade)` keys
/// stay well-formed.
const UNGRADED: &str = "_";

/// Summary returned by the scheduled [`run`].
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RateIndexRun {
    /// Distinct cities that had at least one group this week.
    pub cities: usize,
    /// `RateIndexEntry` rows written.
    pub rows: usize,
}

/// A single `(city, category, grade)` price observation from one settled line,
/// stored so [`run`] can recompute exact medians for the week.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct RatePriceSample {
    city: String,
    category_id: String,
    grade: String,
    iso_week: String,
    deal_id: DealId,
    vendor_id: UserId,
    line_n: u32,
    paise_per_kg: Paise,
    weighed_g: Grams,
    at: DateTime<Utc>,
}

/// Registry marker so [`run`] can enumerate the active groups of a week without
/// a table scan.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct RateGroup {
    city: String,
    category_id: String,
    grade: String,
    iso_week: String,
}

/// ISO-week label like `2026-W41`, lexically sortable within a year.
pub fn iso_week_of(dt: DateTime<Utc>) -> String {
    let w = dt.iso_week();
    format!("{}-W{:02}", w.year(), w.week())
}

fn group_pk(iso_week: &str) -> String {
    format!("RATEWEEK#{iso_week}")
}

fn group_sk(city: &str, category: &str, grade: &str) -> String {
    format!("GROUP#{city}#{category}#{grade}")
}

fn sample_sk(iso_week: &str, deal: &DealId, line_n: u32) -> String {
    format!("SAMPLE#{iso_week}#{deal}#{line_n:04}")
}

fn sample_prefix(iso_week: &str) -> String {
    format!("SAMPLE#{iso_week}#")
}

/// Records one settled deal into the weekly price samples and the running city
/// impact. Idempotent: a redelivered `DealSettled` is ignored after the first.
pub async fn on_deal_settled(ctx: &WorkerCtx, deal_id: &DealId) -> anyhow::Result<()> {
    // Exactly-once guard — keep the marker well beyond the batch window so a
    // late redelivery still cannot double-count a deal.
    if !ctx.db.first_time("rateidx", &format!("settled#{deal_id}"), Duration::days(120)).await? {
        return Ok(());
    }

    let deal = ctx.db.deal(deal_id).await?.context("rate_index: settled deal not found")?.data;
    let listing = ctx.db.listing(&deal.listing_id).await?.context("rate_index: deal listing not found")?.data;
    let city = listing.approx.city.clone();
    let iso_week = iso_week_of(deal.updated_at);

    let tax = ctx.taxonomy().await?;
    let vendor = ctx.db.vendor(&deal.vendor_id).await?.context("rate_index: deal vendor not found")?.data;
    let vendor_hazardous_authorized = vendor.hazardous_auth_valid(Utc::now());

    let children = ctx.db.deal_children(deal_id).await?;
    let now = Utc::now();

    // City-impact accumulators, folded into the stored record at the end.
    let mut kg_by_family: Vec<(String, i64)> = Vec::new();
    let mut hazardous_to_authorized_g: i64 = 0;
    let mut co2e_avoided_g: i64 = 0;

    for rec in &children.lines {
        let line = &rec.data;
        if line.status == LineStatus::Removed {
            continue;
        }
        let Some(weighed) = line.weighed_g else { continue };
        if weighed.0 <= 0 {
            continue;
        }
        let Some(cat) = tax.get(&line.category_id) else { continue };

        // Every weighed line (KG or piece) contributes to the city's physical
        // impact...
        let family = cat.family().to_owned();
        match kg_by_family.iter_mut().find(|(f, _)| *f == family) {
            Some((_, g)) => *g += weighed.0,
            None => kg_by_family.push((family, weighed.0)),
        }
        co2e_avoided_g += weighed.0.saturating_mul(cat.emission_factor_g_per_kg) / 1000;
        if cat.hazardous && vendor_hazardous_authorized {
            hazardous_to_authorized_g += weighed.0;
        }

        // ...but only KG-unit lines with a settled amount feed the price index.
        if cat.unit != kabados_domain::taxonomy::Unit::Kg {
            continue;
        }
        let Some(amount) = line.amount else { continue };
        // paise per kg = amount / (weighed_g / 1000), integer, rounded.
        let per_kg = kabados_domain::money::div_round(amount.0 as i128 * 1000, weighed.0 as i128) as i64;
        if per_kg <= 0 {
            continue;
        }
        let grade = line.grade.clone().unwrap_or_else(|| UNGRADED.to_owned());

        let sample = RatePriceSample {
            city: city.clone(),
            category_id: line.category_id.clone(),
            grade: grade.clone(),
            iso_week: iso_week.clone(),
            deal_id: deal.id.clone(),
            vendor_id: deal.vendor_id.clone(),
            line_n: line.n,
            paise_per_kg: Paise(per_kg),
            weighed_g: weighed,
            at: now,
        };
        let sample_rec = Record::new(
            Meta::new(keys::rate(&city, &line.category_id, &grade), sample_sk(&iso_week, &deal.id, line.n), "rate_sample")
                .ttl(now + Duration::days(120)),
            sample,
        );
        ctx.db.put_rec(Table::Core, &sample_rec).await?;

        // Registry marker (overwrite-idempotent) so run() can find this group.
        let group =
            RateGroup { city: city.clone(), category_id: line.category_id.clone(), grade: grade.clone(), iso_week: iso_week.clone() };
        let group_rec = Record::new(
            Meta::new(group_pk(&iso_week), group_sk(&city, &line.category_id, &grade), "rate_group").ttl(now + Duration::days(120)),
            group,
        );
        ctx.db.put_rec(Table::Core, &group_rec).await?;
    }

    // Fold this deal into the city's cumulative impact (read-modify-write; the
    // first_time guard makes each deal count once, but concurrent settlements
    // for the same city can still race — acceptable for a v1 dashboard figure).
    let mut impact = ctx.db.city_impact(&city).await?.unwrap_or_else(|| CityImpact {
        city: city.clone(),
        kg_by_category: Vec::new(),
        hazardous_to_authorized: Grams(0),
        co2e_avoided_g: 0,
        deals: 0,
        emission_factor_source: tax.emission_factor_source.clone(),
        updated_at: now,
    });
    for (family, grams) in kg_by_family {
        match impact.kg_by_category.iter_mut().find(|(f, _)| *f == family) {
            Some((_, g)) => g.0 += grams,
            None => impact.kg_by_category.push((family, Grams(grams))),
        }
    }
    impact.kg_by_category.sort_by(|a, b| a.0.cmp(&b.0));
    impact.hazardous_to_authorized.0 += hazardous_to_authorized_g;
    impact.co2e_avoided_g += co2e_avoided_g;
    impact.deals += 1;
    impact.emission_factor_source = tax.emission_factor_source.clone();
    impact.updated_at = now;
    ctx.db.put_city_impact(impact).await?;

    Ok(())
}

/// Recomputes the Rate Index for one ISO week from the stored price samples.
///
/// For each active `(city, category, grade)` group it drops IQR outliers and
/// writes one unpublished [`RateIndexEntry`] carrying the median / p25 / p75 and
/// the distinct deal and vendor counts. Rows are written `published = false`;
/// publishing above the k-anonymity threshold is a separate ops action.
pub async fn run(ctx: &WorkerCtx, iso_week: &str) -> anyhow::Result<RateIndexRun> {
    let groups: Vec<RateGroup> = ctx
        .db
        .query_all_recs::<RateGroup>(Query::new(Table::Core, group_pk(iso_week)).begins_with("GROUP#"))
        .await?
        .into_iter()
        .map(|r| r.data)
        .collect();

    let mut out = RateIndexRun::default();
    let mut cities: BTreeSet<String> = BTreeSet::new();

    for g in groups {
        let samples: Vec<RatePriceSample> = ctx
            .db
            .query_all_recs::<RatePriceSample>(
                Query::new(Table::Core, keys::rate(&g.city, &g.category_id, &g.grade)).begins_with(sample_prefix(iso_week)),
            )
            .await?
            .into_iter()
            .map(|r| r.data)
            .collect();
        if samples.is_empty() {
            continue;
        }

        // Distinct deals/vendors over all samples (k-anonymity signal), prices
        // after dropping outliers.
        let deals: BTreeSet<String> = samples.iter().map(|s| s.deal_id.to_string()).collect();
        let vendors: BTreeSet<String> = samples.iter().map(|s| s.vendor_id.to_string()).collect();
        let values: Vec<Paise> = samples.iter().map(|s| s.paise_per_kg).collect();
        let (kept, _dropped) = kabados_pricing::stats::without_outliers(&values, IQR_K_TENTHS);
        let Some(summary) = kabados_pricing::stats::summarize(&kept) else { continue };

        let entry = RateIndexEntry {
            city: g.city.clone(),
            category_id: g.category_id.clone(),
            grade: g.grade.clone(),
            iso_week: iso_week.to_owned(),
            median_paise_per_kg: summary.median,
            p25_paise_per_kg: summary.p25,
            p75_paise_per_kg: summary.p75,
            deals: deals.len() as u32,
            vendors: vendors.len() as u32,
            published: false,
        };
        ctx.db.put_rate(entry).await?;
        cities.insert(g.city);
        out.rows += 1;
    }

    out.cities = cities.len();
    Ok(out)
}
