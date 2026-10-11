//! Scrap category taxonomy (PLAN.md §12.1).
//!
//! The seed taxonomy is embedded so every component (including the offline
//! transporter app) can validate category IDs. Ops can override it at runtime
//! through `TAXONOMY` items in DynamoDB.

use crate::{DomainError, Grams};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::OnceLock;

const SEED: &str = include_str!("../data/taxonomy.json");

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Unit {
    Kg,
    Piece,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HazardClass {
    EwasteGeneral,
    EwasteRefrigerant,
    EwasteCrt,
    EwasteMercury,
    BatteryLeadAcid,
    BatteryLithium,
    BatteryMixed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct Grade {
    pub id: String,
    pub name: String,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct WeightRange {
    pub min: Grams,
    pub max: Grams,
}

impl WeightRange {
    pub fn contains(&self, w: Grams) -> bool {
        self.min <= w && w <= self.max
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct Category {
    pub id: String,
    pub name: String,
    pub unit: Unit,
    #[serde(default)]
    pub grades: Vec<Grade>,
    #[serde(default)]
    pub hazardous: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hazard_class: Option<HazardClass>,
    /// E-Waste (Management) Rules 2022 schedule category, e.g. `ITEW`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ewaste_schedule: Option<String>,
    /// Ops-curated per-piece weight range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_weight_g: Option<WeightRange>,
    /// Seller-estimate tolerance used for quote ranges (PLAN.md §12.3 step 3).
    #[serde(default = "default_tolerance")]
    pub weight_tolerance_pct: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handling: Option<String>,
    /// Grams CO2e avoided per kg recycled (estimate, PLAN.md §11.11).
    #[serde(default)]
    pub emission_factor_g_per_kg: i64,
}

fn default_tolerance() -> u32 {
    15
}

impl Category {
    /// Top-level family, e.g. `metal` for `metal.copper`.
    pub fn family(&self) -> &str {
        self.id.split('.').next().unwrap_or(&self.id)
    }

    pub fn has_grade(&self, grade: &str) -> bool {
        self.grades.iter().any(|g| g.id == grade)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct Taxonomy {
    pub version: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub emission_factor_source: String,
    pub categories: Vec<Category>,
    #[serde(skip)]
    #[schema(ignore)]
    #[schemars(skip)]
    index: BTreeMap<String, usize>,
}

impl Taxonomy {
    pub fn from_json(json: &str) -> Result<Self, DomainError> {
        let mut t: Taxonomy = serde_json::from_str(json).map_err(|e| DomainError::validation(format!("taxonomy: {e}")))?;
        t.reindex()?;
        Ok(t)
    }

    pub fn new(version: impl Into<String>, categories: Vec<Category>) -> Result<Self, DomainError> {
        let mut t = Taxonomy {
            version: version.into(),
            note: String::new(),
            emission_factor_source: String::new(),
            categories,
            index: BTreeMap::new(),
        };
        t.reindex()?;
        Ok(t)
    }

    fn reindex(&mut self) -> Result<(), DomainError> {
        self.index.clear();
        for (i, c) in self.categories.iter().enumerate() {
            if !is_valid_category_id(&c.id) {
                return Err(DomainError::validation(format!("bad category id {:?}", c.id)));
            }
            if c.hazardous && c.hazard_class.is_none() {
                return Err(DomainError::validation(format!("{} is hazardous without a hazard class", c.id)));
            }
            if self.index.insert(c.id.clone(), i).is_some() {
                return Err(DomainError::validation(format!("duplicate category {}", c.id)));
            }
        }
        Ok(())
    }

    /// The embedded seed taxonomy.
    pub fn seed() -> &'static Taxonomy {
        static T: OnceLock<Taxonomy> = OnceLock::new();
        T.get_or_init(|| Taxonomy::from_json(SEED).expect("embedded taxonomy is valid"))
    }

    pub fn get(&self, id: &str) -> Option<&Category> {
        self.index.get(id).map(|&i| &self.categories[i])
    }

    pub fn require(&self, id: &str) -> Result<&Category, DomainError> {
        self.get(id).ok_or_else(|| DomainError::UnknownCategory(id.to_owned()))
    }

    pub fn is_hazardous(&self, id: &str) -> bool {
        self.get(id).is_some_and(|c| c.hazardous)
    }

    /// Categories matching a glob like `paper.*` or `metal.copper`.
    pub fn matching<'a>(&'a self, glob: &'a str) -> impl Iterator<Item = &'a Category> + 'a {
        self.categories.iter().filter(move |c| glob_matches(glob, &c.id))
    }
}

/// Category IDs are dot-separated lowercase segments.
pub fn is_valid_category_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.split('.').all(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'))
}

/// Matches `glob` against a category ID. Supports exact IDs, `family.*`
/// (any depth below `family`) and `*`.
pub fn glob_matches(glob: &str, id: &str) -> bool {
    if glob == "*" {
        return true;
    }
    match glob.strip_suffix(".*") {
        Some(prefix) => id.len() > prefix.len() && id.starts_with(prefix) && id.as_bytes()[prefix.len()] == b'.',
        None => glob == id,
    }
}

/// Validates a glob (`*`, `family.*`, or an exact valid ID).
pub fn is_valid_glob(glob: &str) -> bool {
    glob == "*" || glob.strip_suffix(".*").map_or_else(|| is_valid_category_id(glob), is_valid_category_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_loads_and_flags_hazards() {
        let t = Taxonomy::seed();
        assert!(t.categories.len() >= 30);
        assert!(t.is_hazardous("battery.lead_acid"));
        assert!(t.is_hazardous("ewaste.ceew.split_ac_outdoor"));
        assert!(!t.is_hazardous("paper.newspaper"));
        let cu = t.require("metal.copper").unwrap();
        assert!(cu.has_grade("clean") && cu.has_grade("with_insulation"));
        assert_eq!(cu.family(), "metal");
        // Every e-waste and battery category is hazardous (PLAN.md §2 #10).
        for c in &t.categories {
            if c.id.starts_with("ewaste.") || c.id.starts_with("battery.") {
                assert!(c.hazardous, "{} must be hazardous", c.id);
            }
            if c.unit == Unit::Piece {
                assert!(c.reference_weight_g.is_some(), "{} needs a reference weight", c.id);
            }
        }
    }

    #[test]
    fn globs() {
        assert!(glob_matches("paper.*", "paper.newspaper"));
        assert!(glob_matches("ewaste.*", "ewaste.itew.laptop"));
        assert!(!glob_matches("paper.*", "paperx.foo"));
        assert!(!glob_matches("paper.*", "paper"));
        assert!(glob_matches("metal.copper", "metal.copper"));
        assert!(glob_matches("*", "glass.mixed"));
        assert!(is_valid_glob("glass.*") && !is_valid_glob("glass*") && !is_valid_glob("Glass.*"));
        assert_eq!(Taxonomy::seed().matching("glass.*").count(), 2);
    }
}
