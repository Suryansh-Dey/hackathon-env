//! Prefixed, time-ordered identifiers (UUIDv7, PLAN.md §16.2).
//!
//! Every ID is `<prefix>_<32 lowercase hex>`. Parsing validates the shape so
//! that untrusted path parameters can never inject key separators (`#`) into
//! DynamoDB keys.

use crate::DomainError;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

macro_rules! id_type {
    ($(#[$meta:meta])* $name:ident, $prefix:literal) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
        #[serde(try_from = "String", into = "String")]
        #[schema(value_type = String, example = json!(concat!($prefix, "_0192f5b8c1a77d3e9b1e0c2d4f6a8b0c")))]
        #[schemars(with = "String")]
        pub struct $name(String);

        impl $name {
            pub const PREFIX: &'static str = $prefix;

            /// Generates a new time-ordered ID.
            #[allow(clippy::new_without_default)]
            pub fn new() -> Self {
                Self(format!("{}_{}", $prefix, Uuid::now_v7().simple()))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// Parses and validates an ID string.
            pub fn parse(s: &str) -> Result<Self, DomainError> {
                if is_valid_id(s, $prefix) {
                    Ok(Self(s.to_owned()))
                } else {
                    Err(DomainError::InvalidId(format!("expected {}_<32 hex>, got {:?}", $prefix, truncate(s))))
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = DomainError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::parse(s)
            }
        }

        impl TryFrom<String> for $name {
            type Error = DomainError;
            fn try_from(s: String) -> Result<Self, Self::Error> {
                Self::parse(&s)
            }
        }

        impl From<$name> for String {
            fn from(id: $name) -> String {
                id.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}

fn truncate(s: &str) -> &str {
    match s.char_indices().nth(48) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

fn is_valid_id(s: &str, prefix: &str) -> bool {
    let Some(rest) = s.strip_prefix(prefix).and_then(|r| r.strip_prefix('_')) else {
        return false;
    };
    rest.len() == 32 && rest.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

id_type!(/// A KabadOS user (one account can hold several roles).
    UserId, "usr");
id_type!(AddressId, "adr");
id_type!(PayoutMethodId, "pay");
id_type!(DocumentId, "doc");
id_type!(VehicleId, "veh");
id_type!(ListingId, "lst");
id_type!(ItemId, "itm");
id_type!(ClaimId, "clm");
id_type!(ProofRequestId, "prf");
id_type!(ImageId, "img");
id_type!(OfferId, "ofr");
id_type!(DealId, "del");
id_type!(JobId, "job");
id_type!(DriveId, "drv");
id_type!(ThreadId, "thr");
id_type!(MessageId, "msg");
id_type!(RuleSetId, "rs");
id_type!(CallId, "cal");
id_type!(DisputeId, "dsp");
id_type!(EventId, "evt");
id_type!(ReceiptId, "rcp");
id_type!(NotificationId, "ntf");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_ids_round_trip() {
        let id = ListingId::new();
        assert!(id.as_str().starts_with("lst_"));
        assert_eq!(ListingId::parse(id.as_str()).unwrap(), id);
        let json = serde_json::to_string(&id).unwrap();
        let back: ListingId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn rejects_wrong_prefix_and_injection() {
        let good = ListingId::new();
        let s = good.as_str().replace("lst_", "ofr_");
        assert!(ListingId::parse(&s).is_err());
        assert!(ListingId::parse("lst_abc#META").is_err());
        assert!(ListingId::parse("lst_0192F5B8C1A77D3E9B1E0C2D4F6A8B0C").is_err());
        assert!(serde_json::from_str::<ListingId>("\"lst_x\"").is_err());
    }

    #[test]
    fn ids_sort_by_creation_time() {
        let a = DealId::new();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = DealId::new();
        assert!(a < b);
    }
}
