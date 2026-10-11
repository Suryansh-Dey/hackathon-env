//! AI services used synchronously by the API (rules authoring, chat
//! screening, scale reading). Implemented over `kabados-agents`; the port
//! keeps handlers testable and lets the API degrade gracefully (PLAN.md §21.4).

use async_trait::async_trait;
use kabados_agent::AgentRuntime;
use kabados_domain::geo::LatLng;
use kabados_domain::ids::{ImageId, UserId};
use kabados_domain::listing::ImageKind;
use kabados_domain::ruleset::{RuleSet, ValidationIssue};
use kabados_domain::taxonomy::Taxonomy;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RulesDraft {
    pub draft: RuleSet,
    /// Questions to confirm ("Does 'paper' mean newspaper only?").
    pub ambiguities: Vec<String>,
    /// Lines priced far from the Rate Index (±40%), needing confirmation.
    pub far_from_index: Vec<String>,
    pub issues: Vec<ValidationIssue>,
}

#[derive(Clone, Debug)]
pub struct RulesRequest {
    pub instruction: String,
    pub current: Option<RuleSet>,
    pub vendor_id: UserId,
    pub hazardous_authorized: bool,
    pub rate_index_paise_per_kg: BTreeMap<String, i64>,
    pub taxonomy: Arc<Taxonomy>,
    pub yard: Option<LatLng>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub struct TextCheck {
    pub prompt_attack: bool,
    pub abusive: bool,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ScaleRead {
    pub reading_g: Option<i64>,
    pub unit_detected: Option<String>,
    pub confidence: f32,
}

#[async_trait]
pub trait AiServices: Send + Sync {
    async fn draft_rules(&self, req: RulesRequest) -> anyhow::Result<RulesDraft>;
    async fn classify_text(&self, text: &str) -> anyhow::Result<TextCheck>;
    async fn read_scale(&self, image: Vec<u8>, mime: &str) -> anyhow::Result<ScaleRead>;
}

/// Builds the agent runtime: live Gemini (key from Secrets Manager) unless
/// `AI_MODE=mock`, in which case the deterministic offline model is used.
pub async fn runtime(cfg: &kabados_runtime::Config, secrets: &kabados_runtime::secrets::Secrets) -> anyhow::Result<AgentRuntime> {
    if cfg.modes.ai == kabados_runtime::Mode::Mock {
        return Ok(kabados_agents::mock_runtime());
    }
    let key = secrets.get("gemini/api_key", "GEMINI_API_KEY").await?;
    let client = kabados_agent::GeminiModel::new(key).map_err(|e| anyhow::anyhow!("gemini client: {e}"))?;
    Ok(AgentRuntime::new(Arc::new(client), kabados_agent::ModelConfig::from_env()))
}

/// [`AiServices`] over `kabados-agents`.
pub struct AgentsAi {
    rt: AgentRuntime,
}

impl AgentsAi {
    pub fn new(rt: AgentRuntime) -> Self {
        Self { rt }
    }
}

#[async_trait]
impl AiServices for AgentsAi {
    async fn draft_rules(&self, req: RulesRequest) -> anyhow::Result<RulesDraft> {
        let out = kabados_agents::author_rules(
            &self.rt,
            &kabados_agents::RulesAuthorInput {
                instruction: req.instruction,
                current: req.current,
                taxonomy: &req.taxonomy,
                rate_index_paise_per_kg: req.rate_index_paise_per_kg,
                hazardous_authorized: req.hazardous_authorized,
                vendor_id: req.vendor_id,
                now: chrono::Utc::now(),
                yard: req.yard,
            },
        )
        .await
        .map_err(|e| anyhow::anyhow!("rules agent: {e}"))?;
        let v = out.value;
        Ok(RulesDraft { draft: v.draft, ambiguities: v.ambiguities, far_from_index: v.far_from_index, issues: v.issues })
    }

    async fn classify_text(&self, text: &str) -> anyhow::Result<TextCheck> {
        let v = kabados_agents::classify_text(&self.rt, text).await.map_err(|e| anyhow::anyhow!("classifier: {e}"))?.value;
        Ok(TextCheck { prompt_attack: v.prompt_attack, abusive: v.abusive, reasons: v.reasons })
    }

    async fn read_scale(&self, image: Vec<u8>, mime: &str) -> anyhow::Result<ScaleRead> {
        let input =
            kabados_agents::ImageInput { image_id: ImageId::new(), mime: mime.to_owned(), bytes: image, kind: Some(ImageKind::Scale) };
        let v = kabados_agents::read_scale(&self.rt, &input).await.map_err(|e| anyhow::anyhow!("scale agent: {e}"))?.value;
        Ok(ScaleRead { reading_g: v.reading_g, unit_detected: v.unit_detected, confidence: v.confidence })
    }
}

/// Used when no model is configured: callers fall back to manual input.
pub struct NoAi;

#[async_trait]
impl AiServices for NoAi {
    async fn draft_rules(&self, _req: RulesRequest) -> anyhow::Result<RulesDraft> {
        anyhow::bail!("AI is unavailable; use the form editor")
    }
    async fn classify_text(&self, _text: &str) -> anyhow::Result<TextCheck> {
        Ok(TextCheck::default())
    }
    async fn read_scale(&self, _image: Vec<u8>, _mime: &str) -> anyhow::Result<ScaleRead> {
        Ok(ScaleRead { reading_g: None, unit_detected: None, confidence: 0.0 })
    }
}
