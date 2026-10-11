//! `kabados-evals run --suite <itemize|hazard|verify|scale|rules|triage|all> [--live] [--out report.json] [--gate]`

use anyhow::Result;
use clap::{Parser, Subcommand};
use kabados_agent::{AgentRuntime, GeminiModel, ModelConfig};
use kabados_evals::{Suite, default_evals_dir, run};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "kabados-evals", about = "Replay KabadOS golden sets through the AI agents")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run one suite (or all) and print a JSON report.
    Run {
        /// Suite to run; omit for all suites.
        #[arg(long, value_enum)]
        suite: Option<Suite>,
        /// Use the live Gemini API (needs GEMINI_API_KEY); default is the offline mock.
        #[arg(long)]
        live: bool,
        /// Write the JSON report here instead of stdout.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Exit non-zero when a release threshold is missed.
        #[arg(long)]
        gate: bool,
        /// Golden-set directory (default: <repo>/evals).
        #[arg(long)]
        evals_dir: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // Model/tool call spans (latency, tokens, retries) on stderr, e.g. RUST_LOG=kabados_agent=info.
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()))
        .with_writer(std::io::stderr)
        .init();
    let Cmd::Run { suite, live, out, gate, evals_dir } = Cli::parse().cmd;
    let rt =
        if live { AgentRuntime::new(Arc::new(GeminiModel::from_env()?), ModelConfig::from_env()) } else { kabados_agents::mock_runtime() };
    let suites: Vec<Suite> = suite.map(|s| vec![s]).unwrap_or_else(|| Suite::ALL.to_vec());
    let dir = evals_dir.unwrap_or_else(default_evals_dir);
    let result = run(&rt, &suites, &dir, false).await?;
    let json = serde_json::to_string_pretty(&result)?;
    match out {
        Some(p) => std::fs::write(&p, &json)?,
        None => println!("{json}"),
    }
    for r in &result {
        eprintln!(
            "{:?} [{}]: {} cases, {:?}, gate {}",
            r.suite,
            r.mode,
            r.cases,
            r.metrics,
            if r.gate_passed() { "passed" } else { "FAILED" }
        );
    }
    if gate && result.iter().any(|r| !r.gate_passed()) {
        anyhow::bail!("eval gate failed");
    }
    Ok(())
}
