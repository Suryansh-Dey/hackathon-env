//! `kabados-authz`: Cedar authorization (PLAN.md §18).
//!
//! Policies and the schema live in `policies/` at the repository root and are
//! the single source of truth. Two engines evaluate them:
//! - [`LocalCedar`]: the embedded `cedar-policy` engine (local dev, CI policy
//!   tests, and a fallback);
//! - [`VerifiedPermissions`]: Amazon Verified Permissions (deployed envs).
//!
//! Both build their entities with [`entities_for`], so they cannot drift.
//! Time-dependent facts (KYC, authorization expiry, suspension, deal and job
//! state) are resolved into plain attributes by the caller at request time.

mod avp;
mod entities;
mod local;

pub use avp::{AvpRequest, VerifiedPermissions};
pub use entities::{EntityRef, entities_for};
pub use local::LocalCedar;

pub use kabados_domain::deal::DealStatus;
pub use kabados_domain::drive::DriveStatus;
pub use kabados_domain::ids::{DealId, DriveId, JobId, ListingId, OfferId, ThreadId, UserId};
pub use kabados_domain::job::JobStatus;
pub use kabados_domain::listing::ListingStatus;
pub use kabados_domain::offer::OfferStatus;

use std::str::FromStr;

/// Cedar namespace of every entity type and action.
pub const NAMESPACE: &str = "Kabados";
/// Entity ID of the singleton `Kabados::Platform` resource.
pub const PLATFORM_ID: &str = "kabados";

const SCHEMA_SRC: &str = include_str!("../../../policies/schema.cedarschema");

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/policies.rs"));
}

