//! Provider-agnostic model interface (PLAN.md §11.2).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// Largest inline image accepted. Callers send downscaled copies (PLAN.md §11.12);
/// anything larger is a bug upstream, not something to forward to the model.
pub const MAX_IMAGE_BYTES: usize = 4 * 1024 * 1024;

/// Model tiering: cost and latency proportional to task difficulty.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ModelTier {
    /// Photo triage, scale reading, simple classification.
    Flash,
    /// Itemization, claim verification, matching, rule authoring (thinking on).
    Pro,
    /// Offline evaluation and hard review cases.
    Strongest,
}

/// Model IDs per tier. Lives in AppConfig in deployed environments; env vars here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelConfig {
    pub flash: String,
    pub pro: String,
    pub strongest: String,
}

pub const DEFAULT_FLASH_MODEL: &str = "gemini-3.5-flash";
pub const DEFAULT_PRO_MODEL: &str = "gemini-3.1-pro-preview";
pub const DEFAULT_STRONGEST_MODEL: &str = "gemini-3.1-pro-preview";

impl Default for ModelConfig {
    fn default() -> Self {
        Self { flash: DEFAULT_FLASH_MODEL.into(), pro: DEFAULT_PRO_MODEL.into(), strongest: DEFAULT_STRONGEST_MODEL.into() }
    }
}

impl ModelConfig {
    /// Reads `AI_MODEL_FLASH`, `AI_MODEL_PRO`, `AI_MODEL_STRONGEST`, falling back to defaults.
    pub fn from_env() -> Self {
        let d = Self::default();
        let get = |k: &str, dflt: String| std::env::var(k).ok().filter(|v| !v.trim().is_empty()).unwrap_or(dflt);
        Self { flash: get("AI_MODEL_FLASH", d.flash), pro: get("AI_MODEL_PRO", d.pro), strongest: get("AI_MODEL_STRONGEST", d.strongest) }
    }

    pub fn model_for(&self, tier: ModelTier) -> &str {
        match tier {
            ModelTier::Flash => &self.flash,
            ModelTier::Pro => &self.pro,
            ModelTier::Strongest => &self.strongest,
        }
    }
}

/// One input part. Images must already be downscaled, EXIF-stripped and face-blurred.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Part {
    Text(String),
    Image { image_id: String, mime: String, bytes: Vec<u8> },
}

impl std::fmt::Debug for Part {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Part::Text(t) => f.debug_tuple("Text").field(t).finish(),
            Part::Image { image_id, mime, bytes } => {
                f.debug_struct("Image").field("image_id", image_id).field("mime", mime).field("bytes", &bytes.len()).finish()
            }
        }
    }
}

impl Part {
    pub fn text(s: impl Into<String>) -> Self {
        Part::Text(s.into())
    }
}

/// Token usage reported by the provider.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub output_tokens: u64,
    pub thought_tokens: u64,
    pub total_tokens: u64,
}

impl Usage {
    pub fn add(&mut self, other: &Usage) {
        self.prompt_tokens += other.prompt_tokens;
        self.output_tokens += other.output_tokens;
        self.thought_tokens += other.thought_tokens;
        self.total_tokens += other.total_tokens;
    }
}

/// A single structured-JSON call.
#[derive(Clone, Debug)]
pub struct JsonRequest {
    /// Agent name; used for tracing and mock routing.
    pub agent: String,
    pub system: String,
    pub parts: Vec<Part>,
    /// Response schema in Gemini's OpenAPI subset (`#[gemini_schema]` output).
    pub schema: Value,
    pub tier: ModelTier,
    pub model: String,
    pub thinking: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModelReply {
    pub text: String,
    pub model: String,
    pub usage: Option<Usage>,
}

/// A function the model may call during a tool loop.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolDecl {
    pub name: String,
    pub description: String,
    /// Parameters schema (Gemini OpenAPI subset, `type: OBJECT`).
    pub parameters: Value,
}

