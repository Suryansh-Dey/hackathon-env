//! Live Gemini API client built on `gemini-client-api` (PLAN.md §11.2).

use crate::model::*;
use async_trait::async_trait;
use base64::Engine;
use gemini_client_api::gemini::ask::Gemini;
use gemini_client_api::gemini::error::GeminiResponseError;
use gemini_client_api::gemini::types::request::{
    BlockThreshold, FunctionCallingConfig, FunctionCallingMode, HarmCategory, InlineData, Part as GPart, PartType, SafetySetting,
    ThinkingConfig, ThinkingLevel, Tool, ToolConfig,
};
use gemini_client_api::gemini::types::response::{FinishReason, GeminiResponse};
use gemini_client_api::gemini::types::sessions::Session;
use serde_json::Value;
use std::str::FromStr;
use std::time::Duration;

/// Per-request HTTP timeout; the runtime enforces the overall run budget.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

pub struct GeminiModel {
    api_key: String,
    http: reqwest::Client,
}

impl std::fmt::Debug for GeminiModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GeminiModel").field("api_key", &"<redacted>").finish()
    }
}

impl GeminiModel {
    pub fn new(api_key: impl Into<String>) -> Result<Self, ModelError> {
        let api_key = api_key.into();
        if api_key.trim().is_empty() {
            return Err(ModelError::NotConfigured("GEMINI_API_KEY is empty".into()));
        }
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|e| ModelError::NotConfigured(format!("http client: {e}")))?;
        Ok(Self { api_key, http })
    }

    /// Reads `GEMINI_API_KEY`.
    pub fn from_env() -> Result<Self, ModelError> {
        let key = std::env::var("GEMINI_API_KEY").map_err(|_| ModelError::NotConfigured("GEMINI_API_KEY is not set".into()))?;
        Self::new(key)
    }

    fn client(&self, model: &str, system: &str, thinking: bool) -> Gemini {
        Gemini::new_with_client(self.api_key.clone(), model, Some(system.into()), self.http.clone())
            .set_safety_settings(Some(safety_settings()))
            .set_thinking_config(thinking_config(model, thinking))
    }
}

/// Blocks harassment, hate, sexual and dangerous content (PLAN.md §11.9). Dangerous
/// content uses the high threshold only, so battery and refrigerant handling
/// instructions are not blocked as false positives.
pub fn safety_settings() -> Vec<SafetySetting> {
    vec![
        SafetySetting::new(HarmCategory::HarmCategoryHarassment, BlockThreshold::BlockMediumAndAbove),
        SafetySetting::new(HarmCategory::HarmCategoryHateSpeech, BlockThreshold::BlockMediumAndAbove),
        SafetySetting::new(HarmCategory::HarmCategorySexuallyExplicit, BlockThreshold::BlockMediumAndAbove),
        SafetySetting::new(HarmCategory::HarmCategoryDangerousContent, BlockThreshold::BlockOnlyHigh),
    ]
}

/// Gemini 3 models take thinking levels; 2.5 models take token budgets.
pub fn thinking_config(model: &str, thinking: bool) -> ThinkingConfig {
    let gen3 = model.contains("gemini-3") || model.ends_with("-latest");
    let pro = model.contains("pro");
    match (gen3, thinking, pro) {
        (true, true, _) => ThinkingConfig::new(false, ThinkingLevel::High),
        // LOW is accepted by every Gemini 3 model; some reject MINIMAL.
        (true, false, _) => ThinkingConfig::new(false, ThinkingLevel::Low),
        (false, true, _) => ThinkingConfig::new_dynamic_thinking(false),
        (false, false, false) => ThinkingConfig::new_disable_thinking(),
        // 2.5 Pro cannot disable thinking; use the smallest budget.
        (false, false, true) => ThinkingConfig::new(false, 128u32),
    }
}

fn to_gemini_parts(parts: &[Part]) -> Result<Vec<GPart>, ModelError> {
    check_parts(parts)?;
    parts
        .iter()
        .map(|p| match p {
            Part::Text(t) => Ok(GPart::from(t.as_str())),
            Part::Image { mime, bytes, .. } => {
                let m = mime::Mime::from_str(mime).map_err(|e| ModelError::InvalidInput(format!("bad mime {mime}: {e}")))?;
                Ok(GPart::from(InlineData::new(m, base64::engine::general_purpose::STANDARD.encode(bytes))))
            }
        })
        .collect()
}

fn usage_of(resp: &GeminiResponse) -> Option<Usage> {
    let u = &resp.usage_metadata;
    let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
    if u.is_null() {
        return None;
    }
    Some(Usage {
        prompt_tokens: n("promptTokenCount"),
        output_tokens: n("candidatesTokenCount"),
        thought_tokens: n("thoughtsTokenCount"),
        total_tokens: n("totalTokenCount"),
    })
}

fn map_error(e: GeminiResponseError) -> ModelError {
    match e {
        GeminiResponseError::StatusNotOk(err) => {
            let code = err.error.code.as_u16();
            let msg = format!("{code} {:?}: {}", err.error.status, err.error.message);
            match code {
                429 => ModelError::RateLimited(msg),
                500..=599 => ModelError::Server(msg),
                _ => ModelError::Client(msg),
            }
        }
        GeminiResponseError::ReqwestError(e) => {
            if let Some(status) = e.status() {
                let msg = format!("{status}: {e}");
                return match status.as_u16() {
                    429 => ModelError::RateLimited(msg),
                    500..=599 => ModelError::Server(msg),
                    _ => ModelError::Client(msg),
                };
            }
            ModelError::Transport(e.to_string())
        }
        GeminiResponseError::NothingToRespond => ModelError::InvalidInput("nothing to respond to".into()),
    }
}

