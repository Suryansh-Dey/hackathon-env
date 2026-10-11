//! Caller identity. On Lambda, API Gateway's JWT authorizer has already
//! validated the Cognito access token; we read its claims from the request
//! context. Locally, `Bearer dev.<base64url claims>` tokens are accepted.

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::Utc;
use kabados_db::Record;
use kabados_domain::ids::UserId;
use kabados_domain::user::{Role, UserProfile, normalize_in_phone};
use std::collections::{BTreeSet, HashMap};

#[derive(Clone, Debug, PartialEq)]
pub struct Caller {
    pub sub: String,
    /// Cognito username (the E.164 phone number for customer accounts).
    pub username: String,
    pub phone: Option<String>,
    pub groups: Vec<String>,
    /// Token from the staff pool (mandatory MFA, PLAN.md §10.1).
    pub staff: bool,
}

impl Caller {
    pub fn from_claims(claims: &HashMap<String, String>, staff_pool_id: Option<&str>) -> Option<Caller> {
        let sub = claims.get("sub")?.clone();
        let username = claims.get("username").or_else(|| claims.get("cognito:username")).cloned().unwrap_or_default();
        let phone = claims.get("phone_number").cloned().or_else(|| normalize_in_phone(&username));
        let groups = claims
            .get("cognito:groups")
            .map(|g| {
                g.trim_matches(['[', ']']).split([',', ' ']).filter(|s| !s.is_empty()).map(|s| s.trim_matches('"').to_owned()).collect()
            })
            .unwrap_or_default();
        let iss = claims.get("iss").map(String::as_str).unwrap_or_default();
        let staff = staff_pool_id.is_some_and(|p| iss.ends_with(p)) || claims.get("kabados:staff").is_some_and(|v| v == "true");
        Some(Caller { sub, username, phone, groups, staff })
    }

    /// Creates a local development token for these claims.
    pub fn dev_token(sub: &str, phone: &str, staff: bool) -> String {
        let claims = serde_json::json!({"sub": sub, "username": phone, "kabados:staff": staff.to_string()});
        format!("dev.{}", URL_SAFE_NO_PAD.encode(claims.to_string()))
    }
}

fn lambda_claims(parts: &Parts) -> Option<HashMap<String, String>> {
    use lambda_http::request::RequestContext;
    match parts.extensions.get::<RequestContext>()? {
        RequestContext::ApiGatewayV2(ctx) => ctx.authorizer.as_ref()?.jwt.as_ref().map(|j| j.claims.clone()),
        _ => None,
    }
}

fn dev_claims(parts: &Parts) -> Option<HashMap<String, String>> {
    let h = parts.headers.get(axum::http::header::AUTHORIZATION)?.to_str().ok()?;
    let token = h.strip_prefix("Bearer ")?.strip_prefix("dev.")?;
    let v: serde_json::Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(token).ok()?).ok()?;
    Some(v.as_object()?.iter().map(|(k, v)| (k.clone(), v.as_str().map(str::to_owned).unwrap_or_else(|| v.to_string()))).collect())
}

/// Extracts the caller's claims (used by the idempotency layer as its scope).
pub fn caller_from_parts(parts: &Parts, st: &AppState) -> Option<Caller> {
    let claims = lambda_claims(parts).or_else(|| if st.cfg.is_local() { dev_claims(parts) } else { None })?;
    Caller::from_claims(&claims, st.cfg.staff_user_pool_id.as_deref())
}

impl FromRequestParts<AppState> for Caller {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, st: &AppState) -> Result<Self, Self::Rejection> {
        caller_from_parts(parts, st).ok_or(ApiError::Unauthorized)
    }
}

/// The signed-in customer with their profile (created on first use).
#[derive(Clone, Debug)]
pub struct Me {
    pub caller: Caller,
    pub user: Record<UserProfile>,
}

impl Me {
    pub fn id(&self) -> &UserId {
        &self.user.data.id
    }

    pub fn has_role(&self, r: Role) -> bool {
        self.user.data.roles.contains(&r)
    }
}

impl FromRequestParts<AppState> for Me {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, st: &AppState) -> Result<Self, Self::Rejection> {
        let caller = Caller::from_request_parts(parts, st).await?;
        if caller.staff {
            return Err(ApiError::forbidden("staff accounts can't use customer endpoints"));
        }
        let user = load_or_create(st, &caller).await?;
        if user.data.suspended {
            return Err(ApiError::forbidden("this account is suspended"));
        }
        Ok(Me { caller, user })
    }
}

/// Resolves the profile by Cognito `sub`, provisioning it on first sign-in.
pub async fn load_or_create(st: &AppState, caller: &Caller) -> ApiResult<Record<UserProfile>> {
    if let Some(u) = st.db.user_by_sub(&caller.sub).await? {
        return Ok(u);
    }
    let phone = caller.phone.clone().ok_or_else(|| ApiError::bad("account has no phone number"))?;
    let hash = st.hasher.phone(&phone);
    if st.db.user_by_phone_hash(&hash).await?.is_some() {
        return Err(ApiError::conflict("this phone number is linked to another account"));
    }
    let profile = UserProfile {
        id: UserId::new(),
        cognito_sub: caller.sub.clone(),
        phone_hash: hash,
        phone_last4: phone.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect(),
        name: None,
        roles: BTreeSet::from([Role::Seller]),
        adult_confirmed: false,
        suspended: false,
        honesty: Default::default(),
        created_at: Utc::now(),
    };
    match st.db.create_user(profile).await {
        Ok(r) => Ok(r),
        Err(e) if e.is_conflict() => st.db.user_by_sub(&caller.sub).await?.ok_or_else(|| ApiError::conflict("account setup raced; retry")),
        Err(e) => Err(e.into()),
    }
}

/// An ops agent (staff pool token).
#[derive(Clone, Debug)]
pub struct Staff(pub Caller);

impl FromRequestParts<AppState> for Staff {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, st: &AppState) -> Result<Self, Self::Rejection> {
        let c = Caller::from_request_parts(parts, st).await?;
        if !c.staff {
            return Err(ApiError::forbidden("ops console access only"));
        }
        Ok(Staff(c))
    }
}