impl ToolDecl {
    /// Gemini `functionDeclaration` JSON.
    pub fn to_declaration(&self) -> Value {
        serde_json::json!({ "name": self.name, "description": self.description, "parameters": self.parameters })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub name: String,
    pub args: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    pub name: String,
    pub response: Value,
}

/// Provider-agnostic transcript of a tool loop.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Turn {
    User(Vec<Part>),
    /// Model asked for calls. `raw` holds the provider's own parts (e.g. Gemini
    /// thought signatures) so the exact turn can be replayed.
    ModelCalls {
        calls: Vec<ToolCall>,
        raw: Option<Value>,
    },
    /// Model replied with text instead of a call.
    ModelText {
        text: String,
        raw: Option<Value>,
    },
    ToolResults(Vec<ToolResult>),
}

#[derive(Clone, Debug)]
pub struct ToolRequest {
    pub agent: String,
    pub system: String,
    pub tools: Vec<ToolDecl>,
    pub tier: ModelTier,
    pub model: String,
    pub thinking: bool,
    /// Force the model to always call a function (the final answer is a function too).
    pub require_call: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ToolStep {
    Calls { calls: Vec<ToolCall>, raw: Option<Value> },
    Text { text: String, raw: Option<Value> },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolStepReply {
    pub step: ToolStep,
    pub model: String,
    pub usage: Option<Usage>,
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum ModelError {
    #[error("rate limited: {0}")]
    RateLimited(String),
    #[error("provider error: {0}")]
    Server(String),
    #[error("request rejected: {0}")]
    Client(String),
    #[error("transport error: {0}")]
    Transport(String),
    #[error("blocked by safety settings: {0}")]
    Blocked(String),
    #[error("empty response")]
    Empty,
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("not configured: {0}")]
    NotConfigured(String),
}

impl ModelError {
    /// Rate limits, 5xx and transport failures are retried with backoff.
    pub fn retryable(&self) -> bool {
        matches!(self, ModelError::RateLimited(_) | ModelError::Server(_) | ModelError::Transport(_) | ModelError::Empty)
    }
}

/// A model provider. Implementations: [`crate::GeminiModel`] (live) and [`crate::MockModel`] (CI/offline).
#[async_trait]
pub trait ModelClient: Send + Sync {
    /// Human-readable provider name (`gemini`, `mock`).
    fn provider(&self) -> &'static str;

    /// One structured-JSON generation.
    async fn generate_json(&self, req: &JsonRequest) -> Result<ModelReply, ModelError>;

    /// One step of a tool-calling loop over `transcript`.
    async fn tool_step(&self, req: &ToolRequest, transcript: &[Turn]) -> Result<ToolStepReply, ModelError>;
}

/// Validates image parts before they are sent anywhere.
pub fn check_parts(parts: &[Part]) -> Result<(), ModelError> {
    for p in parts {
        if let Part::Image { image_id, mime, bytes } = p {
            if bytes.is_empty() {
                return Err(ModelError::InvalidInput(format!("image {image_id} is empty")));
            }
            if bytes.len() > MAX_IMAGE_BYTES {
                return Err(ModelError::InvalidInput(format!(
                    "image {image_id} is {} bytes; the limit is {MAX_IMAGE_BYTES} (send the downscaled copy)",
                    bytes.len()
                )));
            }
            if !matches!(mime.as_str(), "image/jpeg" | "image/png" | "image/webp") {
                return Err(ModelError::InvalidInput(format!("image {image_id} has unsupported type {mime}")));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_oversized_or_wrong_type_images() {
        let ok = Part::Image { image_id: "img_1".into(), mime: "image/jpeg".into(), bytes: vec![1, 2, 3] };
        assert!(check_parts(&[ok]).is_ok());
        let big = Part::Image { image_id: "img_2".into(), mime: "image/jpeg".into(), bytes: vec![0; MAX_IMAGE_BYTES + 1] };
        assert!(matches!(check_parts(&[big]), Err(ModelError::InvalidInput(_))));
        let gif = Part::Image { image_id: "img_3".into(), mime: "image/gif".into(), bytes: vec![1] };
        assert!(check_parts(&[gif]).is_err());
    }

    #[test]
    fn retryable_classification() {
        assert!(ModelError::RateLimited("429".into()).retryable());
        assert!(ModelError::Server("503".into()).retryable());
        assert!(!ModelError::Client("400".into()).retryable());
        assert!(!ModelError::Blocked("SAFETY".into()).retryable());
    }

    #[test]
    fn model_config_env_override() {
        let c = ModelConfig::default();
        assert_eq!(c.model_for(ModelTier::Flash), DEFAULT_FLASH_MODEL);
    }
}