fn check_finish(resp: &GeminiResponse) -> Result<(), ModelError> {
    if resp.candidates.is_empty() {
        let feedback = resp.prompt_feedback.as_ref().map(|v| v.to_string()).unwrap_or_default();
        return if feedback.contains("block") || feedback.contains("BLOCK") {
            Err(ModelError::Blocked(feedback))
        } else {
            Err(ModelError::Empty)
        };
    }
    match resp.get_finish_reason() {
        Some(
            r @ (FinishReason::Safety
            | FinishReason::Blocklist
            | FinishReason::ProhibitedContent
            | FinishReason::Spii
            | FinishReason::ImageSafety
            | FinishReason::Recitation),
        ) => Err(ModelError::Blocked(format!("{r:?}"))),
        Some(FinishReason::MalformedFunctionCall) => Err(ModelError::Server("malformed function call".into())),
        _ => Ok(()),
    }
}

#[async_trait]
impl ModelClient for GeminiModel {
    fn provider(&self) -> &'static str {
        "gemini"
    }

    async fn generate_json(&self, req: &JsonRequest) -> Result<ModelReply, ModelError> {
        let parts = to_gemini_parts(&req.parts)?;
        let ai = self.client(&req.model, &req.system, req.thinking).set_json_mode(req.schema.clone());
        let mut session = Session::new(2).set_remember_reply(false);
        session.ask_parts(parts);
        let resp = ai.ask(&mut session).await.map_err(map_error)?;
        check_finish(&resp)?;
        let text = resp.get_chat().get_text_no_think("");
        if text.trim().is_empty() {
            return Err(ModelError::Empty);
        }
        Ok(ModelReply { text, model: resp.model_version.clone(), usage: usage_of(&resp) })
    }

    async fn tool_step(&self, req: &ToolRequest, transcript: &[Turn]) -> Result<ToolStepReply, ModelError> {
        let mut session = Session::new(transcript.len() * 2 + 4);
        for turn in transcript {
            match turn {
                Turn::User(parts) => {
                    session.ask_parts(to_gemini_parts(parts)?);
                }
                // Replay the provider's own parts so thought signatures survive.
                Turn::ModelCalls { raw: Some(raw), .. } | Turn::ModelText { raw: Some(raw), .. } => {
                    let parts: Vec<GPart> =
                        serde_json::from_value(raw.clone()).map_err(|e| ModelError::InvalidInput(format!("bad raw turn: {e}")))?;
                    session.reply_parts(parts);
                }
                Turn::ModelCalls { calls, raw: None } => {
                    let parts = calls
                        .iter()
                        .map(|c| {
                            GPart::from(gemini_client_api::gemini::types::request::FunctionCall::new(c.name.clone(), Some(c.args.clone())))
                        })
                        .collect();
                    session.reply_parts(parts);
                }
                Turn::ModelText { text, raw: None } => {
                    session.reply(text.as_str());
                }
                Turn::ToolResults(results) => {
                    for r in results {
                        session
                            .add_function_response(r.name.clone(), r.response.clone())
                            .map_err(|e| ModelError::InvalidInput(format!("function response: {e}")))?;
                    }
                }
            }
        }
        let decls = req.tools.iter().map(ToolDecl::to_declaration).collect();
        let mut ai = self.client(&req.model, &req.system, req.thinking).set_tools(vec![Tool::FunctionDeclarations(decls)]);
        if req.require_call {
            ai = ai.set_tool_config(ToolConfig {
                function_calling_config: Some(FunctionCallingConfig { mode: Some(FunctionCallingMode::Any), allowed_function_names: None }),
            });
        }
        let resp = ai.ask(&mut session).await.map_err(map_error)?;
        check_finish(&resp)?;
        let chat = resp.get_chat();
        let raw = serde_json::to_value(chat.parts()).ok();
        let calls: Vec<ToolCall> =
            chat.get_function_calls().map(|c| ToolCall { name: c.name().clone(), args: c.args().clone().unwrap_or(Value::Null) }).collect();
        let step = if calls.is_empty() {
            let text = chat.parts().iter().filter_map(|p| if let PartType::Text(t) = p.data() { Some(t.as_str()) } else { None }).collect();
            ToolStep::Text { text, raw }
        } else {
            ToolStep::Calls { calls, raw }
        };
        Ok(ToolStepReply { step, model: resp.model_version.clone(), usage: usage_of(&resp) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thinking_config_per_generation() {
        let v = serde_json::to_value(thinking_config("gemini-3.5-flash", false)).unwrap();
        assert_eq!(v["thinkingLevel"], "LOW");
        let v = serde_json::to_value(thinking_config("gemini-2.5-flash", false)).unwrap();
        assert_eq!(v["thinkingBudget"], 0);
        let v = serde_json::to_value(thinking_config("gemini-2.5-pro", true)).unwrap();
        assert_eq!(v["thinkingBudget"], -1);
    }

    #[test]
    fn key_is_redacted_in_debug() {
        let m = GeminiModel::new("secret-key").unwrap();
        assert!(!format!("{m:?}").contains("secret"));
        assert!(GeminiModel::new(" ").is_err());
    }
}
