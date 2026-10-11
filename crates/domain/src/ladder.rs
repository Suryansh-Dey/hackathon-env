//! Verification ladder (PLAN.md §11.5).

use serde::{Deserialize, Serialize};

/// How much evidence supports a claim. Ordered from weakest to strongest.
#[derive(
    Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema,
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LadderLevel {
    /// Seller said so; no supporting image.
    #[default]
    #[serde(rename = "L0_CLAIMED")]
    L0Claimed,
    /// Photos are consistent with the claim (AI).
    #[serde(rename = "L1_PHOTO_CONSISTENT")]
    L1PhotoConsistent,
    /// Close-up shows material, colour or grade (AI).
    #[serde(rename = "L2_CLOSE_UP")]
    L2CloseUp,
    /// Label, model number or markings read and consistent (AI + Textract).
    #[serde(rename = "L3_IDENTIFIER")]
    L3Identifier,
    /// Vendor or transporter confirmed physically at pickup (human).
    #[serde(rename = "L4_VERIFIED_AT_PICKUP")]
    L4VerifiedAtPickup,
}

impl LadderLevel {
    /// Levels the AI may assign; L4 is human-only.
    pub fn ai_assignable(self) -> bool {
        self < LadderLevel::L4VerifiedAtPickup
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::L0Claimed => "Claimed",
            Self::L1PhotoConsistent => "Photo-consistent",
            Self::L2CloseUp => "Close-up evidence",
            Self::L3Identifier => "Identifier evidence",
            Self::L4VerifiedAtPickup => "Verified at pickup",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_and_serialized_as_in_plan() {
        assert!(LadderLevel::L0Claimed < LadderLevel::L2CloseUp);
        assert_eq!(serde_json::to_string(&LadderLevel::L2CloseUp).unwrap(), "\"L2_CLOSE_UP\"");
        assert!(!LadderLevel::L4VerifiedAtPickup.ai_assignable());
    }
}
