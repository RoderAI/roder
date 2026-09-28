//! An end-to-end eval corpus: fixture pages, one task each, graded on what
//! a user could observe afterwards.
//!
//! Tasks are data, in `tests/fixtures/evals/tasks.json`. Each names a fixture
//! page, a natural-language goal, the field values the goal supplies, an
//! outcome `expect` block, and a `script`: the plan a keyless decider plays
//! plus the trace that plan must produce. Two tiers run them:
//!
//! - [`keyless`]: the scripted plan against real headless Chrome, in
//!   `cargo test`. It checks the page layer and the loop, not the model.
//! - [`live`]: the hosted decision model on the same pages and graders,
//!   opt-in and `#[ignore]`d, with the audit's request variants as toggles.
//! - [`miniwob`]: the hosted model on the public MiniWoB++ benchmark, from a
//!   checkout named by `JEV_MINIWOB_DIR`; opt-in and `#[ignore]`d.
//!
//! To add a task, add a page under `tests/fixtures/pages/` if none fits and
//! an entry to `tasks.json`; both tiers pick it up.

mod effect_variant;
mod grade;
mod probe;
mod scripted;
#[cfg(test)]
mod secret_tests;
mod variants;

mod keyless;
mod live;
mod miniwob;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::Harness;
use crate::engine::{JevDecisionClient, JevEngine, JevEngineConfig, JevRunResult};
pub(crate) use grade::Expect;
use probe::{Probed, ProbedPage};
pub(crate) use scripted::{FirstDecisionDelay, Step, StepDecider, TaskValues};

/// One eval task, as written in `tasks.json`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Task {
    pub(crate) id: String,
    /// What the task exercises, for the report table.
    pub(crate) covers: String,
    /// A fixture page name, with an optional query.
    pub(crate) page: String,
    /// What a caller would ask for; only the live tier sends it to a model.
    pub(crate) goal: String,
    /// Field label to the value the goal supplies for it.
    #[serde(default)]
    pub(crate) values: BTreeMap<String, String>,
    #[serde(default = "default_timeout")]
    pub(crate) timeout_s: u64,
    /// Hold the first decision back, so the page can move under it.
    #[serde(default)]
    pub(crate) delay_first_decision_ms: u64,
    /// Origins the run may visit; `{site}` is the fixture site's own.
    #[serde(default)]
    pub(crate) allowed_origins: Vec<String>,
    /// Run with the irreversible-action gate on.
    #[serde(default)]
    pub(crate) confirm_irreversible: bool,
    /// Authorize the gate to let a confident irreversible action through.
    #[serde(default)]
    pub(crate) authorize_irreversible: bool,
    /// Refuse cookie banners before reading each document (autoconsent
    /// injected, then `consent.js`); absent is the default, on.
    #[serde(default)]
    pub(crate) refuse_cookie_banners: Option<bool>,
    /// Outcome graders both tiers apply.
    pub(crate) expect: Expect,
    pub(crate) script: Script,
}

/// The keyless tier's plan and the trace only that plan pins.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Script {
    pub(crate) plan: Vec<Step>,
    #[serde(default)]
    pub(crate) expect: Expect,
}

fn default_timeout() -> u64 {
    60
}

fn tasks_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/evals/tasks.json")
}

/// Load the corpus, keeping only the ids in `JEV_EVAL_TASKS` when it is set.
pub(crate) fn load_tasks() -> anyhow::Result<Vec<Task>> {
    select(load_all()?, std::env::var("JEV_EVAL_TASKS").ok().as_deref())
}

fn load_all() -> anyhow::Result<Vec<Task>> {
    let path = tasks_path();
    let raw = std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))
}

/// Narrow to a comma-separated id list. An id the corpus lacks is an error,
/// so a typo cannot pass by running nothing.
fn select(tasks: Vec<Task>, wanted: Option<&str>) -> anyhow::Result<Vec<Task>> {
    let Some(wanted) = wanted.filter(|value| !value.trim().is_empty()) else {
        return Ok(tasks);
    };
    let wanted = wanted
        .split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .collect::<Vec<_>>();
    let unknown = wanted
        .iter()
        .filter(|id| !tasks.iter().any(|task| task.id == **id))
        .collect::<Vec<_>>();
    anyhow::ensure!(
        unknown.is_empty(),
        "JEV_EVAL_TASKS names unknown tasks: {unknown:?}"
    );
    Ok(tasks
        .into_iter()
        .filter(|task| wanted.contains(&task.id.as_str()))
        .collect())
}

