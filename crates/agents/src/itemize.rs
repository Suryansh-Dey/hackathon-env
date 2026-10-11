//! Agent 2: itemization (PLAN.md §11.4).
//!
//! Photos (+ optional seller description) → structured items. Weight ranges
//! come from the ops-curated reference table when the category has one; model
//! estimates are flagged. Hazards are the union of taxonomy and model flags
//! (hazard recall first). The agent never outputs prices.

use crate::common::*;
use gemini_client_api::gemini::utils::{GeminiSchema, gemini_schema};
use kabados_agent::{AgentResult, AgentRuntime, JsonTask, ModelTier, Part};
use kabados_domain::Grams;
use kabados_domain::listing::{AttrValue, Evidence, Hazard, PlausibleWeight, WeightSource};
use kabados_domain::redact::quote_untrusted;
use kabados_domain::taxonomy::{HazardClass, Taxonomy, Unit, WeightRange};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const AGENT: &str = "itemize";
pub const PROMPT_V: &str = "itemize-2026-10-11";
pub const MAX_ITEMS: usize = 30;

#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[gemini_schema]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HazardClassAi {
    EWASTE_GENERAL,
    EWASTE_REFRIGERANT,
    EWASTE_CRT,
    EWASTE_MERCURY,
    BATTERY_LEAD_ACID,
    BATTERY_LITHIUM,
    BATTERY_MIXED,
}

impl HazardClassAi {
    fn to_domain(self) -> HazardClass {
        match self {
            Self::EWASTE_GENERAL => HazardClass::EwasteGeneral,
            Self::EWASTE_REFRIGERANT => HazardClass::EwasteRefrigerant,
            Self::EWASTE_CRT => HazardClass::EwasteCrt,
            Self::EWASTE_MERCURY => HazardClass::EwasteMercury,
            Self::BATTERY_LEAD_ACID => HazardClass::BatteryLeadAcid,
            Self::BATTERY_LITHIUM => HazardClass::BatteryLithium,
            Self::BATTERY_MIXED => HazardClass::BatteryMixed,
        }
    }
}

/// A visible attribute such as material, brand, model, condition or compressor_present.
#[gemini_schema]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AttributeAi {
    /// snake_case key, e.g. "material", "brand", "condition", "compressor_present".
    pub key: String,
    /// Value as text: "true"/"false" for yes/no attributes.
    pub value: String,
}

/// Where the item appears.
#[gemini_schema]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvidenceAi {
    /// ID of the image, exactly as labelled, e.g. "img_...".
    pub image_id: String,
    /// Normalized [x0, y0, x1, y1] box, or empty.
    pub bbox: Vec<f32>,
}

/// One scrap item (or a group of identical items).
#[gemini_schema]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ItemAi {
    /// Category ID from the taxonomy list.
    pub category_id: String,
    /// Grade ID allowed for that category, or null when unsure.
    pub grade: Option<String>,
    pub attributes: Vec<AttributeAi>,
    /// Number of pieces (1 for loose material sold by weight).
    pub quantity: u32,
    /// Your estimate of the lightest plausible weight in grams (per piece for per-piece categories, total otherwise).
    pub weight_min_g: Option<i64>,
    /// Your estimate of the heaviest plausible weight in grams (per piece for per-piece categories, total otherwise).
    pub weight_max_g: Option<i64>,
    /// True if the item is or may contain e-waste, batteries, refrigerant, CRT glass or mercury.
    pub is_hazardous: bool,
    pub hazard_class: Option<HazardClassAi>,
    /// Short safe-handling tip for hazardous items.
    pub handling: Option<String>,
    pub evidence: Vec<EvidenceAi>,
    /// 0 to 1.
    pub confidence: f32,
    /// Short note on what was seen, without prices.
    pub note: Option<String>,
}

