//! Amazon Verified Permissions client (PLAN.md §18.3).
//!
//! Entities are sent as Cedar JSON produced by [`entities_for`], the same
//! document [`crate::LocalCedar`] evaluates, so the two engines see identical
//! inputs. Requests are first checked against the embedded schema so that a
//! principal type an action does not apply to is denied the same way locally
//! and remotely, without a network call.

use crate::{Action, Authorizer, Decision, Principal, Resource, entities_for};
use aws_sdk_verifiedpermissions::types::{
    ActionIdentifier, BatchIsAuthorizedInputItem, Decision as AvpDecision, EntitiesDefinition, EntityIdentifier,
};
use cedar_policy::{Context, Request, Schema};
use serde_json::Value;
use std::collections::BTreeMap;

/// AVP limits `BatchIsAuthorized` to 30 requests.
pub const MAX_BATCH: usize = 30;

/// One `IsAuthorized` request, built without any AWS call (unit-testable).
#[derive(Clone, Debug, PartialEq)]
pub struct AvpRequest {
    pub principal: EntityIdentifier,
    pub action: ActionIdentifier,
    pub resource: EntityIdentifier,
    /// Cedar entities JSON (`EntitiesDefinition::CedarJson`).
    pub entities: Value,
}

fn identifier(r: &crate::EntityRef) -> anyhow::Result<EntityIdentifier> {
    Ok(EntityIdentifier::builder().entity_type(r.type_name).entity_id(r.id.clone()).build()?)
}

impl AvpRequest {
    pub fn build(principal: &Principal, action: Action, resource: &Resource) -> anyhow::Result<Self> {
        let a = action.entity_ref();
        Ok(Self {
            principal: identifier(&principal.entity_ref())?,
            action: ActionIdentifier::builder().action_type(a.type_name).action_id(a.id).build()?,
            resource: identifier(&resource.entity_ref())?,
            entities: entities_for(principal, resource),
        })
    }
}

pub struct VerifiedPermissions {
    client: aws_sdk_verifiedpermissions::Client,
    policy_store_id: String,
    schema: Schema,
}

impl VerifiedPermissions {
    pub fn new(client: aws_sdk_verifiedpermissions::Client, policy_store_id: impl Into<String>) -> Self {
        Self { client, policy_store_id: policy_store_id.into(), schema: crate::schema().expect("embedded schema is valid") }
    }

    pub fn policy_store_id(&self) -> &str {
        &self.policy_store_id
    }

    /// Denies requests the schema rules out (e.g. a transporter accepting an offer).
    pub fn precheck(&self, principal: &Principal, action: Action, resource: &Resource) -> anyhow::Result<Option<Decision>> {
        match Request::new(
            principal.entity_ref().to_cedar()?,
            action.entity_ref().to_cedar()?,
            resource.entity_ref().to_cedar()?,
            Context::empty(),
            Some(&self.schema),
        ) {
            Ok(_) => Ok(None),
            Err(e) => Ok(Some(Decision::deny(e.to_string()))),
        }
    }
}

fn decision(decision: &AvpDecision, policies: Vec<String>, errors: Vec<String>) -> Decision {
    Decision { allowed: *decision == AvpDecision::Allow, determining_policies: policies, errors }
}

/// Groups request indices into AVP batches: one principal per batch (AVP
/// requires a shared principal or resource), at most [`MAX_BATCH`] requests,
/// and no entity UID with conflicting attributes inside one batch.
pub fn plan_batches(reqs: &[(Principal, Action, Resource)]) -> Vec<Vec<usize>> {
    let mut by_principal: BTreeMap<crate::EntityRef, Vec<usize>> = BTreeMap::new();
    for (i, (p, _, _)) in reqs.iter().enumerate() {
        by_principal.entry(p.entity_ref()).or_default().push(i);
    }
    let mut batches = Vec::new();
    for indices in by_principal.into_values() {
        let mut current: Vec<usize> = Vec::new();
        let mut seen: BTreeMap<String, Value> = BTreeMap::new();
        for i in indices {
            let (p, _, r) = &reqs[i];
            let ents = entities_for(p, r);
            let conflict = ents.as_array().into_iter().flatten().any(|e| seen.get(&e["uid"].to_string()).is_some_and(|prev| prev != e));
            if current.len() == MAX_BATCH || conflict {
                batches.push(std::mem::take(&mut current));
                seen.clear();
            }
            for e in ents.as_array().into_iter().flatten() {
                seen.insert(e["uid"].to_string(), e.clone());
            }
            current.push(i);
        }
        if !current.is_empty() {
            batches.push(current);
        }
    }
    batches
}