/// Everything a run produced that a grader may look at.
pub(crate) struct Outcome {
    pub(crate) result: JevRunResult,
    pub(crate) probed: Probed,
    pub(crate) posts: Vec<super::site::Post>,
    pub(crate) wall_ms: u64,
}

/// Drive the real engine on one task's page and collect its outcome.
///
/// DOM probes are read when the engine closes the page, so they see the
/// final document, whatever the run left behind.
pub(crate) async fn run_task(
    harness: &Harness,
    task: &Task,
    decision: Arc<dyn JevDecisionClient>,
    text: Arc<dyn crate::engine::JevTextValueResolver>,
    probes: Vec<String>,
) -> anyhow::Result<Outcome> {
    let started = Instant::now();
    let refuse = task.refuse_cookie_banners.unwrap_or(true);
    let page = match refuse {
        true => harness.open_refusing(&task.page).await?,
        false => harness.open(&task.page).await?,
    };
    let target_id = page.target_id().to_string();
    let (browser, probed) = ProbedPage::new(page, probes);
    let decision: Arc<dyn JevDecisionClient> = if task.delay_first_decision_ms > 0 {
        Arc::new(FirstDecisionDelay::new(
            decision,
            Duration::from_millis(task.delay_first_decision_ms),
        ))
    } else {
        decision
    };
    let mut config = JevEngineConfig::new(task.goal.clone(), decision)
        .with_text_resolver(text)
        .with_wait(Duration::from_millis(100));
    if !task.allowed_origins.is_empty() {
        let origins = task
            .allowed_origins
            .iter()
            .map(|origin| origin.replace("{site}", harness.site.origin()))
            .collect::<Vec<_>>();
        config = config.with_scope(crate::scope::JevOriginScope::any().narrow(&origins)?);
    }
    if task.confirm_irreversible {
        config = config.with_irreversible_gate();
    }
    if task.authorize_irreversible {
        config = config.with_irreversible_authorized();
    }
    config = config.with_cookie_banner_refusal(refuse);
    let mut engine = match JevEngine::start(Box::new(browser), config).await {
        Ok(engine) => engine,
        Err(error) => {
            // The engine dropped the page unclosed; the Chrome is shared by
            // the tasks still to run, so do not leave the tab working.
            harness.close_target(&target_id).await.ok();
            return Err(error);
        }
    };
    let result = engine.run(Duration::from_secs(task.timeout_s)).await;
    engine.close().await.ok();
    let wall_ms = started.elapsed().as_millis() as u64;
    // A submit navigates, so its POST can land after the run has returned.
    let expected_posts = task.expect.posts.as_ref().map_or(0, Vec::len);
    let posts = harness
        .site
        .wait_for_posts(expected_posts, Duration::from_secs(2))
        .await;
    Ok(Outcome {
        result,
        probed: probed.take(),
        posts,
        wall_ms,
    })
}

/// One JSONL line per task.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Row {
    pub(crate) task: String,
    pub(crate) covers: String,
    pub(crate) tier: &'static str,
    pub(crate) variants: Vec<String>,
    pub(crate) pass: bool,
    pub(crate) failures: Vec<String>,
    pub(crate) status: Value,
    pub(crate) stopped_because: Option<String>,
    pub(crate) steps: usize,
    pub(crate) model_calls: usize,
    pub(crate) text_calls: usize,
    pub(crate) wall_ms: u64,
    pub(crate) url: String,
    /// Tier-specific telemetry, such as per-head confidence on live runs.
    #[serde(skip_serializing_if = "Value::is_null")]
    pub(crate) telemetry: Value,
}

impl Row {
    pub(crate) fn new(
        task: &Task,
        tier: &'static str,
        variants: Vec<String>,
        outcome: &Outcome,
        failures: Vec<String>,
    ) -> Self {
        let result = &outcome.result;
        Self {
            task: task.id.clone(),
            covers: task.covers.clone(),
            tier,
            variants,
            pass: failures.is_empty(),
            failures,
            status: serde_json::to_value(result.status).unwrap_or(Value::Null),
            stopped_because: result.stopped_because.clone(),
            steps: result.actions.len(),
            model_calls: result.model_calls,
            text_calls: result.text_calls,
            wall_ms: outcome.wall_ms,
            url: result.url.clone(),
            telemetry: Value::Null,
        }
    }

