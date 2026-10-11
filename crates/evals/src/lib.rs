//! `kabados-evals`: replays golden sets through the same agent code and
//! computes the release metrics (PLAN.md §11.10). Regressions in hazard recall
//! or category accuracy fail the gate.

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use kabados_agent::AgentRuntime;
use kabados_agents::synthetic;
use kabados_agents::{ImageInput, ItemizeInput, RulesAuthorInput, VerifyInput};
use kabados_domain::Grams;
use kabados_domain::geo::LatLng;
use kabados_domain::ids::{ImageId, ItemId, RuleSetId, UserId};
use kabados_domain::ladder::LadderLevel;
use kabados_domain::listing::{Hazard, Item, ItemSource, ShotType, Verdict};
use kabados_domain::ruleset::{Minimums, RuleLine, RuleSet, RuleSetStatus, Schedule, ServiceArea};
use kabados_domain::taxonomy::{Taxonomy, Unit};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum Suite {
    Itemize,
    Hazard,
    Verify,
    Scale,
    Rules,
    Triage,
}

impl Suite {
    pub const ALL: [Suite; 6] = [Suite::Itemize, Suite::Hazard, Suite::Verify, Suite::Scale, Suite::Rules, Suite::Triage];

    pub fn dir(self) -> &'static str {
        match self {
            Suite::Itemize => "itemize",
            Suite::Hazard => "hazard",
            Suite::Verify => "verify",
            Suite::Scale => "scale",
            Suite::Rules => "rules",
            Suite::Triage => "triage",
        }
    }
}

/// Release thresholds (PLAN.md §11.4: top-1 ≥ 90%, hazard recall ≥ 99%).
pub const THRESHOLDS: &[(&str, f64)] = &[
    ("hazard_recall", 0.99),
    ("category_accuracy", 0.90),
    ("verdict_check_accuracy", 0.90),
    ("scale_exact", 0.95),
    ("rules_field_accuracy", 0.90),
    ("triage_accuracy", 0.90),
];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "render", rename_all = "snake_case")]
pub enum ImageSpec {
    Scale {
        text: String,
        unit: String,
    },
    Flat {
        rgb: [u8; 3],
    },
    Busy {
        seed: u32,
    },
    /// A labelled fixture photo, relative to the suite directory.
    File {
        path: String,
    },
}

