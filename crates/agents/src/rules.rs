//! Agent 5: rules authoring from a vendor's typed instruction (PLAN.md §11.7).
//!
//! The model extracts changes; deterministic code composes the `RuleSet`
//! draft (₹ → paise, unit from taxonomy, upserts, % adjustments), validates it
//! and flags prices far from the Rate Index. Nothing publishes here.

use crate::common::*;
use crate::keywords;
use chrono::{DateTime, Duration, FixedOffset, NaiveTime, TimeZone, Utc};
use gemini_client_api::gemini::utils::{GeminiSchema, gemini_schema};
use kabados_agent::{AgentResult, AgentRuntime, JsonTask, ModelTier, Part};
use kabados_domain::geo::LatLng;
use kabados_domain::ids::{RuleSetId, UserId};
use kabados_domain::listing::Weekday;
use kabados_domain::money::div_round;
use kabados_domain::redact::quote_untrusted;
use kabados_domain::ruleset::{Minimums, RuleLine, RuleSet, RuleSetStatus, Schedule, ServiceArea, ValidationIssue};
use kabados_domain::taxonomy::{self, Taxonomy, Unit};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::OnceLock;

pub const AGENT: &str = "rules_author";
pub const PROMPT_V: &str = "rules-author-2026-10-11";
/// Prices this far from the Rate Index need confirmation (PLAN.md §11.7).
pub const FAR_FROM_INDEX_PCT: i64 = 40;

#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[gemini_schema]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnitAi {
    KG,
    PIECE,
}

/// A price line to add or change.
#[gemini_schema]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuleLineAi {
    /// Category ID from the taxonomy.
    pub category_id: String,
    /// Grade ID, only if the vendor named one.
    pub grade: Option<String>,
    pub unit: UnitAi,
    /// Price in rupees per kg or per piece, exactly as stated.
    pub price_rupees: f64,
    /// Minimum weight in kg for this price (e.g. "above 5 kilo" → 5).
    pub min_weight_kg: Option<f64>,
}

/// A percentage change to existing prices, e.g. "all metals +3%".
#[gemini_schema]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdjustmentAi {
    /// Category glob such as "metal.*" or an exact category ID.
    pub category_glob: String,
    /// Signed percentage, e.g. 3 or -5.
    pub percent: f64,
}

#[gemini_schema]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RulesAi {
    pub lines: Vec<RuleLineAi>,
    /// Category IDs or globs whose existing lines should be removed.
    pub remove: Vec<String>,
    /// Category globs the vendor does not buy, e.g. "glass.*".
    pub exclusions: Vec<String>,
    pub adjustments: Vec<AdjustmentAi>,
    /// ISO-8601 start time with +05:30 offset, or null to start immediately.
    pub effective_from: Option<String>,
    /// True only when the instruction is a complete new rate card replacing the old one.
    pub replace_all: bool,
    /// Questions to confirm with the vendor, e.g. "Does 'paper' mean newspaper only, or cardboard too?"
    pub ambiguities: Vec<String>,
}

pub struct RulesAuthorInput<'a> {
    pub instruction: String,
    pub current: Option<RuleSet>,
    pub taxonomy: &'a Taxonomy,
    /// Rate Index medians, paise per kg, by category ID.
    pub rate_index_paise_per_kg: BTreeMap<String, i64>,
    pub hazardous_authorized: bool,
    pub vendor_id: UserId,
    pub now: DateTime<Utc>,
    /// Vendor yard, used for the default service area when there is no current rule set.
    pub yard: Option<LatLng>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RulesDraftResult {
    /// Status DRAFT, version = current + 1. The vendor must tap Publish on a preview.
    pub draft: RuleSet,
    pub ambiguities: Vec<String>,
    pub far_from_index: Vec<String>,
    pub issues: Vec<ValidationIssue>,
}

