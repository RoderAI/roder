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
//! - [`sessions`]: several calls on one thread's session, from
//!   `tests/fixtures/evals/sessions.json`, keyless and live.
//!
//! To add a task, add a page under `tests/fixtures/pages/` if none fits and
//! an entry to `tasks.json`; both tiers pick it up.

mod effect_variant;
pub(crate) mod fallback_live;
mod grade;
mod probe;
mod rows;
mod scripted;
#[cfg(test)]
mod secret_tests;
mod sessions;
mod text_sources;
mod variants;

mod decisions_live;
mod decisions_comparison;
mod keyless;
mod live;
mod miniwob;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, ensure};
use serde::Deserialize;

use super::Harness;
use super::fallback_script::FallbackStep;
use crate::engine::{JevDecisionClient, JevEngine, JevEngineConfig, JevRunResult};
use crate::fallback::model::FallbackModel;
use crate::fallback::{FallbackOutcome, Limits, Rules};
pub(crate) use grade::Expect;
use probe::{Probed, ProbedPage};
pub(crate) use rows::{FallbackRow, Row, table, write_rows};
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
    /// A task Jev alone cannot finish: what the fallback must reach.
    #[serde(default)]
    pub(crate) fallback: Option<FallbackTask>,
}

/// What a fallback task expects of the fallback that follows Jev.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FallbackTask {
    /// The keyless tier's scripted fallback plan.
    #[serde(default)]
    pub(crate) plan: Vec<FallbackStep>,
    /// The call's end state after the fallback, graded in both tiers.
    pub(crate) expect: Expect,
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

/// Everything a run produced that a grader may look at: Jev's own end
/// state, and after a fallback the call's.
pub(crate) struct Outcome {
    pub(crate) result: JevRunResult,
    pub(crate) probed: Probed,
    pub(crate) posts: Vec<super::site::Post>,
    pub(crate) wall_ms: u64,
    /// The end state after a fallback ran, and how it went.
    pub(crate) after: Option<Box<(Outcome, FallbackOutcome)>>,
}

/// The fallback's step ceiling in eval runs (`JEV_EVAL_FALLBACK_STEPS`).
pub(crate) fn fallback_steps() -> usize {
    std::env::var("JEV_EVAL_FALLBACK_STEPS")
        .ok()
        .and_then(|raw| raw.trim().parse().ok())
        .unwrap_or(crate::fallback::settings::DEFAULT_STEPS)
}

/// Jev's result as the call ends after a fallback: the fallback's status
/// and the page it left, with Jev's trace.
pub(crate) fn after_fallback(jev: &JevRunResult, fallback: &FallbackOutcome) -> JevRunResult {
    let mut result = jev.clone();
    result.status = fallback.status;
    result.stopped_because = fallback.stopped_because.clone();
    result.elapsed_ms += fallback.elapsed_ms;
    if let Some(page) = &fallback.last_page {
        let text = |key: &str| page[key].as_str().unwrap_or_default().to_string();
        result.url = text("url");
        result.title = text("title");
        result.visible_text = text("text");
    }
    result
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
    fallback: Option<Arc<dyn FallbackModel>>,
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
    let why = fallback
        .as_ref()
        .and_then(|_| crate::fallback::trigger::trigger(&result));
    let (Some(model), Some(why)) = (fallback, why) else {
        engine.close().await.ok();
        let wall_ms = started.elapsed().as_millis() as u64;
        // A submit navigates, so its POST can land after the run has returned.
        let expected_posts = task.expect.posts.as_ref().map_or(0, Vec::len);
        let posts = harness
            .site
            .wait_for_posts(expected_posts, Duration::from_secs(2))
            .await;
        return Ok(Outcome {
            result,
            probed: probed.take(),
            posts,
            wall_ms,
            after: None,
        });
    };
    // Jev's own end state first, then the fallback in the same tab.
    let expressions = task
        .expect
        .probes()
        .chain(
            task.fallback
                .iter()
                .flat_map(|fallback| fallback.expect.probes()),
        )
        .cloned()
        .collect::<Vec<_>>();
    let before = probe::read(harness.endpoint(), &target_id, &expressions).await;
    let posts = harness
        .site
        .wait_for_posts(
            task.expect.posts.as_ref().map_or(0, Vec::len),
            Duration::from_secs(1),
        )
        .await;
    let wall_ms = started.elapsed().as_millis() as u64;
    let tab = roder_ext_chrome::direct::DirectTab::Target {
        endpoint: harness.endpoint().to_string(),
        target_id: target_id.clone(),
    };
    let rules = Rules {
        scope: config_scope(harness, task)?,
        gate: task.confirm_irreversible,
        authorized: task.authorize_irreversible,
        banners: task.refuse_cookie_banners.unwrap_or(true),
        secrets: result.typed_secrets.clone(),
    };
    let limits = Limits {
        max_steps: fallback_steps(),
        max_tokens: crate::fallback::settings::DEFAULT_TOKENS,
        deadline: tokio::time::Instant::now() + Duration::from_secs(task.timeout_s),
    };
    let brief = crate::fallback::Brief {
        goal: &task.goal,
        trigger: why,
        jev: &result,
    };
    let (outcome, _) =
        crate::fallback::fall_back(&tab, model.as_ref(), brief, rules, limits, None).await;
    engine.close().await.ok();
    let expected = task
        .fallback
        .as_ref()
        .and_then(|fallback| fallback.expect.posts.as_ref())
        .or(task.expect.posts.as_ref())
        .map_or(0, Vec::len);
    let all_posts = harness
        .site
        .wait_for_posts(expected, Duration::from_secs(2))
        .await;
    let combined = Outcome {
        result: after_fallback(&result, &outcome),
        probed: probed.take(),
        posts: all_posts,
        wall_ms: started.elapsed().as_millis() as u64,
        after: None,
    };
    Ok(Outcome {
        result,
        probed: before,
        posts,
        wall_ms,
        after: Some(Box::new((combined, outcome))),
    })
}

/// The operator's allowed origins as the task sets them.
fn config_scope(harness: &Harness, task: &Task) -> anyhow::Result<crate::scope::JevOriginScope> {
    if task.allowed_origins.is_empty() {
        return Ok(crate::scope::JevOriginScope::any());
    }
    let origins = task
        .allowed_origins
        .iter()
        .map(|origin| origin.replace("{site}", harness.site.origin()))
        .collect::<Vec<_>>();
    crate::scope::JevOriginScope::any().narrow(&origins)
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
        if let Some(fallback) = &task.fallback {
            ensure!(
                !fallback.plan.is_empty(),
                "{}: empty fallback plan",
                task.id
            );
            for step in &fallback.plan {
                step.check()
                    .with_context(|| format!("{}: bad fallback step", task.id))?;
            }
            fallback
                .expect
                .check()
                .with_context(|| format!("{}: bad fallback.expect", task.id))?;
        }
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
    fn a_status_is_one_name_or_a_list_of_known_names() {
        let expect = |raw: serde_json::Value| serde_json::from_value::<Expect>(raw).unwrap();
        assert!(
            expect(serde_json::json!({"status": "done"}))
                .check()
                .is_ok()
        );
        assert!(
            expect(serde_json::json!({"status": ["needs_input", "blocked"]}))
                .check()
                .is_ok()
        );
        assert!(
            expect(serde_json::json!({"status": ["blocked", "stuck"]}))
                .check()
                .is_err()
        );
        assert!(expect(serde_json::json!({"status": []})).check().is_err());
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
