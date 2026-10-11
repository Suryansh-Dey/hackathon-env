//! Live Gemini checks. Run with `GEMINI_API_KEY=... cargo test -p kabados-agent --test live -- --ignored`.

use async_trait::async_trait;
use kabados_agent::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Debug, Serialize, Deserialize)]
struct Sum {
    total: i64,
}

fn ok(_: &Sum) -> Result<(), String> {
    Ok(())
}

/// `None` (test skipped) when GEMINI_API_KEY is unset, so CI without keys stays green.
fn live_runtime() -> Option<AgentRuntime> {
    match GeminiModel::from_env() {
        Ok(m) => Some(AgentRuntime::new(Arc::new(m), ModelConfig::from_env())),
        Err(_) => {
            eprintln!("GEMINI_API_KEY not set; skipping live test");
            None
        }
    }
}

async fn json_roundtrip(tier: ModelTier, thinking: bool) {
    let Some(rt) = live_runtime() else { return };
    let out = rt
        .run_json(JsonTask {
            agent: "live-check",
            prompt_version: "live-1",
            system: "You add numbers. Reply with JSON only.".into(),
            parts: vec![Part::text("Add 17 and 25.")],
            schema: json!({"type": "OBJECT", "properties": {"total": {"type": "INTEGER"}}, "required": ["total"]}),
            tier,
            thinking,
            validate: &ok,
            cacheable: false,
        })
        .await
        .expect("live call");
    println!("{tier:?}: model={} latency_ms={} usage={:?}", out.model, out.latency_ms, out.usage);
    assert_eq!(out.value.total, 42);
}

#[tokio::test]
#[ignore]
async fn live_flash_json() {
    json_roundtrip(ModelTier::Flash, false).await;
}

#[tokio::test]
#[ignore]
async fn live_pro_json_with_thinking() {
    json_roundtrip(ModelTier::Pro, true).await;
}

struct Weather;

#[async_trait]
impl ToolDispatcher for Weather {
    async fn dispatch(&self, call: &ToolCall) -> Result<Value, String> {
        match call.name.as_str() {
            "get_price" => Ok(json!({"category": call.args["category"], "paise_per_kg": 65000})),
            other => Err(format!("unknown tool {other}")),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Answer {
    paise_per_kg: i64,
}

#[tokio::test]
#[ignore]
async fn live_tool_loop() {
    let Some(rt) = live_runtime() else { return };
    let ok = |a: &Answer| if a.paise_per_kg > 0 { Ok(()) } else { Err("missing price".to_string()) };
    let out = rt
        .run_tools(
            ToolTask {
                agent: "live-tools",
                prompt_version: "live-1",
                system: "Use get_price to look up prices, then call submit_answer.".into(),
                initial: vec![Part::text("What does the vendor pay for metal.copper?")],
                tools: vec![ToolDecl {
                    name: "get_price".into(),
                    description: "Price per kg in paise for a category".into(),
                    parameters: json!({"type": "OBJECT", "properties": {"category": {"type": "STRING"}}, "required": ["category"]}),
                }],
                final_tool: ToolDecl {
                    name: "submit_answer".into(),
                    description: "Final answer".into(),
                    parameters: json!({"type": "OBJECT", "properties": {"paise_per_kg": {"type": "INTEGER"}}, "required": ["paise_per_kg"]}),
                },
                tier: ModelTier::Pro,
                thinking: true,
                validate: &ok,
            },
            &Weather,
        )
        .await
        .expect("tool loop");
    println!("tools: model={} latency_ms={} attempts={}", out.model, out.latency_ms, out.attempts);
    assert_eq!(out.value.paise_per_kg, 65000);
}
