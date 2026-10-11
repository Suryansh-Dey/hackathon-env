//! KabadOS data access (PLAN.md §16).
//!
//! A small storage port ([`Store`]) with two backends: DynamoDB
//! ([`dynamo::DynamoStore`]) and an in-memory emulation ([`memory::MemoryStore`])
//! that honours keys, GSIs, condition expressions, transactions and emits
//! Streams-like change records. Key formats live only in [`keys`].

pub mod conformance;
pub mod dynamo;
pub mod keys;
pub mod memory;
pub mod record;
pub mod repo;
pub mod schema;
pub mod store;

pub use record::{Meta, Record};
pub use store::*;
