//! Entity loader shared by both engines: turns request attributes into Cedar
//! entity JSON (PLAN.md §18.1).

use crate::{Action, DealAttrs, DriveStatus, ListingAttrs, PLATFORM_ID, Principal, Resource};
use serde_json::{Map, Value, json};

pub(crate) const SELLER: &str = "Kabados::Seller";
pub(crate) const VENDOR: &str = "Kabados::Vendor";
pub(crate) const TRANSPORTER: &str = "Kabados::Transporter";
pub(crate) const SOCIETY_ADMIN: &str = "Kabados::SocietyAdmin";
pub(crate) const OPS_AGENT: &str = "Kabados::OpsAgent";
pub(crate) const PLATFORM: &str = "Kabados::Platform";
pub(crate) const LISTING: &str = "Kabados::Listing";
pub(crate) const DEAL: &str = "Kabados::Deal";
pub(crate) const OFFER: &str = "Kabados::Offer";
pub(crate) const JOB: &str = "Kabados::Job";
pub(crate) const THREAD: &str = "Kabados::Thread";
pub(crate) const DRIVE: &str = "Kabados::Drive";
pub(crate) const ACTION: &str = "Kabados::Action";

/// A Cedar entity UID: fully qualified type plus ID.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntityRef {
    pub type_name: &'static str,
    pub id: String,
}

impl EntityRef {
    pub fn new(type_name: &'static str, id: impl Into<String>) -> Self {
        Self { type_name, id: id.into() }
    }

    /// `{"type": …, "id": …}`.
    pub fn to_json(&self) -> Value {
        json!({ "type": self.type_name, "id": self.id })
    }

    /// Entity reference inside attributes: `{"__entity": {…}}`.
    fn as_attr(&self) -> Value {
        json!({ "__entity": self.to_json() })
    }

    pub fn to_cedar(&self) -> anyhow::Result<cedar_policy::EntityUid> {
        let ty: cedar_policy::EntityTypeName =
            self.type_name.parse().map_err(|e| anyhow::anyhow!("entity type {}: {e}", self.type_name))?;
        Ok(cedar_policy::EntityUid::from_type_name_and_id(ty, cedar_policy::EntityId::new(&self.id)))
    }
}

impl Action {
    pub fn entity_ref(&self) -> EntityRef {
        EntityRef::new(ACTION, self.as_str())
    }
}

impl Principal {
    pub fn entity_ref(&self) -> EntityRef {
        match self {
            Principal::Seller(id) => EntityRef::new(SELLER, id.as_str()),
            Principal::Vendor(v) => EntityRef::new(VENDOR, v.id.as_str()),
            Principal::Transporter(t) => EntityRef::new(TRANSPORTER, t.id.as_str()),
            Principal::SocietyAdmin(id) => EntityRef::new(SOCIETY_ADMIN, id.as_str()),
            Principal::Ops { id, .. } => EntityRef::new(OPS_AGENT, id.clone()),
        }
    }

    fn attrs(&self) -> Map<String, Value> {
        let v = match self {
            Principal::Seller(_) | Principal::SocietyAdmin(_) => json!({}),
            Principal::Vendor(v) => json!({
                "userId": v.id.as_str(),
                "kycApproved": v.kyc_approved,
                "hazardousAuthValid": v.hazardous_auth_valid,
                "suspended": v.suspended,
            }),
            Principal::Transporter(t) => json!({ "kycApproved": t.kyc_approved, "suspended": t.suspended }),
            Principal::Ops { mfa, .. } => json!({ "mfa": mfa }),
        };
        match v {
            Value::Object(m) => m,
            _ => Map::new(),
        }
    }
}

impl Resource {
    pub fn entity_ref(&self) -> EntityRef {
        match self {
            Resource::Listing(l) => EntityRef::new(LISTING, l.id.as_str()),
            Resource::Deal(d) => EntityRef::new(DEAL, d.id.as_str()),
            Resource::Offer(o) => EntityRef::new(OFFER, o.id.as_str()),
            Resource::Job(j) => EntityRef::new(JOB, j.id.as_str()),
            Resource::Thread(t) => EntityRef::new(THREAD, t.id.as_str()),
            Resource::Drive(d) => EntityRef::new(DRIVE, d.id.as_str()),
            Resource::Platform => EntityRef::new(PLATFORM, PLATFORM_ID),
        }
    }
}

fn entity(uid: &EntityRef, attrs: Map<String, Value>) -> Value {
    json!({ "uid": uid.to_json(), "attrs": attrs, "parents": [] })
}

