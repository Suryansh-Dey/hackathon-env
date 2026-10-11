//! Deterministic offline responders for every agent (`AI_MODE=mock`).
//!
//! They read the same prompts the live model receives and apply simple
//! heuristics, so the whole product (and CI evals) runs without network access.

use crate::common::section;
use crate::keywords;
use crate::matching::{FINAL_TOOL, ListingSnapshot, VendorQuote};
use crate::synthetic;
use kabados_agent::{JsonRequest, MockModel, ModelError, Part, ToolCall, ToolStep, Turn};
use kabados_domain::taxonomy::{Taxonomy, Unit};
use regex::Regex;
use serde_json::{Value, json};
use std::sync::OnceLock;

fn texts(parts: &[Part]) -> String {
    parts
        .iter()
        .filter_map(|p| match p {
            Part::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn images(parts: &[Part]) -> Vec<(&str, &[u8])> {
    parts
        .iter()
        .filter_map(|p| match p {
            Part::Image { image_id, bytes, .. } => Some((image_id.as_str(), bytes.as_slice())),
            _ => None,
        })
        .collect()
}

/// Text inside a `quote_untrusted(label, ..)` block.
fn quoted<'a>(text: &'a str, label: &str) -> Option<&'a str> {
    let start = text.find(&format!("<<<{label}"))?;
    let body = &text[start..];
    let nl = body.find('\n')? + 1;
    let end = body.find("\n>>>")?;
    (end >= nl).then(|| &body[nl..end])
}

fn triage(req: &JsonRequest) -> Result<String, ModelError> {
    let mut issues: Vec<&str> = Vec::new();
    match images(&req.parts).first().and_then(|(_, b)| synthetic::image_stats(b)) {
        Some(s) if s.mean_luma < 45.0 => issues.push("DARK"),
        Some(s) if s.std_luma < 6.0 => issues.push("NO_ITEMS"),
        Some(_) => {}
        None => issues.push("NO_ITEMS"),
    }
    let usable = issues.is_empty();
    Ok(json!({"usable": usable, "issues": issues, "retake_hint": if usable { Value::Null } else { json!("Take the photo again in good light with the items in view.") }}).to_string())
}

fn itemize(req: &JsonRequest) -> Result<String, ModelError> {
    let all = texts(&req.parts);
    let first_image = images(&req.parts).first().map(|(id, _)| id.to_string());
    let tax = Taxonomy::seed();
    let mut items: Vec<Value> = Vec::new();
    if let Some(desc) = quoted(&all, "seller_description") {
        for clause in keywords::clauses(desc) {
            let Some((cat_id, pos)) = keywords::find_category(&clause) else { continue };
            let Some(cat) = tax.get(cat_id) else { continue };
            let lower = clause.to_ascii_lowercase();
            let quantity = if cat.unit == Unit::Piece { keywords::quantity_before(&clause, pos).unwrap_or(1) } else { 1 };
            let grade = (cat_id == "metal.copper")
                .then(|| {
                    if lower.contains("insulat") || lower.contains("wire") {
                        Some("with_insulation")
                    } else if lower.contains("clean") || lower.contains("pure") || lower.contains("bright") {
                        Some("clean")
                    } else {
                        None
                    }
                })
                .flatten();
            let mut attributes = vec![];
            if lower.contains("not working") || lower.contains("non-working") || lower.contains("dead") {
                attributes.push(json!({"key": "condition", "value": "non_working"}));
            } else if lower.contains("working") {
                attributes.push(json!({"key": "condition", "value": "working"}));
            }
            items.push(json!({
                "category_id": cat_id,
                "grade": grade,
                "attributes": attributes,
                "quantity": quantity,
                "weight_min_g": Value::Null,
                "weight_max_g": Value::Null,
                "is_hazardous": cat.hazardous,
                "hazard_class": Value::Null,
                "handling": Value::Null,
                "evidence": first_image.iter().map(|id| json!({"image_id": id, "bbox": []})).collect::<Vec<_>>(),
                "confidence": 0.6,
                "note": format!("From description: {}", clause.chars().take(60).collect::<String>()),
            }));
        }
    }
    if items.is_empty() && first_image.is_some() {
        items.push(json!({
            "category_id": "paper.mixed", "grade": null, "attributes": [], "quantity": 1,
            "weight_min_g": null, "weight_max_g": null, "is_hazardous": false, "hazard_class": null, "handling": null,
            "evidence": [{"image_id": first_image, "bbox": []}], "confidence": 0.2,
            "note": "Offline mock: items could not be identified without the AI model."
        }));
    }
    Ok(json!({ "items": items }).to_string())
}

fn verify(req: &JsonRequest) -> Result<String, ModelError> {
    let all = texts(&req.parts);
    let items: Vec<Value> = section(&all, "ITEMS_JSON:").and_then(|s| serde_json::from_str(s).ok()).unwrap_or_default();
    let image_ids: Vec<String> = images(&req.parts).iter().map(|(id, _)| id.to_string()).collect();
    let has_images = !image_ids.is_empty();
    let tax = Taxonomy::seed();
    let mut claims = Vec::new();
    let mut proofs = Vec::new();
    for (n, it) in items.iter().enumerate() {
        let cat = it["category_id"].as_str().unwrap_or("");
        let name = tax.get(cat).map(|c| c.name.as_str()).unwrap_or(cat);
        let key = format!("c{}", n + 1);
        claims.push(json!({
            "claim_key": key, "item_id": it["item_id"], "claim_text": format!("Item is {name}"), "text_ref": "item_list",
            "verdict": if has_images { "SUPPORTED" } else { "INSUFFICIENT_EVIDENCE" },
            "ladder_level": if has_images { "L1_PHOTO_CONSISTENT" } else { "L0_CLAIMED" },
            "rationale": if has_images { "The photos are consistent with this item." } else { "No photo shows this item yet." },
            "evidence_image_ids": if has_images { vec![image_ids[0].clone()] } else { vec![] },
            "confidence": 0.6,
        }));
    }
    let extra: Vec<(&str, &str)> = [("seller_description", "seller_description"), ("claim_reason", "claim_reason")]
        .into_iter()
        .filter_map(|(label, r)| quoted(&all, label).map(|t| (t, r)))
        .collect();
    for (text, text_ref) in extra {
        let l = text.to_ascii_lowercase();
        let shot = if l.contains("pure") || l.contains("clean") || l.contains("copper") {
            Some(("CLOSE_UP", "Take a close-up of the cut end or bare metal in daylight.", "Vendors pay a verified bonus for clean metal."))
        } else if l.contains("brand") || l.contains("model") {
            Some(("LABEL", "Take a photo of the label with the model number.", "A readable label raises the verification level."))
        } else if l.contains("working") {
            Some(("WORKING_DEMO", "Take a photo with the appliance switched on.", "Working items get better offers."))
        } else {
            None
        };
        if let Some((shot, instruction, benefit)) = shot {
            let key = format!("c{}", claims.len() + 1);
            claims.push(json!({
                "claim_key": key, "item_id": null, "claim_text": text.chars().take(120).collect::<String>(), "text_ref": text_ref,
                "verdict": "INSUFFICIENT_EVIDENCE",
                "ladder_level": if has_images { "L1_PHOTO_CONSISTENT" } else { "L0_CLAIMED" },
                "rationale": "Not enough evidence yet; a closer photo would help.",
                "evidence_image_ids": if has_images { vec![image_ids[0].clone()] } else { vec![] },
                "confidence": 0.5,
            }));
            proofs.push(json!({"claim_key": key, "shot_type": shot, "instruction": instruction, "benefit": benefit}));
        }
    }
    Ok(json!({ "claims": claims, "proof_requests": proofs }).to_string())
}

fn rules(req: &JsonRequest) -> Result<String, ModelError> {
    let all = texts(&req.parts);
    let instruction = quoted(&all, "vendor_instruction").unwrap_or("");
    let now = section(&all, "NOW:")
        .and_then(|s| s.split_whitespace().next())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&chrono::Utc))
        .unwrap_or_else(chrono::Utc::now);
    let ai = crate::rules::heuristic_parse(instruction, now);
    serde_json::to_string(&ai).map_err(|e| ModelError::Server(e.to_string()))
}