fn merged_entities(reqs: &[(Principal, Action, Resource)], batch: &[usize]) -> Value {
    let mut seen: BTreeMap<String, Value> = BTreeMap::new();
    for &i in batch {
        let (p, _, r) = &reqs[i];
        for e in entities_for(p, r).as_array().into_iter().flatten() {
            seen.entry(e["uid"].to_string()).or_insert_with(|| e.clone());
        }
    }
    Value::Array(seen.into_values().collect())
}

#[async_trait::async_trait]
impl Authorizer for VerifiedPermissions {
    async fn is_authorized(&self, principal: &Principal, action: Action, resource: &Resource) -> anyhow::Result<Decision> {
        if let Some(denied) = self.precheck(principal, action, resource)? {
            return Ok(denied);
        }
        let req = AvpRequest::build(principal, action, resource)?;
        let out = self
            .client
            .is_authorized()
            .policy_store_id(&self.policy_store_id)
            .principal(req.principal)
            .action(req.action)
            .resource(req.resource)
            .entities(EntitiesDefinition::CedarJson(req.entities.to_string()))
            .send()
            .await
            .map_err(|e| {
                anyhow::anyhow!("verifiedpermissions:IsAuthorized: {}", aws_sdk_verifiedpermissions::error::DisplayErrorContext(e))
            })?;
        Ok(decision(
            out.decision(),
            out.determining_policies().iter().map(|p| p.policy_id().to_string()).collect(),
            out.errors().iter().map(|e| e.error_description().to_string()).collect(),
        ))
    }

    async fn batch_is_authorized(&self, reqs: &[(Principal, Action, Resource)]) -> anyhow::Result<Vec<Decision>> {
        let mut results: Vec<Option<Decision>> = vec![None; reqs.len()];
        let mut pending = Vec::new();
        for (i, (p, a, r)) in reqs.iter().enumerate() {
            match self.precheck(p, *a, r)? {
                Some(denied) => results[i] = Some(denied),
                None => pending.push((i, (p.clone(), *a, r.clone()))),
            }
        }
        let pending_reqs: Vec<(Principal, Action, Resource)> = pending.iter().map(|(_, r)| r.clone()).collect();
        for batch in plan_batches(&pending_reqs) {
            let mut items = Vec::with_capacity(batch.len());
            for &j in &batch {
                let (p, a, r) = &pending_reqs[j];
                let req = AvpRequest::build(p, *a, r)?;
                items
                    .push(BatchIsAuthorizedInputItem::builder().principal(req.principal).action(req.action).resource(req.resource).build());
            }
            let out = self
                .client
                .batch_is_authorized()
                .policy_store_id(&self.policy_store_id)
                .set_requests(Some(items))
                .entities(EntitiesDefinition::CedarJson(merged_entities(&pending_reqs, &batch).to_string()))
                .send()
                .await
                .map_err(|e| {
                    anyhow::anyhow!("verifiedpermissions:BatchIsAuthorized: {}", aws_sdk_verifiedpermissions::error::DisplayErrorContext(e))
                })?;
            anyhow::ensure!(
                out.results().len() == batch.len(),
                "BatchIsAuthorized returned {} results for {} requests",
                out.results().len(),
                batch.len()
            );
            // Results come back in request order.
            for (&j, res) in batch.iter().zip(out.results()) {
                results[pending[j].0] = Some(decision(
                    res.decision(),
                    res.determining_policies().iter().map(|p| p.policy_id().to_string()).collect(),
                    res.errors().iter().map(|e| e.error_description().to_string()).collect(),
                ));
            }
        }
        results.into_iter().map(|d| d.ok_or_else(|| anyhow::anyhow!("missing batch decision"))).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        DealAttrs, DealId, DealStatus, ListingAttrs, ListingId, ListingStatus, OfferAttrs, OfferId, OfferStatus, UserId, VendorAttrs,
    };

    fn offline_client() -> aws_sdk_verifiedpermissions::Client {
        let conf = aws_sdk_verifiedpermissions::Config::builder()
            .behavior_version(aws_sdk_verifiedpermissions::config::BehaviorVersion::latest())
            .region(aws_sdk_verifiedpermissions::config::Region::new("ap-south-1"))
            .build();
        aws_sdk_verifiedpermissions::Client::from_conf(conf)
    }

