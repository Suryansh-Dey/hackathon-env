//! Coverage index builder (PLAN.md §12.5): which vendor prices which category
//! in which r8 cell. Rewritten on `RuleSetPublished` and authorization changes.

use crate::error::ApiResult;
use crate::state::AppState;
use chrono::Utc;
use kabados_domain::ids::UserId;
use std::collections::BTreeSet;

/// Rebuilds one vendor's coverage entries; returns (puts, deletes).
pub async fn rebuild(st: &AppState, vendor: &UserId) -> ApiResult<(usize, usize)> {
    let Some(v) = st.db.vendor(vendor).await? else { return Ok((0, 0)) };
    let rules = st.db.latest_rule_set(vendor).await?;
    let tax = st.taxonomy().await?;
    let mut pairs = BTreeSet::new();
    let active = v.data.kyc_approved() && !v.data.suspended;
    if let (true, Some(rs)) = (active, rules.as_ref()) {
        let hazardous_ok = v.data.hazardous_auth_valid(Utc::now());
        let cats: Vec<String> = rs.data.covered_categories().into_iter().filter(|c| hazardous_ok || !tax.is_hazardous(c)).collect();
        for cell in rs.data.service_area.cells_r8()? {
            for c in &cats {
                pairs.insert((cell.clone(), c.clone()));
            }
        }
    }
    let version = rules.map(|r| r.data.version).unwrap_or(0);
    Ok(st.db.rewrite_coverage(vendor, version, &pairs).await?)
}
