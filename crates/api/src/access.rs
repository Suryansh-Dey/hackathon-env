//! Builds Cedar principals and resources from stored records and enforces
//! decisions (PLAN.md §18). Cedar is the source of truth on detail views and
//! every mutating call.

use crate::auth::Me;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use chrono::Utc;
use kabados_authz::*;
use kabados_db::Record;
use kabados_domain::deal::Deal;
use kabados_domain::drive::Drive;
use kabados_domain::job::Job;
use kabados_domain::listing::Listing;
use kabados_domain::offer::Offer;
use kabados_domain::user::{TransporterProfile, VendorProfile};

pub fn seller(id: &UserId) -> Principal {
    Principal::Seller(id.clone())
}

pub fn vendor_principal(v: &VendorProfile) -> Principal {
    Principal::Vendor(VendorAttrs {
        id: v.user_id.clone(),
        kyc_approved: v.kyc_approved(),
        hazardous_auth_valid: v.hazardous_auth_valid(Utc::now()),
        suspended: v.suspended,
    })
}

pub fn transporter_principal(t: &TransporterProfile) -> Principal {
    Principal::Transporter(TransporterAttrs { id: t.user_id.clone(), kyc_approved: t.kyc_approved(), suspended: t.suspended })
}

pub fn ops_principal(sub: &str) -> Principal {
    Principal::Ops { id: sub.to_owned(), mfa: true }
}

pub fn listing_attrs(l: &Listing, deal: Option<&Deal>, job: Option<&Job>, drive: Option<&Drive>) -> ListingAttrs {
    ListingAttrs {
        id: l.id.clone(),
        seller: l.seller_id.clone(),
        status: l.status,
        contains_hazardous: l.contains_hazardous,
        accepted_vendor: deal.map(|d| d.vendor_id.clone()),
        deal_status: deal.map(|d| d.status),
        assigned_transporter: job.and_then(|j| j.transporter_id.clone()),
        job_status: job.map(|j| j.status),
        drive_admin: drive.map(|d| d.admin_id.clone()),
    }
}

pub fn deal_attrs(d: &Deal) -> DealAttrs {
    DealAttrs {
        id: d.id.clone(),
        seller: d.seller_id.clone(),
        vendor: d.vendor_id.clone(),
        transporter: d.transporter_id.clone(),
        status: d.status,
    }
}

pub fn job_attrs(j: &Job, d: &Deal) -> JobAttrs {
    JobAttrs {
        id: j.id.clone(),
        deal: deal_attrs(d),
        transporter: j.transporter_id.clone(),
        offered_to: j.offered_to.clone(),
        status: j.status,
    }
}

pub fn offer_attrs(o: &Offer, listing: ListingAttrs) -> OfferAttrs {
    OfferAttrs { id: o.id.clone(), listing, vendor: o.vendor_id.clone(), status: o.status }
}

/// Returns `Forbidden` unless Cedar allows the request.
pub async fn ensure(st: &AppState, p: &Principal, a: Action, r: &Resource) -> ApiResult<()> {
    let d = st.authz.is_authorized(p, a, r).await?;
    if d.allowed {
        Ok(())
    } else {
        tracing::info!(action = a.as_str(), errors = ?d.errors, "authorization denied");
        Err(ApiError::forbidden(format!("you can't {} here", humanize(a))))
    }
}

pub async fn allowed(st: &AppState, p: &Principal, a: Action, r: &Resource) -> ApiResult<bool> {
    Ok(st.authz.is_authorized(p, a, r).await?.allowed)
}

fn humanize(a: Action) -> String {
    let s = a.as_str();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push(' ');
        }
        out.extend(c.to_lowercase());
    }
    out
}

/// The caller's vendor profile, or `Forbidden` when they aren't a vendor.
pub async fn vendor_of(st: &AppState, me: &Me) -> ApiResult<Record<VendorProfile>> {
    st.db.vendor(me.id()).await?.ok_or_else(|| ApiError::forbidden("set up your vendor account first"))
}

pub async fn transporter_of(st: &AppState, me: &Me) -> ApiResult<Record<TransporterProfile>> {
    st.db.transporter(me.id()).await?.ok_or_else(|| ApiError::forbidden("set up your transporter account first"))
}

/// Deal and current job attached to a listing (for listing attributes).
pub async fn listing_context(st: &AppState, l: &Listing) -> ApiResult<(Option<Deal>, Option<Job>, Option<Drive>)> {
    let deal = match &l.deal_id {
        Some(id) => st.db.deal(id).await?.map(|r| r.data),
        None => None,
    };
    let job = match deal.as_ref().and_then(|d| d.job_id.clone()) {
        Some(j) => st.db.job(&j).await?.map(|r| r.data),
        None => None,
    };
    let drive = match &l.drive_id {
        Some(d) => st.db.drive(d).await?.map(|r| r.data),
        None => None,
    };
    Ok((deal, job, drive))
}