fn scale(req: &JsonRequest) -> Result<String, ModelError> {
    let decoded = images(&req.parts).first().and_then(|(_, b)| synthetic::decode_scale(b));
    Ok(match decoded {
        Some(d) => json!({"reading_text": d.text, "unit": d.unit.unwrap_or_else(|| "kg".into()), "display_type": "7-segment", "confidence": 0.95, "display_bbox": []}),
        None => json!({"reading_text": null, "unit": null, "display_type": "unknown", "confidence": 0.0, "display_bbox": []}),
    }
    .to_string())
}

fn attack_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"(?i)(ignore (all |any )?(previous|prior|above) (instructions|rules)|system prompt|you are now|disregard (the )?(rules|instructions)|act as|developer mode|</?(system|assistant)>|set (the )?(price|verdict)|mark (this|it) as (supported|verified))").unwrap()
    })
}

fn abuse_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?i)\b(idiot|stupid|bastard|moron|kill you|i will hurt|chutiya|harami|bitch)\b").unwrap())
}

fn classify(req: &JsonRequest) -> Result<String, ModelError> {
    let all = texts(&req.parts);
    let text = quoted(&all, "user_text").unwrap_or("");
    let mut reasons = vec![];
    let attack = attack_re().is_match(text);
    if attack {
        reasons.push("Tries to instruct the AI system.".to_string());
    }
    let abusive = abuse_re().is_match(text);
    if abusive {
        reasons.push("Contains insulting or threatening language.".to_string());
    }
    Ok(json!({"prompt_attack": attack, "abusive": abusive, "reasons": reasons}).to_string())
}

