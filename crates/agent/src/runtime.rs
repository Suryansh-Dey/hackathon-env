//! Agent runtime: budgets, retries with backoff, schema validation with one
//! repair retry, output caching and tracing (PLAN.md §11.2, §11.9, §11.12).

use crate::cache::{InMemoryCache, OutputCache, cache_key};
use crate::model::*;
use async_trait::async_trait;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;
use tracing::Instrument;

/// Per-run limits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Budgets {
    /// Maximum model calls in a tool loop.
    pub max_steps: u32,
    /// Maximum total tokens per run (when the provider reports usage).
    pub max_tokens: u64,
    /// Wall-clock limit for the whole run, including retries.
    pub timeout: Duration,
    /// Limit per model call; a hung call is retried instead of eating the run budget.
    pub call_timeout: Duration,
}

impl Default for Budgets {
    fn default() -> Self {
        Self { max_steps: 12, max_tokens: 400_000, timeout: Duration::from_secs(120), call_timeout: Duration::from_secs(60) }
    }
}

impl Budgets {
    /// Defaults overridden by `AI_MAX_STEPS`, `AI_MAX_TOKENS`, `AI_TIMEOUT_SECS`, `AI_CALL_TIMEOUT_SECS`.
    pub fn from_env() -> Self {
        let d = Self::default();
        let num = |k: &str| std::env::var(k).ok().and_then(|v| v.trim().parse::<u64>().ok());
        Self {
            max_steps: num("AI_MAX_STEPS").map(|v| v as u32).unwrap_or(d.max_steps),
            max_tokens: num("AI_MAX_TOKENS").unwrap_or(d.max_tokens),
            timeout: num("AI_TIMEOUT_SECS").map(Duration::from_secs).unwrap_or(d.timeout),
            call_timeout: num("AI_CALL_TIMEOUT_SECS").map(Duration::from_secs).unwrap_or(d.call_timeout),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetryPolicy {
    pub max_retries: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self { max_retries: 3, base_delay: Duration::from_millis(500), max_delay: Duration::from_secs(8) }
    }
}

impl RetryPolicy {
    /// Exponential backoff with full jitter.
    pub fn delay(&self, attempt: u32) -> Duration {
        let exp = self.base_delay.saturating_mul(1u32 << attempt.min(16));
        let cap = exp.min(self.max_delay);
        let ms = cap.as_millis() as u64;
        Duration::from_millis(if ms == 0 { 0 } else { rand::random_range(ms / 2..=ms) })
    }
}

/// A validated agent result plus provenance (stored with every AI output, PLAN.md §11.1).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AgentOutput<T> {
    pub value: T,
    pub model: String,
    pub prompt_version: String,
    pub latency_ms: u64,
    /// Model calls made (including retries and the repair attempt).
    pub attempts: u32,
    pub usage: Option<Usage>,
    pub cached: bool,
}

impl<T> AgentOutput<T> {
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> AgentOutput<U> {
        AgentOutput {
            value: f(self.value),
            model: self.model,
            prompt_version: self.prompt_version,
            latency_ms: self.latency_ms,
            attempts: self.attempts,
            usage: self.usage,
            cached: self.cached,
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum AgentError {
    #[error("model call failed: {0}")]
    Model(#[from] ModelError),
    /// Output failed validation twice; callers fail closed (PLAN.md §11.9).
    #[error("{agent} returned invalid output after repair: {detail}")]
    InvalidOutput { agent: String, detail: String },
    #[error("budget exceeded: {0}")]
    Budget(String),
    #[error("timed out after {0:?}")]
    Timeout(Duration),
    #[error("invalid input: {0}")]
    InvalidInput(String),
}

pub type AgentResult<T> = Result<AgentOutput<T>, AgentError>;

/// Agent-specific semantic validation; the error text is sent back for the repair retry.
pub type Validator<'a, T> = &'a (dyn Fn(&T) -> Result<(), String> + Send + Sync);

/// A single-shot structured call.
pub struct JsonTask<'a, T> {
    pub agent: &'a str,
    pub prompt_version: &'a str,
    pub system: String,
    pub parts: Vec<Part>,
    pub schema: Value,
    pub tier: ModelTier,
    pub thinking: bool,
    pub validate: Validator<'a, T>,
    pub cacheable: bool,
}

/// Dispatches tool calls to in-process Rust functions (no network hop, PLAN.md §11.2).
#[async_trait]
pub trait ToolDispatcher: Send + Sync {
    async fn dispatch(&self, call: &ToolCall) -> Result<Value, String>;
}

/// A bounded tool-calling loop whose answer is delivered by calling `final_tool`.
pub struct ToolTask<'a, T> {
    pub agent: &'a str,
    pub prompt_version: &'a str,
    pub system: String,
    pub initial: Vec<Part>,
    pub tools: Vec<ToolDecl>,
    /// Its parameters schema is the output type `T`.
    pub final_tool: ToolDecl,
    pub tier: ModelTier,
    pub thinking: bool,
    pub validate: Validator<'a, T>,
}

#[derive(Clone)]
pub struct AgentRuntime {
    client: Arc<dyn ModelClient>,
    models: ModelConfig,
    cache: Arc<dyn OutputCache>,
    budgets: Budgets,
    retry: RetryPolicy,
}

impl std::fmt::Debug for AgentRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentRuntime")
            .field("provider", &self.client.provider())
            .field("models", &self.models)
            .field("budgets", &self.budgets)
            .finish()
    }
}

const REPAIR_PROMPT: &str = "Your previous response did not pass validation. Return a corrected response that follows the schema and every rule above. Respond with JSON only.";

/// Extracts a JSON document from model text (tolerates code fences and prose).
pub fn extract_json(text: &str) -> &str {
    let t = text.trim();
    let t = t.strip_prefix("```json").or_else(|| t.strip_prefix("```")).unwrap_or(t);
    let t = t.strip_suffix("```").unwrap_or(t).trim();
    let start = t.find(['{', '[']);
    let end = t.rfind(['}', ']']);
    match (start, end) {
        (Some(s), Some(e)) if e >= s => &t[s..=e],
        _ => t,
    }
}

fn parse_and_validate<T: DeserializeOwned>(text: &str, validate: Validator<'_, T>) -> Result<T, String> {
    let v: T = serde_json::from_str(extract_json(text)).map_err(|e| format!("response is not valid JSON for the schema: {e}"))?;
    validate(&v)?;
    Ok(v)
}

fn truncate(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

impl AgentRuntime {
    pub fn new(client: Arc<dyn ModelClient>, models: ModelConfig) -> Self {
        Self { client, models, cache: Arc::new(InMemoryCache::default()), budgets: Budgets::from_env(), retry: RetryPolicy::default() }
    }

    /// `client_from_env()` + `ModelConfig::from_env()` + in-memory cache.
    pub fn from_env() -> Self {
        Self::new(crate::client_from_env(), ModelConfig::from_env())
    }

    pub fn with_cache(mut self, cache: Arc<dyn OutputCache>) -> Self {
        self.cache = cache;
        self
    }

    pub fn with_budgets(mut self, budgets: Budgets) -> Self {
        self.budgets = budgets;
        self
    }

    pub fn with_retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    pub fn models(&self) -> &ModelConfig {
        &self.models
    }

    pub fn budgets(&self) -> &Budgets {
        &self.budgets
    }

    pub fn provider(&self) -> &'static str {
        self.client.provider()
    }

    pub fn is_mock(&self) -> bool {
        self.client.provider() == "mock"
    }

    async fn with_retries<R, F, Fut>(&self, agent: &str, attempts: &mut u32, mut call: F) -> Result<R, ModelError>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<R, ModelError>>,
    {
        let mut retry = 0u32;
        loop {
            *attempts += 1;
            let result = match tokio::time::timeout(self.budgets.call_timeout, call()).await {
                Ok(r) => r,
                Err(_) => Err(ModelError::Transport(format!("model call exceeded {:?}", self.budgets.call_timeout))),
            };
            match result {
                Ok(r) => return Ok(r),
                Err(e) if e.retryable() && retry < self.retry.max_retries => {
                    let d = self.retry.delay(retry);
                    tracing::warn!(agent, error = %e, retry, delay_ms = d.as_millis() as u64, "retrying model call");
                    tokio::time::sleep(d).await;
                    retry += 1;
                }
                Err(e) => return Err(e),
            }
        }
    }

    fn charge(&self, total: &mut Usage, u: &Option<Usage>) -> Result<(), AgentError> {
        if let Some(u) = u {
            total.add(u);
            if total.total_tokens > self.budgets.max_tokens {
                return Err(AgentError::Budget(format!("{} tokens > {}", total.total_tokens, self.budgets.max_tokens)));
            }
        }
        Ok(())
    }

    /// Runs a structured-JSON task: cache → call (retries) → validate → one repair → fail closed.
    pub async fn run_json<T>(&self, task: JsonTask<'_, T>) -> AgentResult<T>
    where
        T: DeserializeOwned + Serialize + Send,
    {
        check_parts(&task.parts).map_err(|e| AgentError::InvalidInput(e.to_string()))?;
        let started = Instant::now();
        let model = self.models.model_for(task.tier).to_string();
        let key = cache_key(task.agent, task.prompt_version, &model, &task.system, &task.schema, &task.parts);
        if task.cacheable
            && let Some(hit) = self.cache.get(&key).await
            && let Ok(value) = serde_json::from_str::<T>(&hit)
        {
            tracing::debug!(agent = task.agent, "agent cache hit");
            return Ok(AgentOutput {
                value,
                model,
                prompt_version: task.prompt_version.into(),
                latency_ms: started.elapsed().as_millis() as u64,
                attempts: 0,
                usage: None,
                cached: true,
            });
        }

        let span = tracing::info_span!("agent_run", agent = task.agent, prompt_version = task.prompt_version, model = %model);
        let fut = async {
            let mut req = JsonRequest {
                agent: task.agent.to_string(),
                system: task.system.clone(),
                parts: task.parts.clone(),
                schema: task.schema.clone(),
                tier: task.tier,
                model: model.clone(),
                thinking: task.thinking,
            };
            let mut attempts = 0u32;
            let mut usage = Usage::default();
            let mut repaired = false;
            loop {
                let call_started = Instant::now();
                let reply = self
                    .with_retries(task.agent, &mut attempts, || self.client.generate_json(&req))
                    .instrument(tracing::info_span!("model_call", agent = task.agent, model = %req.model))
                    .await?;
                let u = reply.usage.unwrap_or_default();
                tracing::info!(
                    agent = task.agent,
                    model = %reply.model,
                    prompt_tokens = u.prompt_tokens,
                    output_tokens = u.output_tokens,
                    thought_tokens = u.thought_tokens,
                    latency_ms = call_started.elapsed().as_millis() as u64,
                    "model call"
                );
                self.charge(&mut usage, &reply.usage)?;
                match parse_and_validate::<T>(&reply.text, task.validate) {
                    Ok(value) => {
                        if task.cacheable
                            && let Ok(s) = serde_json::to_string(&value)
                        {
                            self.cache.put(&key, s).await;
                        }
                        return Ok(AgentOutput {
                            value,
                            model: reply.model,
                            prompt_version: task.prompt_version.into(),
                            latency_ms: started.elapsed().as_millis() as u64,
                            attempts,
                            usage: Some(usage),
                            cached: false,
                        });
                    }
                    Err(detail) if !repaired => {
                        tracing::warn!(agent = task.agent, %detail, "invalid output; repairing once");
                        repaired = true;
                        req.parts.push(Part::Text(format!(
                            "{REPAIR_PROMPT}\nProblem: {detail}\nPrevious response:\n{}",
                            truncate(&reply.text, 4000)
                        )));
                    }
                    Err(detail) => {
                        tracing::error!(agent = task.agent, %detail, "invalid output after repair; failing closed");
                        return Err(AgentError::InvalidOutput { agent: task.agent.to_string(), detail });
                    }
                }
            }
        }
        .instrument(span);
        tokio::time::timeout(self.budgets.timeout, fut).await.map_err(|_| AgentError::Timeout(self.budgets.timeout))?
    }

    /// Runs a bounded tool-calling loop; the model answers by calling `final_tool`.
    pub async fn run_tools<T>(&self, task: ToolTask<'_, T>, tools: &dyn ToolDispatcher) -> AgentResult<T>
    where
        T: DeserializeOwned + Send,
    {
        check_parts(&task.initial).map_err(|e| AgentError::InvalidInput(e.to_string()))?;
        let started = Instant::now();
        let model = self.models.model_for(task.tier).to_string();
        let mut decls = task.tools.clone();
        decls.push(task.final_tool.clone());
        let req = ToolRequest {
            agent: task.agent.to_string(),
            system: task.system.clone(),
            tools: decls,
            tier: task.tier,
            model: model.clone(),
            thinking: task.thinking,
            require_call: true,
        };
        let final_name = task.final_tool.name.clone();
        let span = tracing::info_span!("agent_tool_run", agent = task.agent, prompt_version = task.prompt_version, model = %model);
        let fut = async {
            let mut transcript = vec![Turn::User(task.initial.clone())];
            let mut attempts = 0u32;
            let mut usage = Usage::default();
            let mut repaired = false;
            for step in 0..self.budgets.max_steps {
                let reply = self
                    .with_retries(task.agent, &mut attempts, || self.client.tool_step(&req, &transcript))
                    .instrument(tracing::info_span!("model_call", agent = task.agent, step))
                    .await?;
                self.charge(&mut usage, &reply.usage)?;
                let last_model = reply.model.clone();
                match reply.step {
                    ToolStep::Calls { calls, raw } => {
                        transcript.push(Turn::ModelCalls { calls: calls.clone(), raw });
                        let mut results = Vec::with_capacity(calls.len());
                        for call in &calls {
                            if call.name == final_name {
                                let parsed = serde_json::from_value::<T>(call.args.clone())
                                    .map_err(|e| format!("arguments do not match the schema: {e}"))
                                    .and_then(|v| (task.validate)(&v).map(|_| v));
                                match parsed {
                                    Ok(value) => {
                                        return Ok(AgentOutput {
                                            value,
                                            model: last_model,
                                            prompt_version: task.prompt_version.into(),
                                            latency_ms: started.elapsed().as_millis() as u64,
                                            attempts,
                                            usage: Some(usage),
                                            cached: false,
                                        });
                                    }
                                    Err(detail) if !repaired => {
                                        tracing::warn!(agent = task.agent, %detail, "invalid final answer; repairing once");
                                        repaired = true;
                                        results.push(ToolResult {
                                            name: call.name.clone(),
                                            response: json!({"error": format!("Rejected: {detail}. Fix it and call {final_name} again.")}),
                                        });
                                    }
                                    Err(detail) => {
                                        return Err(AgentError::InvalidOutput { agent: task.agent.to_string(), detail });
                                    }
                                }
                                continue;
                            }
                            let t0 = Instant::now();
                            let out = tools.dispatch(call).instrument(tracing::info_span!("tool_call", tool = %call.name)).await;
                            tracing::info!(agent = task.agent, tool = %call.name, ok = out.is_ok(), latency_ms = t0.elapsed().as_millis() as u64, "tool call");
                            let response = match out {
                                Ok(v) if v.is_object() => v,
                                Ok(v) => json!({ "result": v }),
                                Err(e) => json!({ "error": e }),
                            };
                            results.push(ToolResult { name: call.name.clone(), response });
                        }
                        transcript.push(Turn::ToolResults(results));
                    }
                    ToolStep::Text { text, raw } => {
                        transcript.push(Turn::ModelText { text: text.clone(), raw });
                        transcript.push(Turn::User(vec![Part::Text(format!(
                            "Do not answer in text. Call {final_name} with your final answer, or call another tool."
                        ))]));
                    }
                }
            }
            Err(AgentError::Budget(format!("{} exceeded {} steps", task.agent, self.budgets.max_steps)))
        }
        .instrument(span);
        tokio::time::timeout(self.budgets.timeout, fut).await.map_err(|_| AgentError::Timeout(self.budgets.timeout))?
    }
}
