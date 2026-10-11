//! Agent 4: matching orchestrator (PLAN.md §11.6).
//!
//! Runs only on the slow path (few matches, low coverage, mixed hazardous
//! lots). Tools are in-process calls into deterministic code. The model may
//! choose and explain, but every vendor and number in the result must come
//! from tool results.

use crate::common::COMMON_RULES;
use async_trait::async_trait;
use gemini_client_api::gemini::utils::{GeminiSchema, gemini_schema};
use kabados_agent::{AgentResult, AgentRuntime, ModelTier, Part, ToolCall, ToolDecl, ToolDispatcher, ToolTask};
use kabados_domain::ids::ListingId;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

pub const AGENT: &str = "matching";
pub const PROMPT_V: &str = "matching-2026-10-11";
pub const MAX_VENDORS: usize = 10;
pub const FINAL_TOOL: &str = "submit_match_result";

// ---------- tool arguments (model-facing schemas) ----------

/// Arguments for get_listing_snapshot.
#[gemini_schema]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ListingRefArgs {
    pub listing_id: String,
    pub ver: u32,
}

/// Arguments for find_candidate_vendors.
#[gemini_schema]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FindCandidatesArgs {
    /// H3 resolution-8 cells to search.
    pub h3_cells: Vec<String>,
    /// Category IDs to cover.
    pub category_ids: Vec<String>,
}

/// Arguments for quote.
#[gemini_schema]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QuoteArgs {
    pub listing_id: String,
    pub ver: u32,
    pub vendor_ids: Vec<String>,
}

/// Arguments for vendor_signals.
#[gemini_schema]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VendorIdsArgs {
    pub vendor_ids: Vec<String>,
}

/// Arguments for widen_search.
#[gemini_schema]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WidenArgs {
    pub listing_id: String,
    pub ver: u32,
    /// Ring step to add (1 = next ring of cells).
    pub step: u32,
}

/// A vendor chosen for the seller, with reasons.
#[gemini_schema]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MatchedVendorAi {
    pub vendor_id: String,
    /// Copy exactly from the quote tool result.
    pub quote_low_paise: i64,
    /// Copy exactly from the quote tool result.
    pub quote_high_paise: i64,
    /// Copy exactly from the quote tool result.
    pub coverage_permille: u32,
    /// Short reasons, e.g. "covers 4 of 5 items", "authorized for e-waste".
    pub reasons: Vec<String>,
}

/// Items to send to a different set of vendors.
#[gemini_schema]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SplitGroupAi {
    pub item_ids: Vec<String>,
    pub vendor_ids: Vec<String>,
    pub reason: String,
}

/// Final answer.
#[gemini_schema]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MatchResultAi {
    /// Best vendors first, at most 10, only eligible vendors from quote results.
    pub vendors: Vec<MatchedVendorAi>,
    /// Optional split of a mixed lot across specialist vendors; empty if not needed.
    pub split_plan: Vec<SplitGroupAi>,
    /// Photos or proof the seller could add to get better offers.
    pub proof_suggestions: Vec<String>,
}