fn last_result<'a>(transcript: &'a [Turn], name: &str) -> Option<&'a Value> {
    transcript.iter().rev().find_map(|t| match t {
        Turn::ToolResults(rs) => rs.iter().rev().find(|r| r.name == name).map(|r| &r.response),
        _ => None,
    })
}

fn call(name: &str, args: Value) -> ToolStep {
    ToolStep::Calls { calls: vec![ToolCall { name: name.into(), args }], raw: None }
}

fn matching(_req: &kabados_agent::ToolRequest, transcript: &[Turn]) -> Result<ToolStep, ModelError> {
    let first = transcript.iter().find_map(|t| match t {
        Turn::User(p) => Some(texts(p)),
        _ => None,
    });
    let request: Value = first
        .as_deref()
        .and_then(|t| t.split_once("MATCH_REQUEST:"))
        .and_then(|(_, j)| serde_json::from_str(j.trim()).ok())
        .ok_or_else(|| ModelError::InvalidInput("no MATCH_REQUEST".into()))?;
    let (listing_id, ver) = (request["listing_id"].clone(), request["ver"].clone());
    let Some(snapshot) = last_result(transcript, "get_listing_snapshot") else {
        return Ok(call("get_listing_snapshot", json!({"listing_id": listing_id, "ver": ver})));
    };
    let snapshot: ListingSnapshot =
        serde_json::from_value(snapshot.clone()).map_err(|e| ModelError::InvalidInput(format!("snapshot: {e}")))?;
    let mut cats: Vec<String> = snapshot.items.iter().map(|i| i.category_id.clone()).collect();
    cats.sort();
    cats.dedup();
    let widened = last_result(transcript, "widen_search");
    let candidates: Vec<String> = match last_result(transcript, "find_candidate_vendors") {
        None => return Ok(call("find_candidate_vendors", json!({"h3_cells": [snapshot.h3_r8], "category_ids": cats}))),
        Some(v) => v["vendors"]
            .as_array()
            .map(|a| a.iter().filter_map(|x| x["vendor_id"].as_str().map(String::from)).collect())
            .unwrap_or_default(),
    };
    if candidates.len() < 3 && widened.is_none() {
        return Ok(call("widen_search", json!({"listing_id": listing_id, "ver": ver, "step": 1})));
    }
    if let Some(w) = widened {
        let searched_wide =
            transcript.iter().filter(|t| matches!(t, Turn::ToolResults(r) if r.iter().any(|x| x.name == "find_candidate_vendors"))).count()
                >= 2;
        if !searched_wide && w["allowed"].as_bool() == Some(true) {
            return Ok(call("find_candidate_vendors", json!({"h3_cells": w["h3_cells"], "category_ids": cats})));
        }
    }
    let quotes: Vec<VendorQuote> = match last_result(transcript, "quote") {
        None if !candidates.is_empty() => {
            return Ok(call("quote", json!({"listing_id": listing_id, "ver": ver, "vendor_ids": candidates})));
        }
        None => vec![],
        Some(v) => serde_json::from_value(v["quotes"].clone()).unwrap_or_default(),
    };
    let mixed_hazard = snapshot.contains_hazardous && snapshot.items.iter().any(|i| !i.hazardous);
    if mixed_hazard && last_result(transcript, "suggest_split").is_none() {
        return Ok(call("suggest_split", json!({"listing_id": listing_id, "ver": ver})));
    }
    let low_ladder = snapshot.items.iter().any(|i| i.ladder == "L0_CLAIMED");
    if low_ladder && last_result(transcript, "suggest_proof_unlocks").is_none() {
        return Ok(call("suggest_proof_unlocks", json!({"listing_id": listing_id, "ver": ver})));
    }
    let mut eligible: Vec<&VendorQuote> = quotes.iter().filter(|q| q.eligible).collect();
    eligible.sort_by(|a, b| {
        b.quote_high_paise.cmp(&a.quote_high_paise).then(b.coverage_permille.cmp(&a.coverage_permille)).then(a.vendor_id.cmp(&b.vendor_id))
    });
    let n = snapshot.items.len();
    let vendors: Vec<Value> = eligible
        .iter()
        .take(5)
        .map(|q| {
            json!({
                "vendor_id": q.vendor_id, "quote_low_paise": q.quote_low_paise, "quote_high_paise": q.quote_high_paise,
                "coverage_permille": q.coverage_permille,
                "reasons": [format!("covers {} of {} items", q.covered_item_ids.len(), n)],
            })
        })
        .collect();
    let split_plan = last_result(transcript, "suggest_split").map(|s| s["groups"].clone()).unwrap_or(json!([]));
    let proofs = last_result(transcript, "suggest_proof_unlocks").map(|s| s["suggestions"].clone()).unwrap_or(json!([]));
    Ok(call(FINAL_TOOL, json!({"vendors": vendors, "split_plan": split_plan, "proof_suggestions": proofs})))
}

/// A mock model with deterministic responders for every KabadOS agent.
pub fn mock_model() -> MockModel {
    MockModel::new()
        .with_json(crate::triage::AGENT, triage)
        .with_json(crate::itemize::AGENT, itemize)
        .with_json(crate::verify::AGENT, verify)
        .with_json(crate::rules::AGENT, rules)
        .with_json(crate::scale::AGENT, scale)
        .with_json(crate::classify::AGENT, classify)
        .with_tools(crate::matching::AGENT, matching)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoted_block_extraction() {
        let t = kabados_domain::redact::quote_untrusted("seller_description", "two ACs");
        assert_eq!(quoted(&t, "seller_description"), Some("two ACs"));
        assert_eq!(quoted(&t, "claim_reason"), None);
    }
}
