//! Embedded Cedar engine over the policies compiled into this crate.

use crate::{Action, Authorizer, Decision, Principal, Resource, entities_for};
use cedar_policy::{Context, Entities, PolicySet, Request, Schema};

/// Evaluates the embedded `policies/` with the `cedar-policy` crate. Requests
/// and entities are validated against the schema, so a principal type an
/// action does not apply to is denied (with the reason in `errors`).
pub struct LocalCedar {
    schema: Schema,
    policies: PolicySet,
    engine: cedar_policy::Authorizer,
}

impl LocalCedar {
    pub fn new() -> anyhow::Result<Self> {
        crate::validate_policies()?;
        Ok(Self { schema: crate::schema()?, policies: crate::policy_set()?, engine: cedar_policy::Authorizer::new() })
    }

    /// Synchronous evaluation (the trait methods wrap this).
    pub fn decide(&self, principal: &Principal, action: Action, resource: &Resource) -> anyhow::Result<Decision> {
        let entities = Entities::from_json_value(entities_for(principal, resource), Some(&self.schema))
            .map_err(|e| anyhow::anyhow!("entities for {} on {:?}: {e}", action.as_str(), resource.entity_ref()))?;
        let request = match Request::new(
            principal.entity_ref().to_cedar()?,
            action.entity_ref().to_cedar()?,
            resource.entity_ref().to_cedar()?,
            Context::empty(),
            Some(&self.schema),
        ) {
            Ok(r) => r,
            Err(e) => return Ok(Decision::deny(e.to_string())),
        };
        let response = self.engine.is_authorized(&request, &self.policies, &entities);
        let diagnostics = response.diagnostics();
        Ok(Decision {
            allowed: response.decision() == cedar_policy::Decision::Allow,
            determining_policies: diagnostics.reason().map(|p| p.to_string()).collect(),
            errors: diagnostics.errors().map(|e| e.to_string()).collect(),
        })
    }
}

#[async_trait::async_trait]
impl Authorizer for LocalCedar {
    async fn is_authorized(&self, principal: &Principal, action: Action, resource: &Resource) -> anyhow::Result<Decision> {
        self.decide(principal, action, resource)
    }

    async fn batch_is_authorized(&self, reqs: &[(Principal, Action, Resource)]) -> anyhow::Result<Vec<Decision>> {
        reqs.iter().map(|(p, a, r)| self.decide(p, *a, r)).collect()
    }
}