impl ImageSpec {
    fn load(&self, suite_dir: &Path) -> Result<ImageInput> {
        let (mime, bytes) = match self {
            ImageSpec::Scale { text, unit } => ("image/png", synthetic::render_scale(text, unit)),
            ImageSpec::Flat { rgb } => ("image/png", synthetic::render_flat(320, 240, *rgb)),
            ImageSpec::Busy { seed } => ("image/png", synthetic::render_busy(320, 240, *seed)),
            ImageSpec::File { path } => {
                let p = suite_dir.join(path);
                let mime = if path.ends_with(".png") { "image/png" } else { "image/jpeg" };
                (mime, std::fs::read(&p).with_context(|| format!("reading {}", p.display()))?)
            }
        };
        Ok(ImageInput::new(ImageId::new(), mime, bytes))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Manifest<C> {
    pub suite: String,
    pub version: String,
    #[serde(default)]
    pub note: String,
    pub cases: Vec<C>,
}

// ---------- case types ----------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExpectedItem {
    pub category_id: String,
    #[serde(default)]
    pub quantity: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ItemizeCase {
    pub id: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub images: Vec<ImageSpec>,
    pub expected: Vec<ExpectedItem>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CaseItem {
    pub category_id: String,
    pub kg: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct VerifyChecks {
    /// No claim may be SUPPORTED (e.g. when there are no photos).
    #[serde(default)]
    pub no_supported: bool,
    #[serde(default)]
    pub min_claims: Option<usize>,
    #[serde(default)]
    pub max_proofs: Option<usize>,
    /// At least one proof request uses one of these shot types.
    #[serde(default)]
    pub proof_shot_any: Vec<ShotType>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VerifyCase {
    pub id: String,
    pub items: Vec<CaseItem>,
    #[serde(default)]
    pub images: Vec<ImageSpec>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub claim_reason: Option<String>,
    #[serde(default)]
    pub prior_rounds: u8,
    pub checks: VerifyChecks,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScaleCase {
    pub id: String,
    pub image: ImageSpec,
    pub expected_g: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LineSpec {
    pub category_id: String,
    #[serde(default)]
    pub grade: Option<String>,
    #[serde(default)]
    pub price_per_kg_paise: Option<i64>,
    #[serde(default)]
    pub price_per_piece_paise: Option<i64>,
    #[serde(default)]
    pub min_weight_g: Option<i64>,
}

impl LineSpec {
    fn to_rule_line(&self) -> RuleLine {
        RuleLine {
            category_id: self.category_id.clone(),
            grade: self.grade.clone(),
            unit: None,
            price_per_kg_paise: self.price_per_kg_paise,
            price_per_piece_paise: self.price_per_piece_paise,
            min_weight_g: self.min_weight_g,
            conditions: Default::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RulesCase {
    pub id: String,
    pub instruction: String,
    pub now: DateTime<chrono::FixedOffset>,
    #[serde(default)]
    pub current_lines: Option<Vec<LineSpec>>,
    pub expected_lines: Vec<LineSpec>,
    /// Categories that must end up excluded (by any glob).
    #[serde(default)]
    pub expected_excluded: Vec<String>,
    #[serde(default)]
    pub expected_effective_from: Option<DateTime<chrono::FixedOffset>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TriageCase {
    pub id: String,
    pub image: ImageSpec,
    pub expected_usable: bool,
}

// ---------- report ----------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Failure {
    pub case_id: String,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GateCheck {
    pub metric: String,
    pub threshold: f64,
    pub value: f64,
    pub passed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub suite: Suite,
    pub mode: String,
    pub models: kabados_agent::ModelConfig,
    pub manifest_version: String,
    pub started_at: DateTime<Utc>,
    pub duration_ms: u64,
    pub cases: usize,
    pub metrics: BTreeMap<String, f64>,
    pub failures: Vec<Failure>,
    pub gate: Vec<GateCheck>,
}

impl Report {
    pub fn gate_passed(&self) -> bool {
        self.gate.iter().all(|g| g.passed)
    }

    /// Evaluates every metric that has a threshold.
    pub fn apply_gate(&mut self) {
        self.gate = THRESHOLDS
            .iter()
            .filter_map(|(m, t)| {
                self.metrics.get(*m).map(|v| GateCheck { metric: m.to_string(), threshold: *t, value: *v, passed: *v >= *t })
            })
            .collect();
    }
}

fn ratio(n: usize, d: usize) -> f64 {
    if d == 0 { 1.0 } else { n as f64 / d as f64 }
}

fn load<C: for<'de> Deserialize<'de>>(dir: &Path) -> Result<Manifest<C>> {
    let p = dir.join("manifest.json");
    let text = std::fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parsing {}", p.display()))
}

/// Default golden-set directory: `<repo>/evals`.
pub fn default_evals_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../evals")
}

pub async fn run_suite(rt: &AgentRuntime, suite: Suite, evals_dir: &Path) -> Result<Report> {
    let dir = evals_dir.join(suite.dir());
    let started_at = Utc::now();
    let t0 = Instant::now();
    let (version, cases, metrics, failures) = match suite {
        Suite::Itemize | Suite::Hazard => itemize_suite(rt, &dir).await?,
        Suite::Verify => verify_suite(rt, &dir).await?,
        Suite::Scale => scale_suite(rt, &dir).await?,
        Suite::Rules => rules_suite(rt, &dir).await?,
        Suite::Triage => triage_suite(rt, &dir).await?,
    };
    let mut report = Report {
        suite,
        mode: rt.provider().to_string(),
        models: rt.models().clone(),
        manifest_version: version,
        started_at,
        duration_ms: t0.elapsed().as_millis() as u64,
        cases,
        metrics,
        failures,
        gate: vec![],
    };
    report.apply_gate();
    Ok(report)
}

type SuiteOut = (String, usize, BTreeMap<String, f64>, Vec<Failure>);

async fn itemize_suite(rt: &AgentRuntime, dir: &Path) -> Result<SuiteOut> {
    let m: Manifest<ItemizeCase> = load(dir)?;
    let tax = Taxonomy::seed();
    let (mut expected_n, mut matched, mut qty_n, mut qty_ok, mut haz_n, mut haz_ok) = (0, 0, 0, 0, 0, 0);
    let mut failures = Vec::new();
    for c in &m.cases {
        let images = c.images.iter().map(|s| s.load(dir)).collect::<Result<Vec<_>>>()?;
        let predicted = match kabados_agents::itemize(rt, &ItemizeInput { images, description: c.description.clone(), taxonomy: tax }).await
        {
            Ok(o) => o.value.items,
            Err(e) => {
                failures.push(Failure { case_id: c.id.clone(), detail: format!("agent error: {e}") });
                vec![]
            }
        };
        let mut used = vec![false; predicted.len()];
        for exp in &c.expected {
            expected_n += 1;
            let hazardous = tax.is_hazardous(&exp.category_id);
            if hazardous {
                haz_n += 1;
            }
            let hit = predicted.iter().enumerate().position(|(i, p)| !used[i] && p.category_id == exp.category_id);
            match hit {
                Some(i) => {
                    used[i] = true;
                    matched += 1;
                    if let Some(q) = exp.quantity {
                        qty_n += 1;
                        if predicted[i].quantity == q {
                            qty_ok += 1;
                        } else {
                            failures.push(Failure {
                                case_id: c.id.clone(),
                                detail: format!("{}: quantity {} != {q}", exp.category_id, predicted[i].quantity),
                            });
                        }
                    }
                    if hazardous && predicted[i].hazard.is_hazardous {
                        haz_ok += 1;
                    }
                }
                None => {
                    failures.push(Failure {
                        case_id: c.id.clone(),
                        detail: format!(
                            "missing {} (got {:?})",
                            exp.category_id,
                            predicted.iter().map(|p| &p.category_id).collect::<Vec<_>>()
                        ),
                    });
                    // A hazardous item still counts as recalled if an unmatched prediction flagged a hazard.
                    if hazardous && let Some(i) = predicted.iter().enumerate().position(|(i, p)| !used[i] && p.hazard.is_hazardous) {
                        used[i] = true;
                        haz_ok += 1;
                    }
                }
            }
        }
    }
    let metrics = BTreeMap::from([
        ("category_accuracy".to_string(), ratio(matched, expected_n)),
        ("quantity_accuracy".to_string(), ratio(qty_ok, qty_n)),
        ("hazard_recall".to_string(), ratio(haz_ok, haz_n)),
        ("expected_items".to_string(), expected_n as f64),
        ("hazardous_items".to_string(), haz_n as f64),
    ]);
    Ok((m.version, m.cases.len(), metrics, failures))
}

fn case_item(c: &CaseItem, tax: &Taxonomy) -> Item {
    let cat = tax.get(&c.category_id);
    Item {
        id: ItemId::new(),
        category_id: c.category_id.clone(),
        grade: None,
        attributes: BTreeMap::new(),
        quantity: 1,
        unit: cat.map(|c| c.unit).unwrap_or(Unit::Kg),
        est_weight_g: Some(Grams::from_kg(c.kg)),
        plausible_weight: None,
        hazard: Hazard { is_hazardous: tax.is_hazardous(&c.category_id), class: cat.and_then(|c| c.hazard_class), handling: None },
        evidence: vec![],
        source: ItemSource::Seller,
        ai_category_id: None,
        ai_confidence: None,
        ladder: LadderLevel::L0Claimed,
        note: None,
    }
}

async fn verify_suite(rt: &AgentRuntime, dir: &Path) -> Result<SuiteOut> {
    let m: Manifest<VerifyCase> = load(dir)?;
    let tax = Taxonomy::seed();
    let (mut checks, mut passed) = (0, 0);
    let mut failures = Vec::new();
    for c in &m.cases {
        let images = c.images.iter().map(|s| s.load(dir)).collect::<Result<Vec<_>>>()?;
        let input = VerifyInput {
            items: c.items.iter().map(|i| case_item(i, tax)).collect(),
            images,
            description: c.description.clone(),
            asking_price: None,
            claim_reason: c.claim_reason.clone(),
            prior_rounds: c.prior_rounds,
            label_text: BTreeMap::new(),
        };
        let out = match kabados_agents::verify(rt, &input, tax).await {
            Ok(o) => o.value,
            Err(e) => {
                failures.push(Failure { case_id: c.id.clone(), detail: format!("agent error: {e}") });
                let n = [
                    c.checks.no_supported,
                    c.checks.min_claims.is_some(),
                    c.checks.max_proofs.is_some(),
                    !c.checks.proof_shot_any.is_empty(),
                ]
                .iter()
                .filter(|b| **b)
                .count();
                checks += n;
                continue;
            }
        };
        let mut check = |ok: bool, what: String| {
            checks += 1;
            if ok {
                passed += 1;
            } else {
                failures.push(Failure { case_id: c.id.clone(), detail: what });
            }
        };
        if c.checks.no_supported {
            check(out.claims.iter().all(|x| x.verdict != Verdict::Supported), "a claim was SUPPORTED without photo evidence".into());
        }
        if let Some(n) = c.checks.min_claims {
            check(out.claims.len() >= n, format!("{} claims < {n}", out.claims.len()));
        }
        if let Some(n) = c.checks.max_proofs {
            check(out.proof_requests.len() <= n, format!("{} proof requests > {n}", out.proof_requests.len()));
        }
        if !c.checks.proof_shot_any.is_empty() {
            check(
                out.proof_requests.iter().any(|p| c.checks.proof_shot_any.contains(&p.shot_type)),
                format!(
                    "no proof request of type {:?} (got {:?})",
                    c.checks.proof_shot_any,
                    out.proof_requests.iter().map(|p| p.shot_type).collect::<Vec<_>>()
                ),
            );
        }
    }
    let metrics = BTreeMap::from([("verdict_check_accuracy".to_string(), ratio(passed, checks)), ("checks".to_string(), checks as f64)]);
    Ok((m.version, m.cases.len(), metrics, failures))
}

async fn scale_suite(rt: &AgentRuntime, dir: &Path) -> Result<SuiteOut> {
    let m: Manifest<ScaleCase> = load(dir)?;
    let (mut exact, mut within) = (0, 0);
    let mut failures = Vec::new();
    for c in &m.cases {
        let img = c.image.load(dir)?;
        match kabados_agents::read_scale(rt, &img).await {
            Ok(o) => match o.value.reading_g {
                Some(g) if g == c.expected_g => {
                    exact += 1;
                    within += 1;
                }
                Some(g) => {
                    if (g - c.expected_g).abs() * 100 <= c.expected_g.abs() {
                        within += 1;
                    }
                    failures.push(Failure { case_id: c.id.clone(), detail: format!("read {g} g, expected {} g", c.expected_g) });
                }
                None => failures.push(Failure { case_id: c.id.clone(), detail: "no reading".into() }),
            },
            Err(e) => failures.push(Failure { case_id: c.id.clone(), detail: format!("agent error: {e}") }),
        }
    }
    let n = m.cases.len();
    let metrics = BTreeMap::from([("scale_exact".to_string(), ratio(exact, n)), ("scale_within_1pct".to_string(), ratio(within, n))]);
    Ok((m.version, n, metrics, failures))
}

fn current_rule_set(lines: &[LineSpec], now: DateTime<Utc>) -> RuleSet {
    RuleSet {
        rule_set_id: RuleSetId::new(),
        vendor_id: UserId::new(),
        version: 1,
        status: RuleSetStatus::Published,
        effective_from: (now - chrono::Duration::days(7)).with_timezone(&kabados_agents::rules::ist()),
        service_area: ServiceArea::Radius { center: LatLng { lat: 28.61, lng: 77.21 }, radius_km: 8.0 },
        schedule: Schedule {
            days: vec![kabados_domain::listing::Weekday::Mon],
            slots: vec!["09:00-18:00".into()],
            daily_capacity_kg: None,
        },
        minimums: Minimums::default(),
        lines: lines.iter().map(LineSpec::to_rule_line).collect(),
        modifiers: vec![],
        exclusions: vec![],
        accept_partial_lots: true,
        source: None,
    }
}

async fn rules_suite(rt: &AgentRuntime, dir: &Path) -> Result<SuiteOut> {
    let m: Manifest<RulesCase> = load(dir)?;
    let (mut fields, mut ok) = (0, 0);
    let mut failures = Vec::new();
    for c in &m.cases {
        let now = c.now.with_timezone(&Utc);
        let input = RulesAuthorInput {
            instruction: c.instruction.clone(),
            current: c.current_lines.as_ref().map(|l| current_rule_set(l, now)),
            taxonomy: Taxonomy::seed(),
            rate_index_paise_per_kg: BTreeMap::new(),
            hazardous_authorized: true,
            vendor_id: UserId::new(),
            now,
            yard: Some(LatLng { lat: 28.61, lng: 77.21 }),
        };
        let total = c.expected_lines.len() * 2
            + c.expected_lines.iter().filter(|l| l.min_weight_g.is_some()).count()
            + c.expected_excluded.len()
            + c.expected_effective_from.is_some() as usize;
        fields += total;
        let draft = match kabados_agents::author_rules(rt, &input).await {
            Ok(o) => o.value.draft,
            Err(e) => {
                failures.push(Failure { case_id: c.id.clone(), detail: format!("agent error: {e}") });
                continue;
            }
        };
        let mut check = |good: bool, what: String| {
            if good {
                ok += 1;
            } else {
                failures.push(Failure { case_id: c.id.clone(), detail: what });
            }
        };
        for exp in &c.expected_lines {
            let line = draft.lines.iter().find(|l| l.category_id == exp.category_id && (exp.grade.is_none() || l.grade == exp.grade));
            check(line.is_some(), format!("missing line {}", exp.category_id));
            let price = line.map(|l| (l.price_per_kg_paise, l.price_per_piece_paise));
            check(
                price == Some((exp.price_per_kg_paise, exp.price_per_piece_paise)),
                format!("{} price {:?} != {:?}", exp.category_id, price, (exp.price_per_kg_paise, exp.price_per_piece_paise)),
            );
            if exp.min_weight_g.is_some() {
                let got = line.and_then(|l| l.min_weight_g);
                check(got == exp.min_weight_g, format!("{} min weight {:?} != {:?}", exp.category_id, got, exp.min_weight_g));
            }
        }
        for cat in &c.expected_excluded {
            check(draft.is_excluded(cat), format!("{cat} not excluded (exclusions {:?})", draft.exclusions));
        }
        if let Some(t) = c.expected_effective_from {
            check(draft.effective_from == t, format!("effective_from {} != {t}", draft.effective_from));
        }
    }
    let metrics = BTreeMap::from([("rules_field_accuracy".to_string(), ratio(ok, fields)), ("fields".to_string(), fields as f64)]);
    Ok((m.version, m.cases.len(), metrics, failures))
}

async fn triage_suite(rt: &AgentRuntime, dir: &Path) -> Result<SuiteOut> {
    let m: Manifest<TriageCase> = load(dir)?;
    let mut ok = 0;
    let mut failures = Vec::new();
    for c in &m.cases {
        let img = c.image.load(dir)?;
        match kabados_agents::triage(rt, &img).await {
            Ok(o) if o.value.usable == c.expected_usable => ok += 1,
            Ok(o) => {
                failures.push(Failure { case_id: c.id.clone(), detail: format!("usable={} issues={:?}", o.value.usable, o.value.issues) })
            }
            Err(e) => failures.push(Failure { case_id: c.id.clone(), detail: format!("agent error: {e}") }),
        }
    }
    let metrics = BTreeMap::from([("triage_accuracy".to_string(), ratio(ok, m.cases.len()))]);
    Ok((m.version, m.cases.len(), metrics, failures))
}

/// Runs suites and fails when `gate` is set and any threshold is missed.
pub async fn run(rt: &AgentRuntime, suites: &[Suite], evals_dir: &Path, gate: bool) -> Result<Vec<Report>> {
    let mut reports = Vec::new();
    for s in suites {
        reports.push(run_suite(rt, *s, evals_dir).await?);
    }
    if gate {
        let failed: Vec<String> = reports
            .iter()
            .flat_map(|r| {
                r.gate.iter().filter(|g| !g.passed).map(move |g| format!("{:?}: {} {:.3} < {}", r.suite, g.metric, g.value, g.threshold))
            })
            .collect();
        if !failed.is_empty() {
            bail!("eval gate failed: {}", failed.join("; "));
        }
    }
    Ok(reports)
}