pub fn ist() -> FixedOffset {
    FixedOffset::east_opt(5 * 3600 + 30 * 60).expect("valid offset")
}

const SYSTEM: &str = "You turn a scrap vendor's short typed instruction into structured rate-card changes for a recycling marketplace in India.
- Prices in the text are rupees: \"copper 650\" means ₹650 per kg. Copy numbers exactly; never invent prices.
- Map every material to a category ID from the taxonomy. If a word could mean several categories (e.g. \"paper\" could be newspaper or cardboard), choose the most common one (newspaper) and add a question to ambiguities.
- Per-piece categories (appliances, laptops) use unit PIECE; materials use KG.
- \"above 5 kilo\" sets min_weight_kg 5 on that line.
- \"no glass\" adds the exclusion \"glass.*\". \"stop buying X\" adds X to remove.
- \"all metals +3%\" is an adjustment on \"metal.*\" with percent 3. Adjustments change existing prices; do not also add lines for them.
- Times like \"from tomorrow 9am\" become effective_from in IST (+05:30) relative to the current time given.
- replace_all is true only if the vendor clearly gives a complete new card.";

fn system_prompt(tax: &Taxonomy) -> String {
    format!("{SYSTEM}\n\n{COMMON_RULES}\n\n{}", taxonomy_prompt(tax))
}

fn validate_with(tax: &Taxonomy) -> impl Fn(&RulesAi) -> Result<(), String> + Send + Sync + '_ {
    move |ai: &RulesAi| {
        if ai.lines.is_empty() && ai.remove.is_empty() && ai.exclusions.is_empty() && ai.adjustments.is_empty() && ai.ambiguities.is_empty()
        {
            return Err("no changes found; extract the price lines, exclusions or adjustments, or ask a question in ambiguities".into());
        }
        for l in &ai.lines {
            if !l.price_rupees.is_finite() || l.price_rupees <= 0.0 || l.price_rupees > 1_000_000.0 {
                return Err(format!("price for {} must be a positive number of rupees", l.category_id));
            }
            if l.min_weight_kg.is_some_and(|w| !w.is_finite() || w <= 0.0) {
                return Err(format!("min_weight_kg for {} must be positive", l.category_id));
            }
        }
        let unknown: Vec<&str> = ai.lines.iter().map(|l| l.category_id.as_str()).filter(|c| tax.get(c).is_none()).collect();
        if !ai.lines.is_empty() && unknown.len() * 2 > ai.lines.len() {
            return Err(format!("unknown category IDs {unknown:?}; use taxonomy IDs"));
        }
        for a in &ai.adjustments {
            if !a.percent.is_finite() || a.percent.abs() > 50.0 {
                return Err("adjustments must be between -50 and 50 percent".into());
            }
        }
        Ok(())
    }
}

fn rupees_to_paise(r: f64) -> i64 {
    (r * 100.0).round() as i64
}

/// Applies a signed percentage to a paise amount, rounding half away from zero.
pub fn adjust(price: i64, percent: f64) -> i64 {
    let bp = (percent * 100.0).round() as i128;
    div_round(price as i128 * (10_000 + bp), 10_000) as i64
}

fn default_rule_set(input: &RulesAuthorInput<'_>) -> RuleSet {
    RuleSet {
        rule_set_id: RuleSetId::new(),
        vendor_id: input.vendor_id.clone(),
        version: 0,
        status: RuleSetStatus::Draft,
        effective_from: input.now.with_timezone(&ist()),
        service_area: ServiceArea::Radius { center: input.yard.unwrap_or(LatLng { lat: 0.0, lng: 0.0 }), radius_km: 5.0 },
        schedule: Schedule {
            days: vec![Weekday::Mon, Weekday::Tue, Weekday::Wed, Weekday::Thu, Weekday::Fri, Weekday::Sat],
            slots: vec!["09:00-18:00".into()],
            daily_capacity_kg: None,
        },
        minimums: Minimums::default(),
        lines: vec![],
        modifiers: vec![],
        exclusions: vec![],
        accept_partial_lots: true,
        source: None,
    }
}

