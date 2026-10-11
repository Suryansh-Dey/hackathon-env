//! Agent 3: claim verification and proof requests (PLAN.md §11.5).
//!
//! Deterministic post-checks: SUPPORTED needs an evidence image; the ladder
//! moves only from images (L3 needs label text read from an evidence image);
//! the AI never assigns L4; at most 3 proof requests per round and none after
//! round 2; neutral language only.

use crate::common::*;
use gemini_client_api::gemini::utils::{GeminiSchema, gemini_schema};
use kabados_agent::{AgentResult, AgentRuntime, JsonTask, ModelTier, Part};
use kabados_domain::Paise;
use kabados_domain::ids::ItemId;
use kabados_domain::ladder::LadderLevel;
use kabados_domain::listing::{Evidence, Item, MAX_PROOF_ROUNDS, MAX_PROOFS_PER_ROUND, ShotType, Verdict};
use kabados_domain::redact::quote_untrusted;
use kabados_domain::taxonomy::Taxonomy;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const AGENT: &str = "verify";
pub const PROMPT_V: &str = "verify-2026-10-11";

#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[gemini_schema]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VerdictAi {
    SUPPORTED,
    INSUFFICIENT_EVIDENCE,
    INCONSISTENT,
}

#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[gemini_schema]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LadderAi {
    L0_CLAIMED,
    L1_PHOTO_CONSISTENT,
    L2_CLOSE_UP,
    L3_IDENTIFIER,
}

#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[gemini_schema]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShotTypeAi {
    CLOSE_UP,
    LABEL,
    CUT_SECTION,
    COMPONENT_SIDE,
    WORKING_DEMO,
    SCALE,
}

/// Assessment of one seller claim.
#[gemini_schema]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClaimAi {
    /// Short key you assign, e.g. "c1".
    pub claim_key: String,
    /// Item ID the claim is about, if any.
    pub item_id: Option<String>,
    /// The claim in the seller's words, e.g. "pure copper winding".
    pub claim_text: String,
    /// Where the claim came from, e.g. "seller_description" or "claim_reason" or "item_list".
    pub text_ref: Option<String>,
    pub verdict: VerdictAi,
    /// Evidence level shown by the photos (never from text alone).
    pub ladder_level: LadderAi,
    /// One or two neutral sentences explaining the verdict.
    pub rationale: String,
    /// Image IDs that show the evidence.
    pub evidence_image_ids: Vec<String>,
    /// 0 to 1.
    pub confidence: f32,
}

/// A specific photo that would raise the evidence level.
#[gemini_schema]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProofAi {
    pub claim_key: String,
    pub shot_type: ShotTypeAi,
    /// Concrete instruction, e.g. "Take a close-up of the cut end of the coil in daylight."
    pub instruction: String,
    /// Why it helps the seller, e.g. "Vendors pay a verified bonus for clean copper."
    pub benefit: String,
}

