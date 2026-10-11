//! KabadOS domain model (PLAN.md §4, §12, §16).
//!
//! Pure types and rules with no I/O. Shared by the Rust API, workers, AI agents
//! and the transporter app so that states and money math never drift.

pub mod api;
pub mod chat;
pub mod deal;
pub mod drive;
pub mod error;
pub mod events;
pub mod geo;
pub mod ids;
pub mod job;
pub mod ladder;
pub mod ledger;
pub mod listing;
pub mod money;
pub mod offer;
pub mod redact;
pub mod ruleset;
pub mod score;
pub mod state;
pub mod taxonomy;
pub mod user;

pub use error::DomainError;
pub use ids::*;
pub use ladder::LadderLevel;
pub use money::{Grams, Paise};

/// Current schema version written on every stored item (PLAN.md §16.2).
pub const SCHEMA_V: u32 = 1;
