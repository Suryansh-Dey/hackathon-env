//! Agent 6: scale reading (PLAN.md §11.8). On low confidence the driver types
//! the value and the photo is kept as evidence.

use crate::common::{COMMON_RULES, ImageInput, bbox_permille, pct_to_unit};
use gemini_client_api::gemini::utils::{GeminiSchema, gemini_schema};
use kabados_agent::{AgentResult, AgentRuntime, JsonTask, ModelTier, Part};
use serde::{Deserialize, Serialize};

pub const AGENT: &str = "scale_read";
pub const PROMPT_V: &str = "scale-read-2026-10-11";
/// Below this the app asks the driver to type the value.
pub const MIN_CONFIDENCE: f32 = 0.7;
/// Largest plausible single reading (5 tonnes).
pub const MAX_READING_G: i64 = 5_000_000;

#[gemini_schema]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScaleAi {
    /// The number shown on the display exactly as read (e.g. "42.60"), or null if unreadable.
    pub reading_text: Option<String>,
    /// Unit shown or implied: "kg" or "g"; null if unknown.
    pub unit: Option<String>,
    /// Display type: "7-segment", "lcd", "analogue" or "unknown".
    pub display_type: Option<String>,
    /// Confidence as an integer percentage, 0-100.
    pub confidence_pct: u32,
    /// Box [x0, y0, x1, y1] around the display in thousandths of the image size (integers 0-1000), or empty.
    pub display_bbox: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScaleReading {
    pub reading_g: Option<i64>,
    pub unit_detected: Option<String>,
    pub confidence: f32,
    pub display_bbox: Option<[f32; 4]>,
}

impl ScaleReading {
    /// The driver must confirm or type the value.
    pub fn needs_manual_entry(&self) -> bool {
        self.reading_g.is_none() || self.confidence < MIN_CONFIDENCE
    }
}

const SYSTEM: &str = "You read weighing-scale displays in photos taken by transporters at scrap pickups (7-segment, LCD or analogue dials).
Report the number exactly as displayed, including the decimal point, and the unit (kg or g). Platform scales in India almost always show kg.
If digits are partly hidden, glare covers them or the display is not visible, return reading_text null and a low confidence. Never guess digits.";

/// Parses `"42.60"` + unit into grams.
pub fn to_grams(text: &str, unit: Option<&str>) -> Option<i64> {
    let cleaned: String = text.trim().chars().filter(|c| c.is_ascii_digit() || *c == '.').collect();
    if cleaned.is_empty() || cleaned.matches('.').count() > 1 {
        return None;
    }
    let v: f64 = cleaned.parse().ok()?;
    let g = match unit.map(|u| u.trim().to_ascii_lowercase()) {
        Some(u) if u == "g" || u == "gm" || u == "grams" => v.round() as i64,
        _ => (v * 1000.0).round() as i64,
    };
    (0..=MAX_READING_G).contains(&g).then_some(g)
}

fn validate(s: &ScaleAi) -> Result<(), String> {
    if let Some(u) = &s.unit
        && !matches!(u.trim().to_ascii_lowercase().as_str(), "kg" | "g" | "gm" | "grams" | "kgs")
    {
        return Err(format!("unit must be \"kg\" or \"g\", got {u:?}"));
    }
    if let Some(t) = &s.reading_text
        && to_grams(t, s.unit.as_deref()).is_none()
    {
        return Err(format!("reading_text {t:?} is not a plausible number; return null if unreadable"));
    }
    Ok(())
}

pub fn to_domain(s: ScaleAi) -> ScaleReading {
    let unit = s.unit.map(|u| {
        let u = u.trim().to_ascii_lowercase();
        if u == "g" || u == "gm" || u == "grams" { "g".to_string() } else { "kg".to_string() }
    });
    let reading_g = s.reading_text.as_deref().and_then(|t| to_grams(t, unit.as_deref()));
    let confidence = if reading_g.is_some() { pct_to_unit(s.confidence_pct) } else { 0.0 };
    ScaleReading { reading_g, unit_detected: unit, confidence, display_bbox: bbox_permille(&s.display_bbox) }
}

pub async fn read_scale(rt: &AgentRuntime, image: &ImageInput) -> AgentResult<ScaleReading> {
    let mut parts = vec![Part::text("Read the scale display.")];
    parts.extend(image.to_parts());
    let out = rt
        .run_json(JsonTask {
            agent: AGENT,
            prompt_version: PROMPT_V,
            system: format!("{SYSTEM}\n\n{COMMON_RULES}"),
            parts,
            schema: ScaleAi::gemini_schema(),
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
    fn grams_conversion() {
        assert_eq!(to_grams("42.60", Some("kg")), Some(42_600));
        assert_eq!(to_grams("350", Some("g")), Some(350));
        assert_eq!(to_grams("7.5", None), Some(7_500));
        assert_eq!(to_grams("1.2.3", Some("kg")), None);
        assert_eq!(to_grams("99999", Some("kg")), None);
    }

    #[test]
    fn low_confidence_needs_manual_entry() {
        let r = to_domain(ScaleAi {
            reading_text: Some("12.3".into()),
            unit: Some("kg".into()),
            display_type: None,
            confidence_pct: 40,
            display_bbox: vec![],
        });
        assert_eq!(r.reading_g, Some(12_300));
        assert!(r.needs_manual_entry());
        let none = to_domain(ScaleAi { reading_text: None, unit: None, display_type: None, confidence_pct: 90, display_bbox: vec![] });
        assert!(none.needs_manual_entry() && none.confidence == 0.0);
    }
}
