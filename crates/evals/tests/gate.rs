//! The golden sets must pass the release gate deterministically in mock mode
//! (CI). The `live_` test replays them against Gemini when a key is present.

use kabados_agent::{AgentRuntime, GeminiModel, ModelConfig};
use kabados_evals::{Suite, default_evals_dir, run};
use std::sync::Arc;

#[tokio::test]
async fn mock_mode_passes_the_gate_on_every_suite() {
    let rt = kabados_agents::mock_runtime();
    let reports = run(&rt, &Suite::ALL, &default_evals_dir(), true).await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(reports.len(), Suite::ALL.len());
    for r in &reports {
        assert!(r.cases > 0, "{:?} has no cases", r.suite);
        assert!(r.gate_passed(), "{:?}: {:?} failures {:?}", r.suite, r.gate, r.failures);
        assert!(!r.gate.is_empty(), "{:?} has no gated metric", r.suite);
    }
    let hazard = reports.iter().find(|r| r.suite == Suite::Hazard).unwrap();
    assert_eq!(hazard.metrics["hazard_recall"], 1.0);
}

#[tokio::test]
async fn gate_fails_when_thresholds_are_missed() {
    let rt = kabados_agents::mock_runtime();
    let mut reports = run(&rt, &[Suite::Itemize], &default_evals_dir(), false).await.unwrap();
    let r = &mut reports[0];
    r.metrics.insert("hazard_recall".into(), 0.5);
    r.apply_gate();
    assert!(!r.gate_passed());
}

#[tokio::test]
#[ignore]
async fn live_eval_all_suites() {
    let Ok(model) = GeminiModel::from_env() else {
        eprintln!("GEMINI_API_KEY not set; skipping live evals");
        return;
    };
    let rt = AgentRuntime::new(Arc::new(model), ModelConfig::from_env());
    let reports = run(&rt, &Suite::ALL, &default_evals_dir(), false).await.unwrap();
    for r in &reports {
        println!("{:?}: {:?} gate={} failures={:?}", r.suite, r.metrics, r.gate_passed(), r.failures);
    }
    assert!(reports.iter().all(|r| r.gate_passed()), "live evals missed a release threshold");
}
