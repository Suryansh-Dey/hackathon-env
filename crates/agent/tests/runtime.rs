//! Runtime behaviour against the mock model (no network).

use async_trait::async_trait;
use kabados_agent::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Out {
    n: i64,
}

fn no_check(_: &Out) -> Result<(), String> {
    Ok(())
}

fn positive(o: &Out) -> Result<(), String> {
    if o.n > 0 { Ok(()) } else { Err("n must be positive".into()) }
}

fn rt(m: MockModel) -> AgentRuntime {
    AgentRuntime::new(Arc::new(m), ModelConfig::default()).with_retry(RetryPolicy {
        max_retries: 3,
        base_delay: Duration::from_millis(1),
        max_delay: Duration::from_millis(2),
    })
}

fn task<'a>(validate: Validator<'a, Out>, cacheable: bool) -> JsonTask<'a, Out> {
    JsonTask {
        agent: "t",
        prompt_version: "t-1",
        system: "sys".into(),
        parts: vec![Part::text("hello")],
        schema: json!({"type": "OBJECT", "properties": {"n": {"type": "INTEGER"}}, "required": ["n"]}),
        tier: ModelTier::Flash,
        thinking: false,
        validate,
        cacheable,
    }
}

#[tokio::test]
async fn retries_rate_limits_then_succeeds() {
    let m = MockModel::new().with_json_sequence(
        "t",
        vec![Err(ModelError::RateLimited("429".into())), Err(ModelError::Server("503".into())), Ok(r#"{"n": 3}"#.into())],
    );
    let out = rt(m).run_json(task(&no_check, false)).await.unwrap();
    assert_eq!(out.value, Out { n: 3 });
    assert_eq!(out.attempts, 3);
    assert_eq!(out.prompt_version, "t-1");
}

#[tokio::test]
async fn non_retryable_errors_surface_immediately() {
    let m = MockModel::new().with_json_sequence("t", vec![Err(ModelError::Client("400".into()))]);
    let err = rt(m.clone()).run_json(task(&no_check, false)).await.unwrap_err();
    assert!(matches!(err, AgentError::Model(ModelError::Client(_))));
    assert_eq!(m.call_count(), 1);
}

#[tokio::test]
async fn one_repair_then_success() {
    let m = MockModel::new().with_json_sequence("t", vec![Ok("```json\n{\"n\": 0}\n```".into()), Ok(r#"{"n": 5}"#.into())]);
    let out = rt(m).run_json(task(&positive, false)).await.unwrap();
    assert_eq!(out.value.n, 5);
    assert_eq!(out.attempts, 2);
}

#[tokio::test]
async fn fails_closed_after_one_repair() {
    let m = MockModel::new().with_json_sequence("t", vec![Ok("not json".into()), Ok(r#"{"n": -1}"#.into()), Ok(r#"{"n": 9}"#.into())]);
    let err = rt(m.clone()).run_json(task(&positive, false)).await.unwrap_err();
    assert!(matches!(err, AgentError::InvalidOutput { .. }), "{err:?}");
    assert_eq!(m.call_count(), 2, "exactly one repair attempt");
}

#[tokio::test]
async fn caches_validated_outputs() {
    let m = MockModel::new().with_value("t", |_| json!({"n": 7}));
    let r = rt(m.clone());
    let a = r.run_json(task(&no_check, true)).await.unwrap();
    let b = r.run_json(task(&no_check, true)).await.unwrap();
    assert!(!a.cached && b.cached);
    assert_eq!(b.value, a.value);
    assert_eq!(m.call_count(), 1);
}

#[tokio::test]
async fn token_budget_is_enforced() {
    let m = MockModel::new().with_value("t", |_| json!({"n": 1}));
    let r = rt(m).with_budgets(Budgets { max_tokens: 1, ..Budgets::default() });
    assert!(matches!(r.run_json(task(&no_check, false)).await.unwrap_err(), AgentError::Budget(_)));
}

#[tokio::test]
async fn unregistered_agents_get_schema_defaults() {
    let out = rt(MockModel::new()).run_json(task(&no_check, false)).await.unwrap();
    assert_eq!(out.value, Out { n: 0 });
    assert_eq!(out.model, mock::MOCK_MODEL_ID);
}

#[tokio::test]
async fn oversized_images_are_rejected_before_any_call() {
    let m = MockModel::new();
    let mut t = task(&no_check, false);
    t.parts.push(Part::Image { image_id: "img".into(), mime: "image/jpeg".into(), bytes: vec![0; MAX_IMAGE_BYTES + 1] });
    assert!(matches!(rt(m.clone()).run_json(t).await.unwrap_err(), AgentError::InvalidInput(_)));
    assert_eq!(m.call_count(), 0);
}

struct Adder {
    calls: AtomicU32,
}

#[async_trait]
impl ToolDispatcher for Adder {
    async fn dispatch(&self, call: &ToolCall) -> Result<Value, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match call.name.as_str() {
            "add" => Ok(json!({"sum": call.args["a"].as_i64().unwrap_or(0) + call.args["b"].as_i64().unwrap_or(0)})),
            other => Err(format!("unknown tool {other}")),
        }
    }
}

fn tool_task<'a>(validate: Validator<'a, Out>) -> ToolTask<'a, Out> {
    ToolTask {
        agent: "calc",
        prompt_version: "calc-1",
        system: "use tools".into(),
        initial: vec![Part::text("add 2 and 3")],
        tools: vec![ToolDecl { name: "add".into(), description: "adds".into(), parameters: json!({"type": "OBJECT"}) }],
        final_tool: ToolDecl { name: "submit".into(), description: "final".into(), parameters: json!({"type": "OBJECT"}) },
        tier: ModelTier::Pro,
        thinking: true,
        validate,
    }
}

fn last_sum(transcript: &[Turn]) -> Option<i64> {
    transcript.iter().rev().find_map(|t| match t {
        Turn::ToolResults(r) => r.iter().find(|x| x.name == "add").and_then(|x| x.response["sum"].as_i64()),
        _ => None,
    })
}

#[tokio::test]
async fn tool_loop_dispatches_and_submits() {
    let m = MockModel::new().with_tools("calc", |_, t| {
        Ok(match last_sum(t) {
            None => ToolStep::Calls { calls: vec![ToolCall { name: "add".into(), args: json!({"a": 2, "b": 3}) }], raw: None },
            Some(s) => ToolStep::Calls { calls: vec![ToolCall { name: "submit".into(), args: json!({"n": s}) }], raw: None },
        })
    });
    let tools = Adder { calls: AtomicU32::new(0) };
    let out = rt(m).run_tools(tool_task(&positive), &tools).await.unwrap();
    assert_eq!(out.value.n, 5);
    assert_eq!(tools.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn tool_loop_repairs_invalid_final_answer_once() {
    let m = MockModel::new().with_tools("calc", |_, t| {
        let rejected = t.iter().any(|x| matches!(x, Turn::ToolResults(r) if r.iter().any(|y| y.response.get("error").is_some())));
        let n = if rejected { 4 } else { -4 };
        Ok(ToolStep::Calls { calls: vec![ToolCall { name: "submit".into(), args: json!({"n": n}) }], raw: None })
    });
    let out = rt(m).run_tools(tool_task(&positive), &Adder { calls: AtomicU32::new(0) }).await.unwrap();
    assert_eq!(out.value.n, 4);
}

#[tokio::test]
async fn tool_loop_step_budget() {
    let m = MockModel::new().with_tools("calc", |_, _| Ok(ToolStep::Text { text: "thinking...".into(), raw: None }));
    let r = rt(m).with_budgets(Budgets { max_steps: 3, ..Budgets::default() });
    let err = r.run_tools(tool_task(&positive), &Adder { calls: AtomicU32::new(0) }).await.unwrap_err();
    assert!(matches!(err, AgentError::Budget(_)));
}

#[test]
fn extract_json_handles_fences_and_prose() {
    assert_eq!(extract_json("```json\n{\"a\":1}\n```"), "{\"a\":1}");
    assert_eq!(extract_json("Here you go: {\"a\":1} thanks"), "{\"a\":1}");
    assert_eq!(extract_json("[1,2]"), "[1,2]");
}

#[test]
fn backoff_is_bounded() {
    let p = RetryPolicy::default();
    for i in 0..20 {
        assert!(p.delay(i) <= p.max_delay);
    }
}

#[test]
fn ai_mode_defaults_to_mock() {
    // SAFETY: tests in this binary do not read AI_MODE concurrently.
    unsafe { std::env::remove_var("AI_MODE") };
    assert_eq!(AiMode::from_env().unwrap(), AiMode::Mock);
    assert_eq!(client_from_env().provider(), "mock");
}