/// Deterministic composition of the draft from the extracted changes.
pub fn compose(ai: RulesAi, input: &RulesAuthorInput<'_>) -> RulesDraftResult {
    let tax = input.taxonomy;
    let mut ambiguities: Vec<String> = ai.ambiguities.into_iter().filter(|a| !a.trim().is_empty()).collect();
    let base = match &input.current {
        Some(c) => c.clone(),
        None => {
            ambiguities.push("Please confirm your service area and working hours; defaults were used.".into());
            default_rule_set(input)
        }
    };
    let mut draft = base.clone();
    draft.version = base.version + 1;
    draft.status = RuleSetStatus::Draft;
    draft.vendor_id = input.vendor_id.clone();
    draft.source = Some("text".into());
    if ai.replace_all {
        draft.lines.clear();
    }

    for r in &ai.remove {
        draft.lines.retain(|l| !taxonomy::glob_matches(r, &l.category_id));
    }
    for a in &ai.adjustments {
        if !taxonomy::is_valid_glob(&a.category_glob) {
            ambiguities.push(format!("Couldn't apply the change to {:?}; which materials did you mean?", a.category_glob));
            continue;
        }
        let mut hit = false;
        for l in draft.lines.iter_mut().filter(|l| taxonomy::glob_matches(&a.category_glob, &l.category_id)) {
            hit = true;
            l.price_per_kg_paise = l.price_per_kg_paise.map(|p| adjust(p, a.percent));
            l.price_per_piece_paise = l.price_per_piece_paise.map(|p| adjust(p, a.percent));
        }
        if !hit {
            ambiguities
                .push(format!("No current prices match {:?}, so the {}% change had nothing to apply to.", a.category_glob, a.percent));
        }
    }
    for l in ai.lines {
        let Some(cat) = tax.get(&l.category_id) else {
            ambiguities.push(format!("Couldn't match {:?} to a category; please pick it from the list.", l.category_id));
            continue;
        };
        let grade = match l.grade {
            Some(g) if cat.has_grade(&g) => Some(g),
            Some(g) => {
                ambiguities.push(format!("{g:?} is not a grade of {}; the price was applied to all grades.", cat.name));
                None
            }
            None => None,
        };
        let ai_unit = match l.unit {
            UnitAi::KG => Unit::Kg,
            UnitAi::PIECE => Unit::Piece,
        };
        if ai_unit != cat.unit {
            ambiguities.push(format!(
                "{} is priced per {}; please confirm the price.",
                cat.name,
                if cat.unit == Unit::Kg { "kg" } else { "piece" }
            ));
        }
        let paise = rupees_to_paise(l.price_rupees);
        let min_weight_g = l.min_weight_kg.map(|kg| (kg * 1000.0).round() as i64);
        let line = RuleLine {
            category_id: cat.id.clone(),
            grade,
            unit: None,
            price_per_kg_paise: (cat.unit == Unit::Kg).then_some(paise),
            price_per_piece_paise: (cat.unit == Unit::Piece).then_some(paise),
            min_weight_g,
            conditions: Default::default(),
        };
        match draft.lines.iter_mut().find(|x| x.category_id == line.category_id && x.grade == line.grade && x.conditions.is_empty()) {
            Some(existing) => *existing = line,
            None => draft.lines.push(line),
        }
    }
    for e in ai.exclusions {
        if taxonomy::is_valid_glob(&e) {
            if !draft.exclusions.contains(&e) {
                draft.exclusions.push(e.clone());
            }
            draft.lines.retain(|l| !taxonomy::glob_matches(&e, &l.category_id));
        } else {
            ambiguities.push(format!("Couldn't understand the exclusion {e:?}."));
        }
    }

    let now_ist = input.now.with_timezone(&ist());
    draft.effective_from = match ai.effective_from.as_deref().map(DateTime::parse_from_rfc3339) {
        Some(Ok(t)) if t.with_timezone(&Utc) >= input.now - Duration::minutes(5) => t.with_timezone(&ist()),
        Some(Ok(_)) => {
            ambiguities.push("The start time was in the past; the draft starts immediately.".into());
            now_ist
        }
        Some(Err(_)) => {
            ambiguities.push("Couldn't read the start time; the draft starts immediately.".into());
            now_ist
        }
        None => now_ist,
    };

    let mut far_from_index = Vec::new();
    for l in &draft.lines {
        if let (Some(p), Some(idx)) = (l.price_per_kg_paise, input.rate_index_paise_per_kg.get(&l.category_id))
            && *idx > 0
            && (p - idx).abs() * 100 > idx * FAR_FROM_INDEX_PCT
        {
            let name = tax.get(&l.category_id).map(|c| c.name.as_str()).unwrap_or(&l.category_id);
            far_from_index.push(format!(
                "{name}: ₹{} per kg is more than {FAR_FROM_INDEX_PCT}% away from the local index (₹{}). Please confirm.",
                kabados_domain::Paise(p).format_inr().trim_start_matches('₹'),
                kabados_domain::Paise(*idx).format_inr().trim_start_matches('₹'),
            ));
        }
    }
    let issues = draft.validate(tax, input.hazardous_authorized);
    ambiguities.dedup();
    RulesDraftResult { draft, ambiguities, far_from_index, issues }
}

