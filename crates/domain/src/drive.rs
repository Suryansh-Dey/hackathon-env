//! Pooled collection drives (PLAN.md §9).

use crate::Grams;
use crate::geo::LatLng;
use crate::ids::*;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DriveStatus {
    Open,
    Closed,
    Completed,
    Cancelled,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Recurrence {
    None,
    Weekly,
    Monthly,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct Drive {
    pub id: DriveId,
    pub admin_id: UserId,
    pub name: String,
    #[schema(value_type = String, format = Date)]
    pub date: NaiveDate,
    pub window: String,
    pub collection_point: LatLng,
    pub collection_note: Option<String>,
    pub h3_r7: String,
    pub h3_r8: String,
    pub city: String,
    pub categories: Vec<String>,
    pub status: DriveStatus,
    pub recurrence: Recurrence,
    /// Join code embedded in the link/QR.
    pub join_code: String,
    pub member_count: u32,
    pub pool_weight: Grams,
    pub listing_id: Option<ListingId>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct DriveMember {
    pub drive_id: DriveId,
    pub listing_id: ListingId,
    pub seller_id: UserId,
    pub est_weight: Grams,
    pub joined_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct DriveReport {
    pub drive_id: DriveId,
    pub participants: u32,
    pub kg_by_category: Vec<(String, Grams)>,
    pub hazardous_to_authorized: Grams,
    pub total_weight: Grams,
}
