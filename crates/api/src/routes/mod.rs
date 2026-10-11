//! Routers per bounded context (PLAN.md §15.1).

pub mod calls;
pub mod chat;
pub mod deals;
pub mod dev;
pub mod drives;
pub mod identity;
pub mod listings;
pub mod logistics;
pub mod offers;
pub mod ops;
pub mod payments;
pub mod public;
pub mod vendor;

use crate::error::{ApiError, ApiResult};
use serde::Deserialize;
use std::str::FromStr;

/// Splits a custom-method segment like `lst_…:publish` (Google AIP style).
pub fn split_action(seg: &str) -> (&str, Option<&str>) {
    match seg.split_once(':') {
        Some((id, action)) => (id, Some(action)),
        None => (seg, None),
    }
}

pub fn parse<T: FromStr>(s: &str, what: &'static str) -> ApiResult<T> {
    s.parse().map_err(|_| ApiError::bad(format!("invalid {what}")))
}

/// Parses `<id>:<action>` and requires an action.
pub fn id_action<T: FromStr>(seg: &str, what: &'static str) -> ApiResult<(T, String)> {
    let (id, action) = split_action(seg);
    let action = action.ok_or_else(|| ApiError::bad("missing :action"))?;
    Ok((parse(id, what)?, action.to_owned()))
}

#[derive(Debug, Default, Deserialize, utoipa::IntoParams)]
pub struct PageQuery {
    pub cursor: Option<String>,
}

pub fn now() -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now()
}