fn current_summary(rs: &Option<RuleSet>) -> String {
    match rs {
        None => "The vendor has no rate card yet.".into(),
        Some(rs) => {
            let lines: Vec<serde_json::Value> = rs
                .lines
                .iter()
                .map(|l| {
                    serde_json::json!({
                        "category_id": l.category_id,
                        "grade": l.grade,
                        "price_rupees": l.price_per_kg_paise.or(l.price_per_piece_paise).map(|p| p as f64 / 100.0),
                        "unit": if l.price_per_piece_paise.is_some() { "PIECE" } else { "KG" },
                        "min_weight_kg": l.min_weight_g.map(|g| g as f64 / 1000.0),
                    })
                })
                .collect();
            format!(
                "CURRENT_CARD (version {}):\n{}\nExclusions: {:?}",
                rs.version,
                serde_json::to_string(&lines).unwrap_or_default(),
                rs.exclusions
            )
        }
    }
}

pub fn build_parts(input: &RulesAuthorInput<'_>) -> Vec<Part> {
    let now = input.now.with_timezone(&ist());
    let index: BTreeMap<&String, f64> = input.rate_index_paise_per_kg.iter().map(|(k, v)| (k, *v as f64 / 100.0)).collect();
    vec![
        Part::Text(format!("NOW: {} ({})", now.to_rfc3339(), now.format("%A"))),
        Part::Text(current_summary(&input.current)),
        Part::Text(format!("Local Rate Index, rupees per kg: {}", serde_json::to_string(&index).unwrap_or_default())),
        Part::Text(quote_untrusted("vendor_instruction", &input.instruction)),
        Part::text("Extract the rate-card changes."),
    ]
}

pub async fn author_rules(rt: &AgentRuntime, input: &RulesAuthorInput<'_>) -> AgentResult<RulesDraftResult> {
    if input.instruction.trim().is_empty() {
        return Err(kabados_agent::AgentError::InvalidInput("instruction is empty".into()));
    }
    let validate = validate_with(input.taxonomy);
    let out = rt
        .run_json(JsonTask {
            agent: AGENT,
            prompt_version: PROMPT_V,
            system: system_prompt(input.taxonomy),
            parts: build_parts(input),
            schema: RulesAi::gemini_schema(),
            tier: ModelTier::Pro,
            thinking: true,
            validate: &validate,
            cacheable: false,
        })
        .await?;
    Ok(out.map(|ai| compose(ai, input)))
}