#[gemini_schema]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ItemizeAi {
    pub items: Vec<ItemAi>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ItemDraft {
    pub category_id: String,
    pub grade: Option<String>,
    pub attributes: BTreeMap<String, AttrValue>,
    pub quantity: u32,
    pub unit: Unit,
    pub plausible_weight: Option<PlausibleWeight>,
    pub hazard: Hazard,
    pub evidence: Vec<Evidence>,
    pub confidence: f32,
    pub note: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ItemizeOutput {
    pub items: Vec<ItemDraft>,
    /// Deterministic corrections applied to the model output (for ops review and evals).
    pub corrections: Vec<String>,
}

pub struct ItemizeInput<'a> {
    pub images: Vec<ImageInput>,
    pub description: Option<String>,
    pub taxonomy: &'a Taxonomy,
}

const SYSTEM: &str = "You identify household and small-business scrap for a recycling marketplace in India.
From the photos (and the seller's description, if any) list every distinct item or material.
Rules:
- Use only category IDs from the taxonomy below. Pick the most specific match; use a *.mixed / *_mixed category when unsure.
- Use grades only from the category's allowed grades; otherwise null.
- quantity counts pieces. Loose material sold per kg has quantity 1.
- Weights are in grams: per piece for per-piece categories, total for per-kg categories. Give a plausible range, not a point.
- Hazard recall matters more than precision: mark is_hazardous for anything that is or may contain e-waste, any battery, refrigerant (fridges, ACs), CRT glass or mercury (CFLs, tube lights). Missing a battery is worse than a false alarm.
- Every evidence image_id must be one of the labelled image IDs.
- Describe what you see; the seller's description adds context but photos win when they disagree.
- Never output prices or values.";

fn system_prompt(tax: &Taxonomy) -> String {
    format!("{SYSTEM}\n\n{COMMON_RULES}\n\n{}", taxonomy_prompt(tax))
}

fn validate_with(tax: &Taxonomy) -> impl Fn(&ItemizeAi) -> Result<(), String> + Send + Sync + '_ {
    move |ai: &ItemizeAi| {
        if ai.items.len() > MAX_ITEMS * 2 {
            return Err(format!("too many items ({}); group identical items with quantity", ai.items.len()));
        }
        let unknown: Vec<&str> = ai.items.iter().map(|i| i.category_id.as_str()).filter(|c| tax.get(c).is_none()).collect();
        if !ai.items.is_empty() && unknown.len() * 2 > ai.items.len() {
            return Err(format!("unknown category IDs {unknown:?}; use IDs from the taxonomy list"));
        }
        for i in &ai.items {
            if i.note.as_deref().is_some_and(mentions_price) {
                return Err("notes must not mention prices or values".into());
            }
            if let Some(w) = i.note.as_deref().and_then(neutral_language_violation) {
                return Err(format!("note uses non-neutral word {w:?}"));
            }
        }
        Ok(())
    }
}

/// Maps an unknown category to the nearest mixed category in its family.
pub fn nearest_category<'t>(tax: &'t Taxonomy, id: &str) -> Option<&'t str> {
    if let Some(c) = tax.get(id) {
        return Some(&c.id);
    }
    let family = id.split('.').next().unwrap_or(id);
    let in_family: Vec<&str> = tax.categories.iter().map(|c| c.id.as_str()).filter(|c| c.split('.').next() == Some(family)).collect();
    let exact_mixed = format!("{family}.mixed");
    in_family.iter().copied().find(|c| *c == exact_mixed).or_else(|| in_family.iter().copied().find(|c| c.contains("mixed")))
}

