//! Agent 1: photo triage (PLAN.md §11.3).

use crate::common::{COMMON_RULES, ImageInput};
use gemini_client_api::gemini::utils::{GeminiSchema, gemini_schema};
use kabados_agent::{AgentResult, AgentRuntime, JsonTask, ModelTier};
use kabados_domain::listing::{PhotoIssue, Triage};
use serde::{Deserialize, Serialize};

pub const AGENT: &str = "triage";
pub const PROMPT_V: &str = "triage-2026-10-11";

#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[gemini_schema]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PhotoIssueAi {
    BLUR,
    DARK,
    CUT_OFF,
    NO_ITEMS,
    PEOPLE_VISIBLE,
}

/// Photo quality assessment.
#[gemini_schema]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TriageAi {
    /// False when the photo cannot be used to identify the items.
    pub usable: bool,
    /// Problems found. PEOPLE_VISIBLE alone does not make a photo unusable.
    pub issues: Vec<PhotoIssueAi>,
    /// One short, friendly instruction for retaking the photo, when needed.
    pub retake_hint: Option<String>,
}

const SYSTEM: &str = "You check photos that sellers take of household scrap (paper, plastic, metal, appliances, e-waste, batteries) before a detailed analysis.
Decide if the photo is usable for identifying the items. Report issues:
- BLUR: too blurry to see what the items are.
- DARK: too dark to see the items.
- CUT_OFF: the main items are cut off at the frame edge.
- NO_ITEMS: no scrap items are visible.
- PEOPLE_VISIBLE: a person or face is visible (faces are blurred later; this alone keeps the photo usable).
If the photo is not usable, give a retake hint such as \"Move to daylight and take the photo again.\"";

const BLOCKING: [PhotoIssueAi; 4] = [PhotoIssueAi::BLUR, PhotoIssueAi::DARK, PhotoIssueAi::CUT_OFF, PhotoIssueAi::NO_ITEMS];

fn validate(t: &TriageAi) -> Result<(), String> {
    if let Some(h) = &t.retake_hint
        && let Some(w) = crate::common::neutral_language_violation(h)
    {
        return Err(format!("retake_hint uses non-neutral word {w:?}"));
    }
    Ok(())
}

/// Deterministic post-processing: usability follows the blocking issues.
pub fn to_domain(ai: TriageAi) -> Triage {
    let mut issues: Vec<PhotoIssue> = Vec::new();
    for i in &ai.issues {
        let d = match i {
            PhotoIssueAi::BLUR => PhotoIssue::Blur,
            PhotoIssueAi::DARK => PhotoIssue::Dark,
            PhotoIssueAi::CUT_OFF => PhotoIssue::CutOff,
            PhotoIssueAi::NO_ITEMS => PhotoIssue::NoItems,
            PhotoIssueAi::PEOPLE_VISIBLE => PhotoIssue::PeopleVisible,
        };
        if !issues.contains(&d) {
            issues.push(d);
        }
    }
    let usable = !ai.issues.iter().any(|i| BLOCKING.contains(i));
    let retake_hint =
        if usable { None } else { ai.retake_hint.filter(|h| !h.trim().is_empty()).or_else(|| Some(default_hint(&ai.issues).to_string())) };
    Triage { usable, issues, retake_hint }
}

fn default_hint(issues: &[PhotoIssueAi]) -> &'static str {
    if issues.contains(&PhotoIssueAi::DARK) {
        "Move to a brighter spot or daylight and take the photo again."
    } else if issues.contains(&PhotoIssueAi::BLUR) {
        "Hold the phone steady and tap to focus before taking the photo."
    } else if issues.contains(&PhotoIssueAi::CUT_OFF) {
        "Step back so all the items fit inside the photo."
    } else {
        "Point the camera at the scrap items and take the photo again."
    }
}

pub async fn triage(rt: &AgentRuntime, image: &ImageInput) -> AgentResult<Triage> {
    let mut parts = vec![kabados_agent::Part::text("Check this photo.")];
    parts.extend(image.to_parts());
    let out = rt
        .run_json(JsonTask {
            agent: AGENT,
            prompt_version: PROMPT_V,
            system: format!("{SYSTEM}\n\n{COMMON_RULES}"),
            parts,
            schema: TriageAi::gemini_schema(),
            tier: ModelTier::Flash,
            thinking: false,
            validate: &validate,
            cacheable: true,
        })
        .await?;
    Ok(out.map(to_domain))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usability_follows_blocking_issues() {
        let t = to_domain(TriageAi { usable: true, issues: vec![PhotoIssueAi::DARK], retake_hint: None });
        assert!(!t.usable);
        assert!(t.retake_hint.unwrap().contains("brighter"));
        let t = to_domain(TriageAi { usable: false, issues: vec![PhotoIssueAi::PEOPLE_VISIBLE], retake_hint: Some("x".into()) });
        assert!(t.usable && t.retake_hint.is_none());
        assert_eq!(t.issues, vec![PhotoIssue::PeopleVisible]);
    }

    #[test]
    fn schema_uses_wire_names() {
        let s = TriageAi::gemini_schema().to_string();
        assert!(s.contains("CUT_OFF") && s.contains("PEOPLE_VISIBLE"));
    }
}
