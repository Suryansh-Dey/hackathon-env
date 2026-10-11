//! Request and response bodies shared by the Rust API and the Rust client
//! (`kabados-client`, used by the transporter app and evals). The OpenAPI
//! document for the websites is generated from these types (PLAN.md §17.1).

use serde::{Deserialize, Serialize};

/// Cursor-paginated list response.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}

/// RFC 9457 problem details.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct Problem {
    #[serde(rename = "type")]
    pub type_: String,
    pub title: String,
    pub status: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<crate::ruleset::ValidationIssue>,
}