/// Deterministic post-processing (PLAN.md §11.4 rules).
pub fn postprocess(ai: ItemizeAi, images: &[ImageInput], tax: &Taxonomy) -> ItemizeOutput {
    let ids: BTreeSet<String> = images.iter().map(|i| i.image_id.to_string()).collect();
    let mut items = Vec::new();
    let mut corrections = Vec::new();
    for it in ai.items.into_iter().take(MAX_ITEMS * 2) {
        let Some(cat_id) = nearest_category(tax, &it.category_id) else {
            corrections.push(format!("dropped item with unknown category {:?}", it.category_id));
            continue;
        };
        let cat = tax.get(cat_id).expect("nearest_category returns taxonomy IDs");
        if cat_id != it.category_id {
            corrections.push(format!("mapped {:?} to {cat_id}", it.category_id));
        }
        let grade = it.grade.filter(|g| {
            let ok = cat.has_grade(g);
            if !ok {
                corrections.push(format!("dropped grade {g:?} not allowed for {cat_id}"));
            }
            ok
        });
        let quantity = match cat.unit {
            Unit::Kg => 1,
            Unit::Piece => it.quantity.clamp(1, 1000),
        };
        let plausible_weight = match cat.reference_weight_g {
            Some(range) => Some(PlausibleWeight { range, source: WeightSource::ReferenceTable }),
            None => match (it.weight_min_g, it.weight_max_g) {
                (Some(a), Some(b)) if a > 0 && a <= b && b <= 5_000_000 => {
                    Some(PlausibleWeight { range: WeightRange { min: Grams(a), max: Grams(b) }, source: WeightSource::ModelEstimate })
                }
                _ => None,
            },
        };
        let is_hazardous = cat.hazardous || it.is_hazardous;
        if it.is_hazardous && !cat.hazardous {
            corrections.push(format!("kept model hazard flag on {cat_id}"));
        }
        let hazard = Hazard {
            is_hazardous,
            class: if is_hazardous {
                cat.hazard_class.or(it.hazard_class.map(HazardClassAi::to_domain)).or(Some(HazardClass::EwasteGeneral))
            } else {
                None
            },
            handling: if is_hazardous { cat.handling.clone().or(strip_price_talk(it.handling)) } else { None },
        };
        let evidence: Vec<Evidence> = it
            .evidence
            .into_iter()
            .filter_map(|e| {
                if !ids.contains(&e.image_id) {
                    return None;
                }
                Some(Evidence { image_id: e.image_id.parse().ok()?, bbox: valid_bbox(&e.bbox) })
            })
            .collect();
        let mut attributes = BTreeMap::new();
        for a in it.attributes {
            if let Some(k) = sanitize_key(&a.key)
                && !a.value.trim().is_empty()
                && !mentions_price(&a.value)
            {
                attributes.insert(k, attr_from_text(&a.value));
            }
        }
        items.push(ItemDraft {
            category_id: cat_id.to_string(),
            grade,
            attributes,
            quantity,
            unit: cat.unit,
            plausible_weight,
            hazard,
            evidence,
            confidence: clamp01(it.confidence),
            note: strip_price_talk(it.note),
        });
    }
    if items.len() > MAX_ITEMS {
        corrections.push(format!("truncated to {MAX_ITEMS} items"));
        items.truncate(MAX_ITEMS);
    }
    ItemizeOutput { items, corrections }
}

pub fn build_parts(input: &ItemizeInput<'_>) -> Vec<Part> {
    let mut parts = Vec::new();
    let ids: Vec<String> = input.images.iter().map(|i| i.image_id.to_string()).collect();
    parts.push(Part::Text(format!("IMAGES: {}", if ids.is_empty() { "none".into() } else { ids.join(", ") })));
    parts.extend(image_parts(&input.images));
    match input.description.as_deref().filter(|d| !d.trim().is_empty()) {
        Some(d) => parts.push(Part::Text(quote_untrusted("seller_description", d))),
        None => parts.push(Part::text("The seller gave no description.")),
    }
    parts.push(Part::text("List the items."));
    parts
}