    /// A row for a task that could not run at all.
    pub(crate) fn errored(task: &Task, tier: &'static str, error: &anyhow::Error) -> Self {
        Self {
            task: task.id.clone(),
            covers: task.covers.clone(),
            tier,
            variants: Vec::new(),
            pass: false,
            failures: vec![format!("run failed: {error:#}")],
            status: Value::Null,
            stopped_because: None,
            steps: 0,
            model_calls: 0,
            text_calls: 0,
            wall_ms: 0,
            url: String::new(),
            telemetry: Value::Null,
        }
    }
}

/// Write rows to `target/jev-evals/<name>.jsonl` and return the path.
pub(crate) fn write_rows(name: &str, rows: &[Row]) -> anyhow::Result<PathBuf> {
    let dir = super::target_dir().join("jev-evals");
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let path = dir.join(format!("{name}.jsonl"));
    let mut out = String::new();
    for row in rows {
        out.push_str(&serde_json::to_string(row)?);
        out.push('\n');
    }
    std::fs::write(&path, out).with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

/// The pass/fail table, in corpus order, for the test's stderr.
pub(crate) fn table(tasks: &[Task], rows: &[Row]) -> String {
    let mut out = format!(
        "{:<26} {:<5} {:<18} {:>5} {:>6} {:>8}  {}\n",
        "task", "pass", "status", "steps", "calls", "wall_ms", "failures"
    );
    for task in tasks {
        let Some(row) = rows.iter().find(|row| row.task == task.id) else {
            continue;
        };
        out.push_str(&format!(
            "{:<26} {:<5} {:<18} {:>5} {:>6} {:>8}  {}\n",
            row.task,
            if row.pass { "pass" } else { "FAIL" },
            row.status.as_str().unwrap_or("-"),
            row.steps,
            row.model_calls,
            row.wall_ms,
            row.failures.join("; ")
        ));
    }
    let passed = rows.iter().filter(|row| row.pass).count();
    out.push_str(&format!("{passed}/{} passed\n", rows.len()));
    out
}

/// Checks that need no browser: every task parses, names a page that
/// exists, and has a plan the scripted decider can play.
pub(crate) fn validate(tasks: &[Task]) -> anyhow::Result<()> {
    let pages = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pages");
    let mut ids = Vec::new();
    for task in tasks {
        ensure!(!ids.contains(&task.id), "duplicate task id {}", task.id);
        ids.push(task.id.clone());
        let name = task.page.split('?').next().unwrap_or_default();
        ensure!(
            pages.join(name).is_file(),
            "{}: no fixture page {name}",
            task.id
        );
        ensure!(!task.goal.trim().is_empty(), "{}: empty goal", task.id);
        ensure!(
            task.confirm_irreversible || !task.authorize_irreversible,
            "{}: authorize_irreversible needs confirm_irreversible",
            task.id
        );
        ensure!(!task.script.plan.is_empty(), "{}: empty plan", task.id);
        for step in &task.script.plan {
            step.check()
                .with_context(|| format!("{}: bad plan step", task.id))?;
        }
        task.expect
            .check()
            .with_context(|| format!("{}: bad expect", task.id))?;
        task.script
            .expect
            .check()
            .with_context(|| format!("{}: bad script.expect", task.id))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_corpus_is_well_formed() {
        let tasks = load_all().unwrap();
        validate(&tasks).unwrap();
        assert!(tasks.len() >= 10, "{} tasks", tasks.len());
    }

    #[test]
    fn a_task_filter_must_name_real_tasks() {
        let ids = |tasks: Vec<Task>| tasks.into_iter().map(|task| task.id).collect::<Vec<_>>();
        let all = load_all().unwrap();
        assert_eq!(select(all.clone(), None).unwrap().len(), all.len());
        assert_eq!(select(all.clone(), Some(" ")).unwrap().len(), all.len());
        assert_eq!(
            ids(select(all.clone(), Some("pagination, contact_form")).unwrap()),
            ["contact_form", "pagination"]
        );
        let error = select(all, Some("contact-form")).unwrap_err().to_string();
        assert!(error.contains("contact-form"), "{error}");
    }
}