#[gemini_schema]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VerifyAi {
    pub claims: Vec<ClaimAi>,
    pub proof_requests: Vec<ProofAi>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClaimDraft {
    pub key: String,
    pub item_id: Option<ItemId>,
    pub text: String,
    pub text_ref: Option<String>,
    pub verdict: Verdict,
    pub ladder_level: LadderLevel,
    pub rationale: String,
    pub evidence: Vec<Evidence>,
    pub confidence: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProofRequestDraft {
    pub claim_key: String,
    pub shot_type: ShotType,
    pub instruction: String,
    pub benefit: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VerifyOutput {
    pub claims: Vec<ClaimDraft>,
    pub proof_requests: Vec<ProofRequestDraft>,
    /// Deterministic corrections applied (downgrades, dropped requests).
    pub corrections: Vec<String>,
}

pub struct VerifyInput {
    pub items: Vec<Item>,
    pub images: Vec<ImageInput>,
    pub description: Option<String>,
    pub asking_price: Option<Paise>,
    pub claim_reason: Option<String>,
    /// Proof rounds already completed for this listing version.
    pub prior_rounds: u8,
    /// Text detected on labels and model plates (Textract), keyed by image ID.
    pub label_text: BTreeMap<String, String>,
}

const SYSTEM: &str = "You review seller claims about scrap items for a recycling marketplace, using only the photos as evidence.
Claims come from the item list (category, grade, attributes such as working condition or brand), the seller's description and the reason given for the asking price.
For each claim decide:
- SUPPORTED: the photos show it. Cite the image IDs.
- INSUFFICIENT_EVIDENCE: the photos neither show nor contradict it. This is the normal outcome when proof is missing; it is not an accusation.
- INCONSISTENT: the photos show something different (e.g. aluminium wire claimed as copper). Explain neutrally what is visible.
Evidence levels (ladder), set only from what images show:
- L0_CLAIMED: no supporting image.
- L1_PHOTO_CONSISTENT: photos are consistent with the claim.
- L2_CLOSE_UP: a close-up shows the material, colour or grade.
- L3_IDENTIFIER: a label, model number or markings are readable and consistent (use the detected label text when given).
Ask for proof only where a specific photo would raise the level of a valuable claim: at most 3 requests. Each request names one shot type, gives a concrete instruction and says how it helps the seller (for example, vendors pay a verified bonus for clean copper).";

fn system_prompt(tax: &Taxonomy) -> String {
    format!("{SYSTEM}\n\n{COMMON_RULES}\n\n{}", taxonomy_prompt(tax))
}

fn validate(ai: &VerifyAi) -> Result<(), String> {
    let keys: BTreeSet<&str> = ai.claims.iter().map(|c| c.claim_key.as_str()).collect();
    if keys.len() != ai.claims.len() {
        return Err("claim_key values must be unique".into());
    }
    for c in &ai.claims {
        if c.rationale.trim().is_empty() {
            return Err(format!("claim {} needs a rationale", c.claim_key));
        }
        for t in [&c.rationale, &c.claim_text] {
            if let Some(w) = neutral_language_violation(t) {
                return Err(format!("use neutral language: replace {w:?} with \"not enough evidence yet\" or describe what is visible"));
            }
        }
    }
    for p in &ai.proof_requests {
        for t in [&p.instruction, &p.benefit] {
            if let Some(w) = neutral_language_violation(t) {
                return Err(format!("use neutral language in proof requests (found {w:?})"));
            }
        }
        if mentions_price(&p.instruction) {
            return Err("proof instructions must not mention prices".into());
        }
    }
    Ok(())
}

fn ladder(l: LadderAi) -> LadderLevel {
    match l {
        LadderAi::L0_CLAIMED => LadderLevel::L0Claimed,
        LadderAi::L1_PHOTO_CONSISTENT => LadderLevel::L1PhotoConsistent,
        LadderAi::L2_CLOSE_UP => LadderLevel::L2CloseUp,
        LadderAi::L3_IDENTIFIER => LadderLevel::L3Identifier,
    }
}

fn shot(s: ShotTypeAi) -> ShotType {
    match s {
        ShotTypeAi::CLOSE_UP => ShotType::CloseUp,
        ShotTypeAi::LABEL => ShotType::Label,
        ShotTypeAi::CUT_SECTION => ShotType::CutSection,
        ShotTypeAi::COMPONENT_SIDE => ShotType::ComponentSide,
        ShotTypeAi::WORKING_DEMO => ShotType::WorkingDemo,
        ShotTypeAi::SCALE => ShotType::Scale,
    }
}

/// Deterministic post-checks (PLAN.md §11.5).
pub fn postprocess(ai: VerifyAi, input: &VerifyInput) -> VerifyOutput {
    let image_ids: BTreeSet<String> = input.images.iter().map(|i| i.image_id.to_string()).collect();
    let item_ids: BTreeSet<String> = input.items.iter().map(|i| i.id.to_string()).collect();
    let mut corrections = Vec::new();
    let mut claims = Vec::new();
    for c in ai.claims {
        let evidence: Vec<Evidence> = c
            .evidence_image_ids
            .iter()
            .filter(|id| image_ids.contains(*id))
            .filter_map(|id| id.parse().ok())
            .map(|image_id| Evidence { image_id, bbox: None })
            .collect();
        let mut verdict = match c.verdict {
            VerdictAi::SUPPORTED => Verdict::Supported,
            VerdictAi::INSUFFICIENT_EVIDENCE => Verdict::InsufficientEvidence,
            VerdictAi::INCONSISTENT => Verdict::Inconsistent,
        };
        let mut level = ladder(c.ladder_level);
        if verdict == Verdict::Supported && evidence.is_empty() {
            corrections.push(format!("{}: SUPPORTED without evidence image → INSUFFICIENT_EVIDENCE", c.claim_key));
            verdict = Verdict::InsufficientEvidence;
        }
        // The ladder moves only from images.
        if evidence.is_empty() && level > LadderLevel::L0Claimed {
            corrections.push(format!("{}: ladder capped at L0 (no evidence image)", c.claim_key));
            level = LadderLevel::L0Claimed;
        }
        if level == LadderLevel::L3Identifier && !evidence.iter().any(|e| input.label_text.contains_key(e.image_id.as_str())) {
            corrections.push(format!("{}: L3 needs label text from an evidence image → L2", c.claim_key));
            level = LadderLevel::L2CloseUp;
        }
        match verdict {
            Verdict::Inconsistent if level > LadderLevel::L0Claimed => level = LadderLevel::L0Claimed,
            Verdict::InsufficientEvidence if level > LadderLevel::L1PhotoConsistent => level = LadderLevel::L1PhotoConsistent,
            _ => {}
        }
        let item_id = c.item_id.filter(|id| item_ids.contains(id)).and_then(|id| id.parse().ok());
        claims.push(ClaimDraft {
            key: c.claim_key,
            item_id,
            text: c.claim_text.chars().take(300).collect(),
            text_ref: c.text_ref,
            verdict,
            ladder_level: level,
            rationale: c.rationale.chars().take(600).collect(),
            evidence,
            confidence: clamp01(c.confidence),
        });
    }

    let mut proof_requests = Vec::new();
    if input.prior_rounds >= MAX_PROOF_ROUNDS {
        if !ai.proof_requests.is_empty() {
            corrections.push(format!("dropped proof requests: {MAX_PROOF_ROUNDS} rounds already done"));
        }
    } else {
        let open: BTreeSet<&str> = claims.iter().filter(|c| c.ladder_level < LadderLevel::L3Identifier).map(|c| c.key.as_str()).collect();
        for p in ai.proof_requests {
            if !open.contains(p.claim_key.as_str()) {
                corrections.push(format!("dropped proof request for unknown or fully verified claim {}", p.claim_key));
                continue;
            }
            if proof_requests.len() == MAX_PROOFS_PER_ROUND {
                corrections.push(format!("dropped proof requests beyond {MAX_PROOFS_PER_ROUND} per round"));
                break;
            }
            proof_requests.push(ProofRequestDraft {
                claim_key: p.claim_key,
                shot_type: shot(p.shot_type),
                instruction: p.instruction.chars().take(300).collect(),
                benefit: p.benefit.chars().take(200).collect(),
            });
        }
    }
    VerifyOutput { claims, proof_requests, corrections }
}

#[derive(Serialize)]
struct ItemForPrompt<'a> {
    item_id: &'a str,
    category_id: &'a str,
    grade: Option<&'a str>,
    quantity: u32,
    attributes: &'a BTreeMap<String, kabados_domain::listing::AttrValue>,
    estimated_weight_g: Option<i64>,
}

pub fn build_parts(input: &VerifyInput) -> Vec<Part> {
    let items: Vec<ItemForPrompt<'_>> = input
        .items
        .iter()
        .map(|i| ItemForPrompt {
            item_id: i.id.as_str(),
            category_id: &i.category_id,
            grade: i.grade.as_deref(),
            quantity: i.quantity,
            attributes: &i.attributes,
            estimated_weight_g: i.est_weight_g.map(|g| g.0),
        })
        .collect();
    let ids: Vec<String> = input.images.iter().map(|i| i.image_id.to_string()).collect();
    let mut parts = vec![
        Part::Text(format!("ITEMS_JSON:\n{}", serde_json::to_string(&items).unwrap_or_default())),
        Part::Text(format!("IMAGES: {}", if ids.is_empty() { "none".into() } else { ids.join(", ") })),
    ];
    parts.extend(image_parts(&input.images));
    if !input.label_text.is_empty() {
        let mut s = String::from("Label text detected by OCR (untrusted):\n");
        for (img, text) in &input.label_text {
            s.push_str(&format!("{img}: {}\n", quote_untrusted("label_text", text)));
        }
        parts.push(Part::Text(s));
    }
    if let Some(d) = input.description.as_deref().filter(|d| !d.trim().is_empty()) {
        parts.push(Part::Text(quote_untrusted("seller_description", d)));
    }
    if let Some(r) = input.claim_reason.as_deref().filter(|d| !d.trim().is_empty()) {
        parts.push(Part::Text(quote_untrusted("claim_reason", r)));
    }
    if input.asking_price.is_some() {
        parts.push(Part::text("The seller set an asking price above the suggestion; check the claims that justify it."));
    }
    parts.push(Part::Text(format!(
        "Proof round {} of {MAX_PROOF_ROUNDS}. {}",
        input.prior_rounds + 1,
        if input.prior_rounds >= MAX_PROOF_ROUNDS { "Do not request more proof." } else { "Request at most 3 proofs." }
    )));
    parts
}

pub async fn verify(rt: &AgentRuntime, input: &VerifyInput, taxonomy: &Taxonomy) -> AgentResult<VerifyOutput> {
    let out = rt
        .run_json(JsonTask {
            agent: AGENT,
            prompt_version: PROMPT_V,
            system: system_prompt(taxonomy),
            parts: build_parts(input),
            schema: VerifyAi::gemini_schema(),
            tier: ModelTier::Pro,
            thinking: true,
            validate: &validate,
            cacheable: true,
        })
        .await?;
    Ok(out.map(|ai| postprocess(ai, input)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kabados_domain::ids::ImageId;

    fn claim(key: &str, verdict: VerdictAi, level: LadderAi, ev: Vec<String>) -> ClaimAi {
        ClaimAi {
            claim_key: key.into(),
            item_id: None,
            claim_text: "pure copper".into(),
            text_ref: None,
            verdict,
            ladder_level: level,
            rationale: "The coil is visible.".into(),
            evidence_image_ids: ev,
            confidence: 0.8,
        }
    }

    fn input(images: Vec<ImageInput>, prior_rounds: u8) -> VerifyInput {
        VerifyInput {
            items: vec![],
            images,
            description: None,
            asking_price: None,
            claim_reason: None,
            prior_rounds,
            label_text: BTreeMap::new(),
        }
    }

    fn proof(key: &str) -> ProofAi {
        ProofAi {
            claim_key: key.into(),
            shot_type: ShotTypeAi::CLOSE_UP,
            instruction: "Close-up of the cut end.".into(),
            benefit: "Verified bonus.".into(),
        }
    }

    #[test]
    fn supported_needs_evidence_and_ladder_moves_only_from_images() {
        let out = postprocess(
            VerifyAi { claims: vec![claim("c1", VerdictAi::SUPPORTED, LadderAi::L2_CLOSE_UP, vec![])], proof_requests: vec![] },
            &input(vec![], 0),
        );
        assert_eq!(out.claims[0].verdict, Verdict::InsufficientEvidence);
        assert_eq!(out.claims[0].ladder_level, LadderLevel::L0Claimed);
    }

    #[test]
    fn l3_requires_label_text_on_an_evidence_image() {
        let img = ImageInput::new(ImageId::new(), "image/png", vec![1]);
        let id = img.image_id.to_string();
        let mut inp = input(vec![img], 0);
        let ai = || VerifyAi {
            claims: vec![claim("c1", VerdictAi::SUPPORTED, LadderAi::L3_IDENTIFIER, vec![id.clone()])],
            proof_requests: vec![],
        };
        assert_eq!(postprocess(ai(), &inp).claims[0].ladder_level, LadderLevel::L2CloseUp);
        inp.label_text.insert(id.clone(), "LG 1.5 TON".into());
        assert_eq!(postprocess(ai(), &inp).claims[0].ladder_level, LadderLevel::L3Identifier);
    }

    #[test]
    fn proof_request_limits() {
        let img = ImageInput::new(ImageId::new(), "image/png", vec![1]);
        let id = img.image_id.to_string();
        let claims: Vec<ClaimAi> = (1..=5)
            .map(|i| claim(&format!("c{i}"), VerdictAi::INSUFFICIENT_EVIDENCE, LadderAi::L1_PHOTO_CONSISTENT, vec![id.clone()]))
            .collect();
        let proofs: Vec<ProofAi> = (1..=5).map(|i| proof(&format!("c{i}"))).chain([proof("missing")]).collect();
        let out = postprocess(VerifyAi { claims: claims.clone(), proof_requests: proofs.clone() }, &input(vec![img.clone()], 0));
        assert_eq!(out.proof_requests.len(), MAX_PROOFS_PER_ROUND);
        let out = postprocess(VerifyAi { claims, proof_requests: proofs }, &input(vec![img], 2));
        assert!(out.proof_requests.is_empty());
    }

    #[test]
    fn neutral_language_triggers_repair() {
        let mut c = claim("c1", VerdictAi::INCONSISTENT, LadderAi::L0_CLAIMED, vec![]);
        c.rationale = "This is a fake copper claim.".into();
        assert!(validate(&VerifyAi { claims: vec![c], proof_requests: vec![] }).is_err());
    }

    #[test]
    fn inconsistent_claims_stay_at_l0() {
        let img = ImageInput::new(ImageId::new(), "image/png", vec![1]);
        let id = img.image_id.to_string();
        let out = postprocess(
            VerifyAi { claims: vec![claim("c1", VerdictAi::INCONSISTENT, LadderAi::L2_CLOSE_UP, vec![id])], proof_requests: vec![] },
            &input(vec![img], 0),
        );
        assert_eq!(out.claims[0].ladder_level, LadderLevel::L0Claimed);
    }
}