    fn listing(seller: &UserId) -> ListingAttrs {
        ListingAttrs {
            id: ListingId::new(),
            seller: seller.clone(),
            status: ListingStatus::Published,
            contains_hazardous: false,
            accepted_vendor: None,
            deal_status: None,
            assigned_transporter: None,
            job_status: None,
            drive_admin: None,
        }
    }

    fn vendor(id: &UserId) -> Principal {
        Principal::Vendor(VendorAttrs { id: id.clone(), kyc_approved: true, hazardous_auth_valid: false, suspended: false })
    }

    #[test]
    fn builds_requests_with_shared_entities() {
        let seller = UserId::new();
        let offer = OfferAttrs { id: OfferId::new(), listing: listing(&seller), vendor: UserId::new(), status: OfferStatus::Active };
        let p = Principal::Seller(seller.clone());
        let r = Resource::Offer(offer.clone());
        let req = AvpRequest::build(&p, Action::AcceptOffer, &r).unwrap();
        assert_eq!(req.principal.entity_type(), "Kabados::Seller");
        assert_eq!(req.principal.entity_id(), seller.as_str());
        assert_eq!(req.action.action_type(), "Kabados::Action");
        assert_eq!(req.action.action_id(), "AcceptOffer");
        assert_eq!(req.resource.entity_type(), "Kabados::Offer");
        assert_eq!(req.resource.entity_id(), offer.id.as_str());
        assert_eq!(req.entities, entities_for(&p, &r));
        // principal + offer + the offer's listing
        assert_eq!(req.entities.as_array().unwrap().len(), 3);
    }

    #[test]
    fn precheck_denies_schema_invalid_requests_offline() {
        let avp = VerifiedPermissions::new(offline_client(), "ps-test");
        let seller = UserId::new();
        let offer = Resource::Offer(OfferAttrs {
            id: OfferId::new(),
            listing: listing(&seller),
            vendor: UserId::new(),
            status: OfferStatus::Active,
        });
        let t = Principal::Transporter(crate::TransporterAttrs { id: UserId::new(), kyc_approved: true, suspended: false });
        let d = avp.precheck(&t, Action::AcceptOffer, &offer).unwrap().expect("denied before any network call");
        assert!(!d.allowed && !d.errors.is_empty());
        assert!(avp.precheck(&Principal::Seller(seller), Action::AcceptOffer, &offer).unwrap().is_none());
        assert_eq!(avp.policy_store_id(), "ps-test");
    }

    #[test]
    fn batches_group_by_principal_and_respect_limit() {
        let v1 = UserId::new();
        let v2 = UserId::new();
        let mut reqs = Vec::new();
        for _ in 0..(MAX_BATCH + 5) {
            reqs.push((vendor(&v1), Action::ViewListing, Resource::Listing(listing(&UserId::new()))));
        }
        reqs.push((vendor(&v2), Action::ViewListing, Resource::Listing(listing(&UserId::new()))));
        let batches = plan_batches(&reqs);
        assert_eq!(batches.iter().map(Vec::len).sum::<usize>(), reqs.len());
        assert!(batches.iter().all(|b| b.len() <= MAX_BATCH));
        for b in &batches {
            let p0 = reqs[b[0]].0.entity_ref();
            assert!(b.iter().all(|&i| reqs[i].0.entity_ref() == p0), "one principal per batch");
        }
        assert_eq!(batches.len(), 3);
    }

    #[test]
    fn conflicting_entities_split_batches() {
        let v = UserId::new();
        let seller = UserId::new();
        let deal_id = DealId::new();
        let deal = |status| {
            Resource::Deal(DealAttrs { id: deal_id.clone(), seller: seller.clone(), vendor: v.clone(), transporter: None, status })
        };
        let reqs =
            vec![(vendor(&v), Action::ViewDeal, deal(DealStatus::Created)), (vendor(&v), Action::ViewDeal, deal(DealStatus::Weighed))];
        assert_eq!(plan_batches(&reqs).len(), 2);
        let same =
            vec![(vendor(&v), Action::ViewDeal, deal(DealStatus::Created)), (vendor(&v), Action::CancelDeal, deal(DealStatus::Created))];
        let batches = plan_batches(&same);
        assert_eq!(batches.len(), 1);
        assert_eq!(merged_entities(&same, &batches[0]).as_array().unwrap().len(), 2);
    }
}