fn listing_entity(l: &ListingAttrs) -> Value {
    let mut a = Map::new();
    a.insert("seller".into(), EntityRef::new(SELLER, l.seller.as_str()).as_attr());
    a.insert("sellerId".into(), json!(l.seller.as_str()));
    a.insert("status".into(), json!(l.status.as_str()));
    a.insert("containsHazardous".into(), json!(l.contains_hazardous));
    if let Some(v) = &l.accepted_vendor {
        a.insert("acceptedVendor".into(), EntityRef::new(VENDOR, v.as_str()).as_attr());
    }
    if let Some(s) = l.deal_status {
        a.insert("dealStatus".into(), json!(s.as_str()));
    }
    if let Some(t) = &l.assigned_transporter {
        a.insert("assignedTransporter".into(), EntityRef::new(TRANSPORTER, t.as_str()).as_attr());
    }
    if let Some(s) = l.job_status {
        a.insert("jobStatus".into(), json!(s.as_str()));
    }
    if let Some(d) = &l.drive_admin {
        a.insert("driveAdmin".into(), EntityRef::new(SOCIETY_ADMIN, d.as_str()).as_attr());
    }
    entity(&EntityRef::new(LISTING, l.id.as_str()), a)
}

fn deal_entity(d: &DealAttrs) -> Value {
    let mut a = Map::new();
    a.insert("seller".into(), EntityRef::new(SELLER, d.seller.as_str()).as_attr());
    a.insert("vendor".into(), EntityRef::new(VENDOR, d.vendor.as_str()).as_attr());
    if let Some(t) = &d.transporter {
        a.insert("transporter".into(), EntityRef::new(TRANSPORTER, t.as_str()).as_attr());
    }
    a.insert("status".into(), json!(d.status.as_str()));
    entity(&EntityRef::new(DEAL, d.id.as_str()), a)
}

fn drive_status(s: DriveStatus) -> String {
    serde_json::to_value(s).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
}

/// Cedar entities JSON for one request: the principal, the resource and the
/// entities the resource points to that policies dereference (an offer's
/// listing, a job's deal).
pub fn entities_for(principal: &Principal, resource: &Resource) -> Value {
    let mut out = vec![entity(&principal.entity_ref(), principal.attrs())];
    match resource {
        Resource::Listing(l) => out.push(listing_entity(l)),
        Resource::Deal(d) => out.push(deal_entity(d)),
        Resource::Offer(o) => {
            let mut a = Map::new();
            a.insert("listing".into(), EntityRef::new(LISTING, o.listing.id.as_str()).as_attr());
            a.insert("vendor".into(), EntityRef::new(VENDOR, o.vendor.as_str()).as_attr());
            a.insert("status".into(), json!(o.status.as_str()));
            out.push(entity(&resource.entity_ref(), a));
            out.push(listing_entity(&o.listing));
        }
        Resource::Job(j) => {
            let mut a = Map::new();
            a.insert("deal".into(), EntityRef::new(DEAL, j.deal.id.as_str()).as_attr());
            if let Some(t) = &j.transporter {
                a.insert("transporter".into(), EntityRef::new(TRANSPORTER, t.as_str()).as_attr());
            }
            let offered: Vec<Value> = j.offered_to.iter().map(|t| EntityRef::new(TRANSPORTER, t.as_str()).as_attr()).collect();
            a.insert("offeredTo".into(), Value::Array(offered));
            a.insert("status".into(), json!(j.status.as_str()));
            out.push(entity(&resource.entity_ref(), a));
            out.push(deal_entity(&j.deal));
        }
        Resource::Thread(t) => {
            let mut a = Map::new();
            a.insert("seller".into(), EntityRef::new(SELLER, t.seller.as_str()).as_attr());
            a.insert("vendor".into(), EntityRef::new(VENDOR, t.vendor.as_str()).as_attr());
            a.insert("listingStatus".into(), json!(t.listing_status.as_str()));
            a.insert("dealOpen".into(), json!(t.deal_open));
            out.push(entity(&resource.entity_ref(), a));
        }
        Resource::Drive(d) => {
            let mut a = Map::new();
            a.insert("admin".into(), EntityRef::new(SOCIETY_ADMIN, d.admin.as_str()).as_attr());
            a.insert("status".into(), json!(drive_status(d.status)));
            out.push(entity(&resource.entity_ref(), a));
        }
        Resource::Platform => out.push(entity(&resource.entity_ref(), Map::new())),
    }
    Value::Array(out)
}