#[derive(Clone, Debug, PartialEq)]
pub struct VendorAttrs {
    pub id: UserId,
    pub kyc_approved: bool,
    pub hazardous_auth_valid: bool,
    pub suspended: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TransporterAttrs {
    pub id: UserId,
    pub kyc_approved: bool,
    pub suspended: bool,
}

/// The caller, typed by the role they are acting in.
#[derive(Clone, Debug, PartialEq)]
pub enum Principal {
    Seller(UserId),
    Vendor(VendorAttrs),
    Transporter(TransporterAttrs),
    SocietyAdmin(UserId),
    /// Staff user-pool subject; `mfa` is true only for MFA-authenticated sessions.
    Ops {
        id: String,
        mfa: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ListingAttrs {
    pub id: ListingId,
    pub seller: UserId,
    pub status: ListingStatus,
    pub contains_hazardous: bool,
    pub accepted_vendor: Option<UserId>,
    pub deal_status: Option<DealStatus>,
    pub assigned_transporter: Option<UserId>,
    pub job_status: Option<JobStatus>,
    pub drive_admin: Option<UserId>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DealAttrs {
    pub id: DealId,
    pub seller: UserId,
    pub vendor: UserId,
    pub transporter: Option<UserId>,
    pub status: DealStatus,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OfferAttrs {
    pub id: OfferId,
    pub listing: ListingAttrs,
    pub vendor: UserId,
    pub status: OfferStatus,
}

#[derive(Clone, Debug, PartialEq)]
pub struct JobAttrs {
    pub id: JobId,
    pub deal: DealAttrs,
    pub transporter: Option<UserId>,
    pub offered_to: Vec<UserId>,
    pub status: JobStatus,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ThreadAttrs {
    pub id: ThreadId,
    pub seller: UserId,
    pub vendor: UserId,
    pub listing_status: ListingStatus,
    pub deal_open: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DriveAttrs {
    pub id: DriveId,
    pub admin: UserId,
    pub status: DriveStatus,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Resource {
    Listing(ListingAttrs),
    Deal(DealAttrs),
    Offer(OfferAttrs),
    Job(JobAttrs),
    Thread(ThreadAttrs),
    Drive(DriveAttrs),
    /// The platform itself (ops-console actions).
    Platform,
}

macro_rules! actions {
    ($($name:ident),* $(,)?) => {
        /// Every Cedar action (`Kabados::Action::"<name>"`).
        #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
        pub enum Action { $($name),* }

        impl Action {
            pub const ALL: &'static [Action] = &[$(Action::$name),*];

            /// Cedar action ID, e.g. `"ViewListing"`.
            pub fn as_str(&self) -> &'static str {
                match self { $(Action::$name => stringify!($name)),* }
            }
        }
    };
}

actions!(
    ViewListing,
    EditListing,
    PublishListing,
    WithdrawListing,
    ViewExactAddress,
    MakeOffer,
    ViewOffers,
    AcceptOffer,
    CounterOffer,
    RejectOffer,
    WithdrawOffer,
    ViewDeal,
    RecordWeighing,
    RegradeLine,
    ConfirmWeights,
    RequestPayment,
    MarkPaidDirect,
    ConfirmReceived,
    Handover,
    ConfirmDelivery,
    CancelDeal,
    OpenDispute,
    RateDeal,
    StartCall,
    AssignTransporter,
    ViewReceipt,
    ViewJob,
    AcceptJob,
    DeclineJob,
    UpdateJob,
    PostJobPosition,
    ViewThread,
    PostMessage,
    ManageDrive,
    JoinDrive,
    ViewDriveReport,
    OpsReviewKyc,
    OpsReviewAi,
    OpsResolveDispute,
    OpsManageTaxonomy,
    OpsManageConfig,
    OpsViewFraud,
    OpsPublishRateIndex,
);

impl FromStr for Action {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Action::ALL.iter().copied().find(|a| a.as_str() == s).ok_or_else(|| anyhow::anyhow!("unknown action {s:?}"))
    }
}

/// Outcome of an authorization request.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Decision {
    pub allowed: bool,
    /// Policies that determined the decision (policy IDs locally, store IDs in AVP).
    pub determining_policies: Vec<String>,
    /// Evaluation or request-validation errors; any error means deny.
    pub errors: Vec<String>,
}

impl Decision {
    pub fn deny(error: impl Into<String>) -> Self {
        Decision { allowed: false, determining_policies: vec![], errors: vec![error.into()] }
    }
}

#[async_trait::async_trait]
pub trait Authorizer: Send + Sync {
    async fn is_authorized(&self, principal: &Principal, action: Action, resource: &Resource) -> anyhow::Result<Decision>;

    /// Many checks in one call (feed pages check many listings at once).
    async fn batch_is_authorized(&self, reqs: &[(Principal, Action, Resource)]) -> anyhow::Result<Vec<Decision>> {
        let mut out = Vec::with_capacity(reqs.len());
        for (p, a, r) in reqs {
            out.push(self.is_authorized(p, *a, r).await?);
        }
        Ok(out)
    }
}

/// The Cedar schema in human-readable syntax.
pub fn schema_source() -> &'static str {
    SCHEMA_SRC
}

/// `(policy id, policy text)` for every policy; one Cedar policy per entry, as
/// Verified Permissions stores policies individually.
pub fn policy_sources() -> Vec<(&'static str, &'static str)> {
    embedded::POLICIES.to_vec()
}

/// The schema in canonical Cedar JSON format (what Verified Permissions'
/// `PutSchema` expects). Type references are resolved to `String`, `Long`,
/// `Boolean` or `Entity` instead of Cedar 4's `EntityOrCommon` placeholders,
/// which the schema does not need since it declares no common types.
pub fn schema_json() -> String {
    let (fragment, _) = cedar_policy::SchemaFragment::from_cedarschema_str(SCHEMA_SRC).expect("embedded schema parses");
    let mut value = fragment.to_json_value().expect("schema serializes");
    resolve_type_refs(&mut value);
    value.to_string()
}

fn resolve_type_refs(v: &mut serde_json::Value) {
    use serde_json::{Value, json};
    match v {
        Value::Object(map) => {
            assert!(!map.contains_key("commonTypes"), "schema_json assumes no common types");
            if map.get("type").and_then(Value::as_str) == Some("EntityOrCommon") {
                let name = map.get("name").and_then(Value::as_str).unwrap_or_default().to_owned();
                let resolved = match name.as_str() {
                    "String" | "__cedar::String" => json!({ "type": "String" }),
                    "Long" | "__cedar::Long" => json!({ "type": "Long" }),
                    "Bool" | "Boolean" | "__cedar::Bool" => json!({ "type": "Boolean" }),
                    _ => json!({ "type": "Entity", "name": name }),
                };
                map.remove("name");
                map.remove("type");
                if let Value::Object(r) = resolved {
                    map.extend(r);
                }
            }
            for child in map.values_mut() {
                resolve_type_refs(child);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(resolve_type_refs),
        _ => {}
    }
}

/// Parsed schema.
pub fn schema() -> anyhow::Result<cedar_policy::Schema> {
    let (schema, _) = cedar_policy::Schema::from_cedarschema_str(SCHEMA_SRC).map_err(|e| anyhow::anyhow!("schema: {e}"))?;
    Ok(schema)
}

/// Every embedded policy, each keyed by its file name.
pub fn policy_set() -> anyhow::Result<cedar_policy::PolicySet> {
    let mut set = cedar_policy::PolicySet::new();
    for (id, src) in embedded::POLICIES {
        let policy =
            cedar_policy::Policy::parse(Some(cedar_policy::PolicyId::new(*id)), src).map_err(|e| anyhow::anyhow!("policy {id}: {e}"))?;
        set.add(policy).map_err(|e| anyhow::anyhow!("policy {id}: {e}"))?;
    }
    Ok(set)
}

/// Strictly validates every policy against the schema.
pub fn validate_policies() -> anyhow::Result<()> {
    let validator = cedar_policy::Validator::new(schema()?);
    let result = validator.validate(&policy_set()?, cedar_policy::ValidationMode::Strict);
    if result.validation_passed() {
        Ok(())
    } else {
        let errors: Vec<String> = result.validation_errors().map(|e| e.to_string()).collect();
        anyhow::bail!("policy validation failed:\n{}", errors.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policies_validate_strictly_against_schema() {
        validate_policies().unwrap();
    }

    #[test]
    fn every_action_is_declared_in_the_schema() {
        let schema = schema().unwrap();
        let declared: Vec<String> = schema.actions().map(|a| a.id().unescaped().to_string()).collect();
        for a in Action::ALL {
            assert!(declared.iter().any(|d| d == a.as_str()), "{} missing from schema", a.as_str());
        }
        assert_eq!(declared.len(), Action::ALL.len(), "schema declares actions the crate does not know: {declared:?}");
    }

    #[test]
    fn policy_ids_are_unique_and_one_policy_per_file() {
        let sources = policy_sources();
        assert!(sources.len() >= 40);
        for (id, src) in &sources {
            let set = cedar_policy::PolicySet::from_str(src).unwrap_or_else(|e| panic!("{id}: {e}"));
            assert_eq!(set.policies().count(), 1, "{id} must contain exactly one policy");
        }
        let mut ids: Vec<_> = sources.iter().map(|(id, _)| *id).collect();
        ids.dedup();
        assert_eq!(ids.len(), sources.len());
    }

    #[test]
    fn schema_json_is_canonical_and_equivalent() {
        let json = schema_json();
        assert!(!json.contains("EntityOrCommon"), "{json}");
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(value.get(NAMESPACE).is_some(), "{json}");
        let from_json = cedar_policy::Schema::from_json_value(value).unwrap();
        let result = cedar_policy::Validator::new(from_json).validate(&policy_set().unwrap(), cedar_policy::ValidationMode::Strict);
        assert!(result.validation_passed(), "{:?}", result.validation_errors().map(|e| e.to_string()).collect::<Vec<_>>());
    }

    #[test]
    fn committed_json_schema_is_current() {
        let committed: serde_json::Value = serde_json::from_str(include_str!("../../../policies/schema.cedarschema.json")).unwrap();
        let fresh: serde_json::Value = serde_json::from_str(&schema_json()).unwrap();
        assert_eq!(committed, fresh, "run `cargo run -p kabados-authz --example export_schema`");
    }

    #[test]
    fn actions_parse() {
        for a in Action::ALL {
            assert_eq!(a.as_str().parse::<Action>().unwrap(), *a);
        }
        assert!("Nope".parse::<Action>().is_err());
    }
}