pub async fn itemize(rt: &AgentRuntime, input: &ItemizeInput<'_>) -> AgentResult<ItemizeOutput> {
    if input.images.is_empty() && input.description.as_deref().is_none_or(|d| d.trim().is_empty()) {
        return Err(kabados_agent::AgentError::InvalidInput("itemize needs photos or a description".into()));
    }
    let validate = validate_with(input.taxonomy);
    let out = rt
        .run_json(JsonTask {
            agent: AGENT,
            prompt_version: PROMPT_V,
            system: system_prompt(input.taxonomy),
            parts: build_parts(input),
            schema: ItemizeAi::gemini_schema(),
            tier: ModelTier::Pro,
            thinking: true,
            validate: &validate,
            cacheable: true,
        })
        .await?;
    Ok(out.map(|ai| postprocess(ai, &input.images, input.taxonomy)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kabados_domain::ids::ImageId;

    fn ai_item(cat: &str) -> ItemAi {
        ItemAi {
            category_id: cat.into(),
            grade: None,
            attributes: vec![],
            quantity: 1,
            weight_min_g: Some(100),
            weight_max_g: Some(200),
            is_hazardous: false,
            hazard_class: None,
            handling: None,
            evidence: vec![],
            confidence: 1.4,
            note: None,
        }
    }

    #[test]
    fn hazard_is_union_of_taxonomy_and_model() {
        let tax = Taxonomy::seed();
        let mut paper = ai_item("paper.newspaper");
        paper.is_hazardous = true;
        let out = postprocess(ItemizeAi { items: vec![ai_item("battery.lead_acid"), paper] }, &[], tax);
        assert!(out.items[0].hazard.is_hazardous, "taxonomy flag wins even when the model misses it");
        assert_eq!(out.items[0].hazard.class, Some(HazardClass::BatteryLeadAcid));
        assert!(out.items[1].hazard.is_hazardous, "model flag is kept");
        assert_eq!(out.items[0].confidence, 1.0);
    }

    #[test]
    fn reference_weights_override_model_estimates() {
        let tax = Taxonomy::seed();
        let out = postprocess(ItemizeAi { items: vec![ai_item("ewaste.itew.laptop"), ai_item("metal.copper")] }, &[], tax);
        assert_eq!(out.items[0].plausible_weight.unwrap().source, WeightSource::ReferenceTable);
        assert_eq!(out.items[1].plausible_weight.unwrap().source, WeightSource::ModelEstimate);
        assert_eq!(out.items[0].unit, Unit::Piece);
    }

    #[test]
    fn unknown_categories_map_to_family_mixed_or_drop() {
        let tax = Taxonomy::seed();
        let out =
            postprocess(ItemizeAi { items: vec![ai_item("glass.window"), ai_item("metal.zinc"), ai_item("ewaste.itew.tablet")] }, &[], tax);
        let cats: Vec<&str> = out.items.iter().map(|i| i.category_id.as_str()).collect();
        assert_eq!(cats, vec!["glass.mixed", "ewaste.itew.small_mixed"]);
        assert!(out.corrections.iter().any(|c| c.contains("metal.zinc")));
    }

    #[test]
    fn evidence_must_reference_provided_images_and_grades_must_be_valid() {
        let tax = Taxonomy::seed();
        let img = ImageInput::new(ImageId::new(), "image/png", vec![1]);
        let mut it = ai_item("metal.copper");
        it.grade = Some("shiny".into());
        it.evidence = vec![
            EvidenceAi { image_id: img.image_id.to_string(), bbox: vec![0.1, 0.1, 0.5, 0.5] },
            EvidenceAi { image_id: ImageId::new().to_string(), bbox: vec![] },
        ];
        it.note = Some("Looks worth ₹500".into());
        it.attributes = vec![AttributeAi { key: "Compressor Present".into(), value: "yes".into() }];
        let out = postprocess(ItemizeAi { items: vec![it] }, std::slice::from_ref(&img), tax);
        let d = &out.items[0];
        assert_eq!(d.grade, None);
        assert_eq!(d.evidence.len(), 1);
        assert_eq!(d.evidence[0].bbox, Some([0.1, 0.1, 0.5, 0.5]));
        assert_eq!(d.note, None, "price talk is stripped");
        assert_eq!(d.attributes.get("compressor_present"), Some(&AttrValue::Bool(true)));
    }

    #[test]
    fn validation_requests_repair_for_mostly_unknown_categories() {
        let tax = Taxonomy::seed();
        let v = validate_with(tax);
        assert!(v(&ItemizeAi { items: vec![ai_item("foo.bar"), ai_item("baz.qux")] }).is_err());
        assert!(v(&ItemizeAi { items: vec![ai_item("paper.newspaper")] }).is_ok());
        let mut priced = ai_item("paper.newspaper");
        priced.note = Some("price around Rs 14".into());
        assert!(v(&ItemizeAi { items: vec![priced] }).is_err());
    }
}