// ---------- deterministic parser used by the offline mock ----------

fn re(p: &str) -> Regex {
    Regex::new(p).unwrap()
}

fn pct_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| re(r"(?i)([+-]|\b(?:increase|raise|up|reduce|decrease|cut|down)\b\s*(?:by)?\s*)\s*(\d+(?:\.\d+)?)\s*%"))
}

fn price_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| re(r"(?i)(?:₹|rs\.?\s*)?(\d+(?:\.\d+)?)(\s*(?:/|per)\s*(?:piece|pc|pcs|unit)|\s*each)?"))
}

fn min_weight_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| re(r"(?i)\b(?:above|over|min(?:imum)?|from)\s+(\d+(?:\.\d+)?)\s*(?:kg|kgs|kilo|kilos|kilogram)"))
}

fn time_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| re(r"(?i)\b(today|tomorrow)\b(?:\s*(?:at|from)?\s*(\d{1,2})(?::(\d{2}))?\s*(am|pm)?)?"))
}

/// Parses simple instructions such as "copper 650, brass 400, paper 14 above 5 kilo, no glass"
/// or "all metals +3% from tomorrow 9am". Deterministic; serves the offline mock.
pub fn heuristic_parse(instruction: &str, now: DateTime<Utc>) -> RulesAi {
    let mut out = RulesAi {
        lines: vec![],
        remove: vec![],
        exclusions: vec![],
        adjustments: vec![],
        effective_from: None,
        replace_all: false,
        ambiguities: vec![],
    };
    if let Some(c) = time_re().captures(instruction) {
        let day = if c[1].eq_ignore_ascii_case("tomorrow") { 1 } else { 0 };
        let mut hour: u32 = c.get(2).and_then(|m| m.as_str().parse().ok()).unwrap_or(9);
        let min: u32 = c.get(3).and_then(|m| m.as_str().parse().ok()).unwrap_or(0);
        if c.get(4).is_some_and(|m| m.as_str().eq_ignore_ascii_case("pm")) && hour < 12 {
            hour += 12;
        }
        let local = now.with_timezone(&ist()).date_naive() + Duration::days(day);
        if let Some(t) = NaiveTime::from_hms_opt(hour.min(23), min.min(59), 0)
            && let Some(dt) = ist().from_local_datetime(&local.and_time(t)).single()
        {
            out.effective_from = Some(dt.to_rfc3339());
        }
    }
    for clause in keywords::clauses(instruction) {
        let lower = clause.to_ascii_lowercase();
        if let Some(rest) =
            lower.strip_prefix("no ").or_else(|| lower.strip_prefix("not buying ")).or_else(|| lower.strip_prefix("stop buying "))
        {
            let glob = match (keywords::find_family(rest), keywords::find_category(rest)) {
                (Some(f), Some((cat, _))) if cat.ends_with(".mixed") || cat.starts_with(&format!("{f}.")) && rest.trim() == f => {
                    format!("{f}.*")
                }
                (_, Some((cat, _))) => cat.to_string(),
                (Some(f), None) => format!("{f}.*"),
                (None, None) => continue,
            };
            if lower.starts_with("stop buying ") {
                out.remove.push(glob)
            } else {
                out.exclusions.push(glob)
            }
            continue;
        }
        if let Some(c) = pct_re().captures(&clause) {
            let sign = match c[1].trim().to_ascii_lowercase().as_str() {
                "-" | "reduce" | "decrease" | "cut" | "down" => -1.0,
                _ => 1.0,
            };
            let pct: f64 = c[2].parse().unwrap_or(0.0) * sign;
            let glob = keywords::find_family(&clause)
                .filter(|_| lower.contains("all") || keywords::find_category(&clause).is_none())
                .map(|f| format!("{f}.*"))
                .or_else(|| keywords::find_category(&clause).map(|(c, _)| c.to_string()));
            if let Some(g) = glob {
                out.adjustments.push(AdjustmentAi { category_glob: g, percent: pct });
            }
            continue;
        }
        let Some((cat, pos)) = keywords::find_category(&clause) else { continue };
        let tail = &clause[pos..];
        let min_weight_kg = min_weight_re().captures(tail).and_then(|c| c[1].parse::<f64>().ok());
        let tail_wo_min = min_weight_re().replace(tail, "");
        let Some(p) = price_re().captures_iter(&tail_wo_min).find(|c| c.get(1).is_some()) else { continue };
        let price_rupees: f64 = p[1].parse().unwrap_or(0.0);
        let piece = p.get(2).is_some() || Taxonomy::seed().get(cat).is_some_and(|c| c.unit == Unit::Piece);
        let mut category_id = cat.to_string();
        if cat == "paper.mixed" && lower.split_whitespace().any(|w| w == "paper") {
            category_id = "paper.newspaper".into();
            out.ambiguities.push("Does 'paper' mean newspaper only, or cardboard too?".into());
        }
        let grade = if cat == "metal.copper" {
            if lower.contains("insulat") {
                Some("with_insulation".to_string())
            } else if lower.contains("mixed") {
                Some("mixed".to_string())
            } else if lower.contains("clean") || lower.contains("bright") || lower.contains("pure") {
                Some("clean".to_string())
            } else {
                None
            }
        } else {
            None
        };
        out.lines.push(RuleLineAi {
            category_id,
            grade,
            unit: if piece { UnitAi::PIECE } else { UnitAi::KG },
            price_rupees,
            min_weight_kg,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use kabados_domain::ruleset::Modifier;

    fn input<'a>(instruction: &str, current: Option<RuleSet>, idx: BTreeMap<String, i64>) -> RulesAuthorInput<'a> {
        RulesAuthorInput {
            instruction: instruction.into(),
            current,
            taxonomy: Taxonomy::seed(),
            rate_index_paise_per_kg: idx,
            hazardous_authorized: false,
            vendor_id: UserId::new(),
            now: Utc.with_ymd_and_hms(2026, 10, 11, 6, 30, 0).unwrap(),
            yard: Some(LatLng { lat: 28.61, lng: 77.21 }),
        }
    }

    #[test]
    fn plan_example_instruction() {
        let inp = input("copper 650, brass 400, paper 14 above 5 kilo, no glass", None, BTreeMap::new());
        let ai = heuristic_parse(&inp.instruction, inp.now);
        let r = compose(ai, &inp);
        let price = |c: &str| r.draft.lines.iter().find(|l| l.category_id == c).and_then(|l| l.price_per_kg_paise);
        assert_eq!(price("metal.copper"), Some(65000));
        assert_eq!(price("metal.brass"), Some(40000));
        assert_eq!(price("paper.newspaper"), Some(1400));
        let paper = r.draft.lines.iter().find(|l| l.category_id == "paper.newspaper").unwrap();
        assert_eq!(paper.min_weight_g, Some(5000));
        assert_eq!(r.draft.exclusions, vec!["glass.*".to_string()]);
        assert_eq!(r.draft.version, 1);
        assert_eq!(r.draft.status, RuleSetStatus::Draft);
        assert!(r.ambiguities.iter().any(|a| a.contains("newspaper only")));
        assert!(r.issues.is_empty(), "{:?}", r.issues);
    }

    #[test]
    fn bulk_change_relative_to_current() {
        let mut current = default_rule_set(&input("x", None, BTreeMap::new()));
        current.version = 7;
        current.status = RuleSetStatus::Published;
        current.lines = vec![
            RuleLine {
                category_id: "metal.copper".into(),
                grade: Some("clean".into()),
                unit: None,
                price_per_kg_paise: Some(65000),
                price_per_piece_paise: None,
                min_weight_g: None,
                conditions: Default::default(),
            },
            RuleLine {
                category_id: "metal.brass".into(),
                grade: None,
                unit: None,
                price_per_kg_paise: Some(40000),
                price_per_piece_paise: None,
                min_weight_g: None,
                conditions: Default::default(),
            },
            RuleLine {
                category_id: "paper.newspaper".into(),
                grade: None,
                unit: None,
                price_per_kg_paise: Some(1400),
                price_per_piece_paise: None,
                min_weight_g: None,
                conditions: Default::default(),
            },
        ];
        current.modifiers = vec![Modifier::VerifiedBonusPct { min_ladder: kabados_domain::LadderLevel::L2CloseUp, value: 3 }];
        let inp = input("all metals +3% from tomorrow 9am", Some(current), BTreeMap::new());
        let ai = heuristic_parse(&inp.instruction, inp.now);
        assert_eq!(ai.adjustments, vec![AdjustmentAi { category_glob: "metal.*".into(), percent: 3.0 }]);
        let r = compose(ai, &inp);
        let price = |c: &str| r.draft.lines.iter().find(|l| l.category_id == c).and_then(|l| l.price_per_kg_paise);
        assert_eq!(price("metal.copper"), Some(66950));
        assert_eq!(price("metal.brass"), Some(41200));
        assert_eq!(price("paper.newspaper"), Some(1400));
        assert_eq!(r.draft.version, 8);
        assert_eq!(r.draft.effective_from.to_rfc3339(), "2026-10-12T09:00:00+05:30");
        assert_eq!(r.draft.modifiers.len(), 1, "modifiers carry over");
    }

    #[test]
    fn far_from_index_and_hazard_issues() {
        let idx: BTreeMap<String, i64> = [("metal.copper".to_string(), 50000)].into();
        let inp = input("copper 900, laptop 300 each, lead acid battery 80", None, idx);
        let r = compose(heuristic_parse(&inp.instruction, inp.now), &inp);
        assert_eq!(r.far_from_index.len(), 1, "{:?}", r.far_from_index);
        let laptop = r.draft.lines.iter().find(|l| l.category_id == "ewaste.itew.laptop").unwrap();
        assert_eq!(laptop.price_per_piece_paise, Some(30000));
        assert!(r.issues.iter().any(|i| i.message.contains("hazardous")), "unauthorized hazardous lines are flagged, not silently dropped");
    }

    #[test]
    fn past_start_time_falls_back_to_now() {
        let inp = input("copper 650", None, BTreeMap::new());
        let mut ai = heuristic_parse(&inp.instruction, inp.now);
        ai.effective_from = Some("2020-01-01T09:00:00+05:30".into());
        let r = compose(ai, &inp);
        assert_eq!(r.draft.effective_from.with_timezone(&Utc), inp.now);
        assert!(r.ambiguities.iter().any(|a| a.contains("past")));
    }

    #[test]
    fn validation_rejects_empty_and_bad_prices() {
        let v = validate_with(Taxonomy::seed());
        let empty = RulesAi {
            lines: vec![],
            remove: vec![],
            exclusions: vec![],
            adjustments: vec![],
            effective_from: None,
            replace_all: false,
            ambiguities: vec![],
        };
        assert!(v(&empty).is_err());
        let mut bad = empty.clone();
        bad.lines.push(RuleLineAi {
            category_id: "metal.copper".into(),
            grade: None,
            unit: UnitAi::KG,
            price_rupees: -5.0,
            min_weight_kg: None,
        });
        assert!(v(&bad).is_err());
    }

    #[test]
    fn adjust_rounds_half_away() {
        assert_eq!(adjust(65000, 3.0), 66950);
        assert_eq!(adjust(1450, -5.0), 1378);
        assert_eq!(adjust(1000, 0.05), 1001);
    }
}
