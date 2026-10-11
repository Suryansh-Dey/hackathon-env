//! Deterministic mock model for CI, local development and demos
//! (`AI_MODE=mock`, external_api.md #11).
//!
//! Agents register responders keyed by agent name; unknown agents get a
//! schema-shaped default so every code path still runs offline.

use crate::model::*;
use async_trait::async_trait;
use serde_json::{Map, Value, json};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub type JsonResponder = Arc<dyn Fn(&JsonRequest) -> Result<String, ModelError> + Send + Sync>;
pub type ToolResponder = Arc<dyn Fn(&ToolRequest, &[Turn]) -> Result<ToolStep, ModelError> + Send + Sync>;

pub const MOCK_MODEL_ID: &str = "mock-1";

#[derive(Clone, Default)]
pub struct MockModel {
    json: HashMap<String, JsonResponder>,
    tools: HashMap<String, ToolResponder>,
    calls: Arc<AtomicU64>,
}

impl std::fmt::Debug for MockModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockModel")
            .field("json_agents", &self.json.keys().collect::<Vec<_>>())
            .field("tool_agents", &self.tools.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl MockModel {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a JSON responder that returns raw response text for `agent`.
    pub fn with_json(mut self, agent: &str, f: impl Fn(&JsonRequest) -> Result<String, ModelError> + Send + Sync + 'static) -> Self {
        self.json.insert(agent.to_string(), Arc::new(f));
        self
    }

    /// Registers a responder returning a serializable value.
    pub fn with_value(self, agent: &str, f: impl Fn(&JsonRequest) -> Value + Send + Sync + 'static) -> Self {
        self.with_json(agent, move |r| Ok(f(r).to_string()))
    }

    /// Replies with each item of `seq` in turn (then repeats the last); useful for retry tests.
    pub fn with_json_sequence(self, agent: &str, seq: Vec<Result<String, ModelError>>) -> Self {
        let q = Mutex::new(VecDeque::from(seq));
        let last: Mutex<Option<Result<String, ModelError>>> = Mutex::new(None);
        self.with_json(agent, move |_| {
            let mut q = q.lock().unwrap();
            let mut last = last.lock().unwrap();
            match q.pop_front() {
                Some(r) => {
                    *last = Some(r.clone());
                    r
                }
                None => last.clone().unwrap_or(Err(ModelError::Empty)),
            }
        })
    }

    pub fn with_tools(
        mut self,
        agent: &str,
        f: impl Fn(&ToolRequest, &[Turn]) -> Result<ToolStep, ModelError> + Send + Sync + 'static,
    ) -> Self {
        self.tools.insert(agent.to_string(), Arc::new(f));
        self
    }

    /// Number of model calls served.
    pub fn call_count(&self) -> u64 {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ModelClient for MockModel {
    fn provider(&self) -> &'static str {
        "mock"
    }

    async fn generate_json(&self, req: &JsonRequest) -> Result<ModelReply, ModelError> {
        check_parts(&req.parts)?;
        self.calls.fetch_add(1, Ordering::SeqCst);
        let text = match self.json.get(&req.agent) {
            Some(f) => f(req)?,
            None => schema_default(&req.schema).to_string(),
        };
        let usage = Usage { prompt_tokens: approx_tokens(&req.parts), output_tokens: (text.len() / 4) as u64, ..Default::default() };
        Ok(ModelReply {
            text,
            model: MOCK_MODEL_ID.into(),
            usage: Some(Usage { total_tokens: usage.prompt_tokens + usage.output_tokens, ..usage }),
        })
    }

    async fn tool_step(&self, req: &ToolRequest, transcript: &[Turn]) -> Result<ToolStepReply, ModelError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let f = self.tools.get(&req.agent).ok_or_else(|| ModelError::NotConfigured(format!("no mock tool responder for {}", req.agent)))?;
        let step = f(req, transcript)?;
        Ok(ToolStepReply { step, model: MOCK_MODEL_ID.into(), usage: Some(Usage { total_tokens: 100, ..Default::default() }) })
    }
}

fn approx_tokens(parts: &[Part]) -> u64 {
    parts
        .iter()
        .map(|p| match p {
            Part::Text(t) => t.len() as u64 / 4,
            Part::Image { .. } => 258,
        })
        .sum()
}

/// Minimal value conforming to a Gemini schema: required fields only, empty
/// arrays, first enum value, zeros, `false`, `null` for nullable.
pub fn schema_default(schema: &Value) -> Value {
    if schema.get("nullable").and_then(Value::as_bool) == Some(true) {
        return Value::Null;
    }
    match schema.get("type").and_then(Value::as_str).unwrap_or("OBJECT") {
        "OBJECT" => {
            let mut m = Map::new();
            let required: Vec<&str> =
                schema.get("required").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
            if let Some(props) = schema.get("properties").and_then(Value::as_object) {
                for (k, v) in props {
                    if required.contains(&k.as_str()) {
                        m.insert(k.clone(), schema_default(v));
                    }
                }
            }
            Value::Object(m)
        }
        "ARRAY" => json!([]),
        "STRING" => schema.get("enum").and_then(|e| e.get(0)).cloned().unwrap_or_else(|| json!("")),
        "INTEGER" => json!(0),
        "NUMBER" => json!(0.0),
        "BOOLEAN" => json!(false),
        _ => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_default_shapes() {
        let s = json!({
            "type": "OBJECT",
            "properties": {
                "usable": {"type": "BOOLEAN"},
                "issues": {"type": "ARRAY", "items": {"type": "STRING"}},
                "hint": {"type": "STRING", "nullable": true},
                "kind": {"type": "STRING", "enum": ["A", "B"]},
                "n": {"type": "INTEGER"}
            },
            "required": ["usable", "issues", "kind", "n"]
        });
        assert_eq!(schema_default(&s), json!({"usable": false, "issues": [], "kind": "A", "n": 0}));
    }

    #[tokio::test]
    async fn sequence_responder_repeats_last() {
        let m = MockModel::new().with_json_sequence("a", vec![Err(ModelError::RateLimited("x".into())), Ok("{}".into())]);
        let req = JsonRequest {
            agent: "a".into(),
            system: String::new(),
            parts: vec![],
            schema: json!({}),
            tier: ModelTier::Flash,
            model: "m".into(),
            thinking: false,
        };
        assert!(m.generate_json(&req).await.is_err());
        assert_eq!(m.generate_json(&req).await.unwrap().text, "{}");
        assert_eq!(m.generate_json(&req).await.unwrap().text, "{}");
        assert_eq!(m.call_count(), 3);
    }
}