// ---------- tool results (deterministic code → model) ----------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SnapshotItem {
    pub item_id: String,
    pub category_id: String,
    pub grade: Option<String>,
    pub quantity: u32,
    pub est_weight_g: i64,
    pub ladder: String,
    pub hazardous: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ListingSnapshot {
    pub listing_id: String,
    pub ver: u32,
    pub items: Vec<SnapshotItem>,
    pub h3_r8: String,
    pub contains_hazardous: bool,
    pub floor: i32,
    pub lift_available: bool,
    pub preferred_days: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CandidateVendor {
    pub vendor_id: String,
    pub rule_set_version: u32,
    pub categories: Vec<String>,
    pub hazardous_authorized: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VendorQuote {
    pub vendor_id: String,
    pub eligible: bool,
    pub quote_low_paise: i64,
    pub quote_high_paise: i64,
    pub coverage_permille: u32,
    pub covered_item_ids: Vec<String>,
    pub explanation_codes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VendorSignal {
    pub vendor_id: String,
    pub reliability: f64,
    pub rating: f64,
    pub response_minutes: Option<u32>,
    pub current_load: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WidenResult {
    pub allowed: bool,
    pub h3_cells: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SplitGroup {
    pub item_ids: Vec<String>,
    pub vendor_ids: Vec<String>,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SplitSuggestion {
    pub groups: Vec<SplitGroup>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProofUnlocks {
    pub suggestions: Vec<String>,
}

/// In-process tools over deterministic code (pricing crate, repositories, Cedar).
#[async_trait]
pub trait MatchingTools: Send + Sync {
    async fn get_listing_snapshot(&self, listing_id: &str, ver: u32) -> Result<ListingSnapshot, String>;
    async fn find_candidate_vendors(&self, h3_cells: &[String], category_ids: &[String]) -> Result<Vec<CandidateVendor>, String>;
    async fn quote(&self, listing_id: &str, ver: u32, vendor_ids: &[String]) -> Result<Vec<VendorQuote>, String>;
    async fn vendor_signals(&self, vendor_ids: &[String]) -> Result<Vec<VendorSignal>, String>;
    async fn widen_search(&self, listing_id: &str, ver: u32, step: u32) -> Result<WidenResult, String>;
    async fn suggest_split(&self, listing_id: &str, ver: u32) -> Result<SplitSuggestion, String>;
    async fn suggest_proof_unlocks(&self, listing_id: &str, ver: u32) -> Result<ProofUnlocks, String>;
}

// ---------- final result ----------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MatchedVendor {
    pub vendor_id: String,
    pub quote_low_paise: i64,
    pub quote_high_paise: i64,
    pub coverage_permille: u32,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MatchResult {
    pub vendors: Vec<MatchedVendor>,
    pub split_plan: Option<Vec<SplitGroup>>,
    pub proof_suggestions: Vec<String>,
}

pub fn tool_decls() -> Vec<ToolDecl> {
    let d = |name: &str, description: &str, parameters: Value| ToolDecl { name: name.into(), description: description.into(), parameters };
    vec![
        d(
            "get_listing_snapshot",
            "Items, weights, ladder levels, location cell and pickup preferences of a listing version.",
            ListingRefArgs::gemini_schema(),
        ),
        d(
            "find_candidate_vendors",
            "Vendors whose rate cards cover the given categories in the given H3 cells (coverage index).",
            FindCandidatesArgs::gemini_schema(),
        ),
        d(
            "quote",
            "Deterministic quotes from the vendors' published rate cards: eligibility, value range in paise and coverage.",
            QuoteArgs::gemini_schema(),
        ),
        d("vendor_signals", "Reliability, rating, response time and current load for vendors.", VendorIdsArgs::gemini_schema()),
        d("widen_search", "Next ring of H3 cells around the listing, within policy limits.", WidenArgs::gemini_schema()),
        d(
            "suggest_split",
            "Splits a mixed lot across specialist vendors (e.g. e-waste to an authorized recycler, paper to a kabadiwala).",
            ListingRefArgs::gemini_schema(),
        ),
        d(
            "suggest_proof_unlocks",
            "Proof photos that would raise ladder levels and unlock verified bonuses.",
            ListingRefArgs::gemini_schema(),
        ),
    ]
}

fn final_decl() -> ToolDecl {
    ToolDecl {
        name: FINAL_TOOL.into(),
        description: "Submit the final match result. Vendors and numbers must come from quote results.".into(),
        parameters: MatchResultAi::gemini_schema(),
    }
}

const SYSTEM: &str = "You match a scrap listing to vendors on a recycling marketplace. Use the tools; they run deterministic code.
Steps: get the listing snapshot, find candidate vendors for its cells and categories, get quotes, optionally check vendor signals.
If fewer than 3 vendors are eligible, call widen_search and look again (at most twice).
If the lot mixes hazardous items (e-waste, batteries) with ordinary scrap, call suggest_split so hazardous items go only to authorized vendors.
If items are at low verification levels, call suggest_proof_unlocks.
Finally call submit_match_result with the best eligible vendors first (higher payout, better coverage, closer and more reliable).
Never invent vendor IDs or numbers: copy quote_low_paise, quote_high_paise and coverage_permille exactly from quote results, and include only vendors whose quote has eligible = true.";

/// Records tool results so the final answer can be checked against them.
struct Recorder<'a> {
    tools: &'a dyn MatchingTools,
    quotes: Mutex<BTreeMap<String, VendorQuote>>,
    item_ids: Mutex<BTreeSet<String>>,
}

fn args<T: DeserializeOwned>(call: &ToolCall) -> Result<T, String> {
    serde_json::from_value(call.args.clone()).map_err(|e| format!("bad arguments for {}: {e}", call.name))
}

fn to_value<T: Serialize>(v: T) -> Result<Value, String> {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

#[async_trait]
impl ToolDispatcher for Recorder<'_> {
    async fn dispatch(&self, call: &ToolCall) -> Result<Value, String> {
        match call.name.as_str() {
            "get_listing_snapshot" => {
                let a: ListingRefArgs = args(call)?;
                let s = self.tools.get_listing_snapshot(&a.listing_id, a.ver).await?;
                self.item_ids.lock().unwrap().extend(s.items.iter().map(|i| i.item_id.clone()));
                to_value(s)
            }
            "find_candidate_vendors" => {
                let a: FindCandidatesArgs = args(call)?;
                to_value(serde_json::json!({ "vendors": self.tools.find_candidate_vendors(&a.h3_cells, &a.category_ids).await? }))
            }
            "quote" => {
                let a: QuoteArgs = args(call)?;
                let qs = self.tools.quote(&a.listing_id, a.ver, &a.vendor_ids).await?;
                let mut rec = self.quotes.lock().unwrap();
                for q in &qs {
                    rec.insert(q.vendor_id.clone(), q.clone());
                }
                to_value(serde_json::json!({ "quotes": qs }))
            }
            "vendor_signals" => {
                let a: VendorIdsArgs = args(call)?;
                to_value(serde_json::json!({ "signals": self.tools.vendor_signals(&a.vendor_ids).await? }))
            }
            "widen_search" => {
                let a: WidenArgs = args(call)?;
                to_value(self.tools.widen_search(&a.listing_id, a.ver, a.step).await?)
            }
            "suggest_split" => {
                let a: ListingRefArgs = args(call)?;
                to_value(self.tools.suggest_split(&a.listing_id, a.ver).await?)
            }
            "suggest_proof_unlocks" => {
                let a: ListingRefArgs = args(call)?;
                to_value(self.tools.suggest_proof_unlocks(&a.listing_id, a.ver).await?)
            }
            other => Err(format!("unknown tool {other}")),
        }
    }
}

impl Recorder<'_> {
    fn check(&self, r: &MatchResultAi) -> Result<(), String> {
        let quotes = self.quotes.lock().unwrap();
        if r.vendors.len() > MAX_VENDORS {
            return Err(format!("return at most {MAX_VENDORS} vendors"));
        }
        let mut seen = BTreeSet::new();
        for v in &r.vendors {
            if !seen.insert(&v.vendor_id) {
                return Err(format!("vendor {} listed twice", v.vendor_id));
            }
            let Some(q) = quotes.get(&v.vendor_id) else {
                return Err(format!("vendor {} did not come from a quote result; call quote first", v.vendor_id));
            };
            if !q.eligible {
                return Err(format!("vendor {} is not eligible (codes {:?})", v.vendor_id, q.explanation_codes));
            }
            if (v.quote_low_paise, v.quote_high_paise, v.coverage_permille) != (q.quote_low_paise, q.quote_high_paise, q.coverage_permille)
            {
                return Err(format!(
                    "numbers for {} must equal the quote result: low {}, high {}, coverage {}",
                    v.vendor_id, q.quote_low_paise, q.quote_high_paise, q.coverage_permille
                ));
            }
        }
        let items = self.item_ids.lock().unwrap();
        for g in &r.split_plan {
            if let Some(i) = g.item_ids.iter().find(|i| !items.contains(*i)) {
                return Err(format!("split plan references unknown item {i}"));
            }
            if let Some(v) = g.vendor_ids.iter().find(|v| !quotes.get(*v).is_some_and(|q| q.eligible || q.coverage_permille > 0)) {
                return Err(format!("split plan references vendor {v} without a quote"));
            }
        }
        Ok(())
    }
}

pub async fn run_matching_agent(
    rt: &AgentRuntime,
    tools: &dyn MatchingTools,
    listing_id: &ListingId,
    ver: u32,
) -> AgentResult<MatchResult> {
    let recorder = Recorder { tools, quotes: Mutex::new(BTreeMap::new()), item_ids: Mutex::new(BTreeSet::new()) };
    let validate = |r: &MatchResultAi| recorder.check(r);
    let initial = vec![Part::Text(format!(
        "MATCH_REQUEST: {}",
        serde_json::json!({ "listing_id": listing_id.as_str(), "ver": ver, "target_vendors": 5 })
    ))];
    let out = rt
        .run_tools(
            ToolTask {
                agent: AGENT,
                prompt_version: PROMPT_V,
                system: format!("{SYSTEM}\n\n{COMMON_RULES}"),
                initial,
                tools: tool_decls(),
                final_tool: final_decl(),
                tier: ModelTier::Pro,
                thinking: true,
                validate: &validate,
            },
            &recorder,
        )
        .await?;
    let quotes = recorder.quotes.lock().unwrap().clone();
    Ok(out.map(|ai| MatchResult {
        // Numbers always come from the recorded tool results.
        vendors: ai
            .vendors
            .into_iter()
            .filter_map(|v| {
                let q = quotes.get(&v.vendor_id)?;
                Some(MatchedVendor {
                    vendor_id: v.vendor_id,
                    quote_low_paise: q.quote_low_paise,
                    quote_high_paise: q.quote_high_paise,
                    coverage_permille: q.coverage_permille,
                    reasons: v.reasons.into_iter().take(5).collect(),
                })
            })
            .collect(),
        split_plan: if ai.split_plan.is_empty() {
            None
        } else {
            Some(
                ai.split_plan
                    .into_iter()
                    .map(|g| SplitGroup { item_ids: g.item_ids, vendor_ids: g.vendor_ids, reason: g.reason })
                    .collect(),
            )
        },
        proof_suggestions: ai.proof_suggestions.into_iter().take(5).collect(),
    }))
}

/// In-memory tools for tests, evals and local development.
#[derive(Clone, Debug, Default)]
pub struct FakeMatchingTools {
    pub snapshot: Option<ListingSnapshot>,
    /// (candidate, quote, signal) per vendor; `near` vendors are found in the first ring.
    pub vendors: Vec<(CandidateVendor, VendorQuote, VendorSignal, bool)>,
    pub split: Option<SplitSuggestion>,
    pub proof_unlocks: Vec<String>,
}

#[async_trait]
impl MatchingTools for FakeMatchingTools {
    async fn get_listing_snapshot(&self, listing_id: &str, ver: u32) -> Result<ListingSnapshot, String> {
        self.snapshot.clone().filter(|s| s.listing_id == listing_id && s.ver == ver).ok_or_else(|| "listing not found".into())
    }

    async fn find_candidate_vendors(&self, h3_cells: &[String], category_ids: &[String]) -> Result<Vec<CandidateVendor>, String> {
        let widened = h3_cells.len() > 1;
        Ok(self
            .vendors
            .iter()
            .filter(|(c, _, _, near)| (*near || widened) && c.categories.iter().any(|x| category_ids.contains(x)))
            .map(|(c, ..)| c.clone())
            .collect())
    }

    async fn quote(&self, _listing_id: &str, _ver: u32, vendor_ids: &[String]) -> Result<Vec<VendorQuote>, String> {
        Ok(self.vendors.iter().filter(|(c, ..)| vendor_ids.contains(&c.vendor_id)).map(|(_, q, ..)| q.clone()).collect())
    }

    async fn vendor_signals(&self, vendor_ids: &[String]) -> Result<Vec<VendorSignal>, String> {
        Ok(self.vendors.iter().filter(|(c, ..)| vendor_ids.contains(&c.vendor_id)).map(|(_, _, s, _)| s.clone()).collect())
    }

    async fn widen_search(&self, _listing_id: &str, _ver: u32, step: u32) -> Result<WidenResult, String> {
        let base = self.snapshot.as_ref().map(|s| s.h3_r8.clone()).unwrap_or_default();
        Ok(WidenResult { allowed: step <= 2, h3_cells: (0..=step * 6).map(|i| format!("{base}-{i}")).collect() })
    }

    async fn suggest_split(&self, _listing_id: &str, _ver: u32) -> Result<SplitSuggestion, String> {
        Ok(self.split.clone().unwrap_or(SplitSuggestion { groups: vec![] }))
    }

    async fn suggest_proof_unlocks(&self, _listing_id: &str, _ver: u32) -> Result<ProofUnlocks, String> {
        Ok(ProofUnlocks { suggestions: self.proof_unlocks.clone() })
    }
}
