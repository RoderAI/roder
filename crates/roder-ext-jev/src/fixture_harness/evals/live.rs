//! The live tier: the hosted decision model on the same pages and graders.
//!
//! Opt-in and `#[ignore]`d; it needs a key and spends model calls:
//!
//! ```text
//! TYPESAFE_API_KEY=… cargo test -p roder-ext-jev live_corpus -- --ignored --nocapture
//! ```
//!
//! - `TYPESAFE_API_KEY` (or `JEV_API_KEY`) is the decision key; `JEV_MODEL`
//!   pins a model, else `jev-latest`.
//! - `JEV_EVAL_VARIANTS=a,b` turns on request variants (see [`super::variants`]).
//! - `JEV_EVAL_TEXT=model` types values from the configured text helper
//!   (`JEV_TEXT_MODEL` and `JEV_TEXT_MODEL_REASONING` pick it, as for
//!   `jev_browse`) instead of each task's `values`, which otherwise isolate
//!   the decision model. A password or one-time code the task holds still
//!   comes from its values, as a supervisor's resolver would supply it; see
//!   [`super::text_sources::Supervised`].
//! - `JEV_EVAL_CONFIRM_IRREVERSIBLE=1` runs every task with the
//!   irreversible-action gate on, to measure what turning it on by default
//!   would cost, and `JEV_EVAL_REFUSE_COOKIE_BANNERS=0` every task that does
//!   not set it with cookie-banner refusal off, to measure what the default
//!   costs; tasks that set either in `tasks.json` always run as they say.
//! - `JEV_EVAL_TASKS=id,id` narrows the run; `JEV_EVAL_CONCURRENCY` (default
//!   2) sets how many runs go at once.
//! - `JEV_EVAL_N=3` runs each task that many times (default 1, at most 20),
//!   all tasks once before any twice, and prints a tally per task.
//! - `JEV_EVAL_STRICT=1` fails the test on a mid-run edit of the tree (see
//!   [`super::pin`]), on any false green, and then on any failed run, or,
//!   when a baseline exists, on any task whose rates got worse than its.
//! - The baseline is `tests/fixtures/evals/live-baseline.json`, or the file
//!   `JEV_EVAL_BASELINE` names. `JEV_EVAL_SAVE_BASELINE=1` writes this run's
//!   tally and pin there, if the run was `JEV_EVAL_N=3` or more over the whole
//!   corpus and the tree did not change under it (see [`super::tally`]).
//!   With `JEV_EVAL_STRICT=1` as well, a run the strict gate rejects is not
//!   saved: record a baseline that holds known failures (the first one, or a
//!   regression accepted on purpose) without `JEV_EVAL_STRICT`.
//!
//! Each run writes `target/jev-evals/live-<unix seconds>.jsonl`, one line per
//! run of a task, graded on outcomes only (the plan-specific `script` checks
//! do not apply to a model), plus per-step telemetry, and beside it
//! `live-<unix seconds>.summary.json`: the pin at the start and at the end,
//! the tally and the comparison with the baseline.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use futures::StreamExt;
use serde_json::{Value, json};

use super::pin::Pin;
use super::tally::{self, Baseline, Run, Settings};
use super::text_sources::{Recorded, Supervised};
use super::variants::{StepTelemetry, VariantTransport, Variants};
use super::{
    FallbackRow, Outcome, Row, Task, TaskValues, load_tasks, run_task, table, validate, write_json,
    write_rows,
};
use crate::decide::{ENDPOINT, JevTypeSafeDecisionClient, TypeSafeHttpTransport};
use crate::engine::{JevDecisionTransport, JevTextValueResolver};
use crate::fallback::model::FallbackModel;
use crate::fixture_harness::Harness;
use crate::http::RetryPolicy;
use crate::text_helper::TextHelper;
use crate::text_model::TextModel;

pub(super) fn env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

pub(super) fn live_key() -> Option<String> {
    env("TYPESAFE_API_KEY").or_else(|| env("JEV_API_KEY"))
}

struct LiveSetup {
    key: String,
    model: String,
    variants: Variants,
    /// `JEV_EVAL_TEXT=model`: the text helper that types ordinary fields.
    text_model: Option<TextModel>,
    /// Every task with the irreversible-action gate on.
    confirm_irreversible: bool,
    /// Every task that does not set it with cookie-banner refusal off.
    no_cookie_banner_refusal: bool,
    /// `JEV_EVAL_FALLBACK=model`: the fallback after every task Jev could
    /// not finish, each row graded on Jev alone and on Jev with it.
    fallback: Option<Arc<dyn FallbackModel>>,
    /// `JEV_EVAL_CONCURRENCY`: how many runs go at once. It sets the request
    /// load on the hosted service and the load on the machine's Chrome, so
    /// runs at different values are not the same measurement.
    concurrency: usize,
}

impl LiveSetup {
    /// The switches that change what the run measures, for its pin and its
    /// header.
    fn label(&self) -> String {
        format!(
            "text {}, fallback {}, variants {:?}, gate {}, cookie refusal {}, concurrency {}",
            self.text_model
                .as_ref()
                .map_or("task values".to_string(), TextModel::label),
            self.fallback
                .as_ref()
                .map_or("off".to_string(), |model| model.label()),
            self.variants.names(),
            self.confirm_irreversible,
            !self.no_cookie_banner_refusal,
            self.concurrency,
        )
    }

    /// The task as this run plays it, and the names of the switches it
    /// forced on, for the row.
    fn task(&self, task: &Task) -> (Task, Vec<String>) {
        let mut task = task.clone();
        let mut names = self.variants.names();
        if self.confirm_irreversible {
            task.confirm_irreversible = true;
            names.push("confirm_irreversible".into());
        }
        if self.no_cookie_banner_refusal && task.refuse_cookie_banners.is_none() {
            task.refuse_cookie_banners = Some(false);
            names.push("no_cookie_banner_refusal".into());
        }
        (task, names)
    }
}

/// The row of a run that could not start a browser: a failed run in the
/// tally, not a run that is missing from it.
fn no_chrome(task: &Task, repeat: usize) -> Row {
    let error = anyhow::anyhow!("no Chrome binary was found (set JEV_CHROME_BINARY)");
    let mut row = Row::errored(task, "live", &error);
    row.repeat = repeat;
    row
}

async fn run_one(setup: &LiveSetup, task: &Task, repeat: usize) -> Row {
    let Some(harness) = Harness::start().await else {
        return no_chrome(task, repeat);
    };
    let (task, names) = setup.task(task);
    let task = &task;
    let http: Arc<dyn JevDecisionTransport> = Arc::new(TypeSafeHttpTransport::new(
        ENDPOINT,
        setup.key.clone(),
        RetryPolicy::default(),
    ));
    let transport = Arc::new(VariantTransport::new(http, setup.variants.clone()));
    let decision: Arc<dyn crate::engine::JevDecisionClient> =
        if setup.variants.has(super::variants::Variant::Effect) {
            super::effect_variant::client(setup.model.clone(), transport.clone())
        } else {
            Arc::new(JevTypeSafeDecisionClient::with_transport(
                setup.model.clone(),
                transport.clone(),
            ))
        };
    let helper = setup
        .text_model
        .as_ref()
        .map(|model| Arc::new(TextHelper::new(model.clone())));
    let text = match &helper {
        Some(helper) => Recorded::new(
            Arc::new(Supervised::new(&task.values, helper.clone())),
            "text-model",
        ),
        None => Recorded::new(Arc::new(TaskValues::new(&task.values)), "task-values"),
    };
    let resolver: Arc<dyn JevTextValueResolver> = text.clone();
    let probes = task.expect.probes().cloned().collect();
    let mut row = match run_task(
        &harness,
        task,
        decision,
        resolver,
        probes,
        setup.fallback.clone(),
    )
    .await
    {
        Ok(outcome) => {
            let graded = task.expect.grade(&outcome);
            let mut row = Row::new(task, "live", names, &outcome, graded);
            if setup.fallback.is_some() {
                row.fallback = Some(FallbackRow::grade(task, &outcome, row.marks));
            }
            row.telemetry = telemetry(&outcome, &transport.steps());
            // The model that wrote values, which is not the one
            // resolved when an unusable Codex sign-in fell back.
            row.telemetry["text_model"] = json!(
                helper
                    .as_ref()
                    .map_or("task-values".to_string(), |helper| helper.current().label())
            );
            row.telemetry["text"] = text.report();
            row
        }
        Err(error) => Row::errored(task, "live", &error),
    };
    row.repeat = repeat;
    row
}

/// Per-head confidence and the call confidence (the least certain head),
/// the answering model, request size, and shadow answers, per decision.
fn telemetry(outcome: &Outcome, steps: &[StepTelemetry]) -> Value {
    let decisions = &outcome.result.decisions;
    let call =
        |confidence: f64, target: Option<f64>| target.map_or(confidence, |t| t.min(confidence));
    let calls = decisions
        .iter()
        .map(|decision| call(decision.confidence, decision.target_confidence))
        .collect::<Vec<_>>();
    let input_tokens = decisions
        .iter()
        .filter_map(|decision| decision.usage["input_tokens"].as_u64())
        .sum::<u64>();
    let per_step = decisions
        .iter()
        .enumerate()
        .map(|(index, decision)| {
            let mut step = json!({
                "operation": decision.operation,
                "choice": decision.choice,
                "confidence": decision.confidence,
                "target_confidence": decision.target_confidence,
                "latency_ms": decision.latency_ms,
                "irreversible": decision.irreversible,
            });
            if let Some(sent) = steps.get(index) {
                step["request"] = serde_json::to_value(sent).unwrap_or(Value::Null);
            }
            step
        })
        .collect::<Vec<_>>();
    json!({
        "model": decisions.iter().rev().find_map(|decision| decision.model.clone()),
        "min_call_confidence": calls.iter().copied().reduce(f64::min),
        "mean_call_confidence": (!calls.is_empty())
            .then(|| calls.iter().sum::<f64>() / calls.len() as f64),
        "input_tokens": input_tokens,
        "max_request_bytes": steps.iter().map(|step| step.request_bytes).max(),
        "none_wins": steps.iter().filter(|step| step.none_won).count(),
        "decisions": per_step,
        "effects": outcome.result.actions.iter().map(|action| action.effect.clone()).collect::<Vec<_>>(),
    })
}

#[tokio::test]
#[ignore = "live: needs TYPESAFE_API_KEY (or JEV_API_KEY), Chrome, and spends model calls"]
async fn live_corpus() {
    let tasks = load_tasks().unwrap();
    validate(&tasks).unwrap();
    let Some(key) = live_key() else {
        eprintln!("skipping: set TYPESAFE_API_KEY (or JEV_API_KEY) to run the live tier");
        return;
    };
    let concurrency = env("JEV_EVAL_CONCURRENCY")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(2)
        .max(1);
    let setup = LiveSetup {
        key,
        model: env("JEV_MODEL").unwrap_or_else(|| "jev-latest".into()),
        variants: Variants::parse(&env("JEV_EVAL_VARIANTS").unwrap_or_default()).unwrap(),
        text_model: match env("JEV_EVAL_TEXT").as_deref() == Some("model") {
            true => Some(
                crate::runner::resolve_text_model(None)
                    .await
                    .unwrap()
                    .expect("JEV_EVAL_TEXT=model but no text model is configured"),
            ),
            false => None,
        },
        confirm_irreversible: crate::runner::switch(
            "JEV_EVAL_CONFIRM_IRREVERSIBLE",
            env("JEV_EVAL_CONFIRM_IRREVERSIBLE").as_deref(),
            false,
        )
        .unwrap(),
        no_cookie_banner_refusal: !crate::runner::switch(
            "JEV_EVAL_REFUSE_COOKIE_BANNERS",
            env("JEV_EVAL_REFUSE_COOKIE_BANNERS").as_deref(),
            true,
        )
        .unwrap(),
        fallback: super::fallback_live::live_fallback().await,
        concurrency,
    };
    if Harness::start().await.is_none() {
        eprintln!("skipping: no Chrome binary found (set JEV_CHROME_BINARY)");
        return;
    }
    let repeats = tally::repeats(env("JEV_EVAL_N").as_deref()).unwrap();
    let settings = Settings {
        n: repeats,
        subset: env("JEV_EVAL_TASKS").is_some(),
        save: crate::runner::switch(
            "JEV_EVAL_SAVE_BASELINE",
            env("JEV_EVAL_SAVE_BASELINE").as_deref(),
            false,
        )
        .unwrap(),
        strict: env("JEV_EVAL_STRICT").as_deref() == Some("1"),
    };
    let baseline_path = tally::baseline_path(env("JEV_EVAL_BASELINE").as_deref());
    let saved = Baseline::load(&baseline_path).unwrap();
    let start = Pin::capture(&setup.model, &setup.label()).unwrap();
    let setup = &setup;
    let rows = futures::stream::iter(tally::plan(&tasks, repeats))
        .map(|(task, repeat)| run_one(setup, task, repeat))
        .buffer_unordered(setup.concurrency)
        .collect::<Vec<_>>()
        .await;
    let end = Pin::capture(&setup.model, &setup.label()).unwrap();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let path = write_rows(&format!("live-{stamp}"), &rows).unwrap();
    let runs = rows.iter().map(Run::from).collect::<Vec<_>>();
    let conclusion = tally::conclude(&runs, start, end, &settings, saved.as_ref());
    let summary = write_json(&format!("live-{stamp}.summary"), &conclusion).unwrap();
    if let Some(baseline) = &conclusion.save {
        baseline.save(&baseline_path).unwrap();
        eprintln!("baseline saved: {}", baseline_path.display());
    }
    eprintln!(
        "model {}, {}\n{}{}rows: {}\nsummary: {}",
        setup.model,
        setup.label(),
        table(&tasks, &rows),
        conclusion.report(),
        path.display(),
        summary.display()
    );
    if settings.strict {
        let failures = conclusion.strict_failures();
        assert!(
            failures.is_empty(),
            "the live run did not hold:\n{}",
            failures.join("\n")
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture_harness::evals::pin::Pin;
    use crate::fixture_harness::evals::tally::conclude;

    fn setup(concurrency: usize) -> LiveSetup {
        LiveSetup {
            key: "test-key".into(),
            model: "jev-latest".into(),
            variants: Variants::parse("").unwrap(),
            text_model: None,
            confirm_irreversible: false,
            no_cookie_banner_refusal: false,
            fallback: None,
            concurrency,
        }
    }

    #[test]
    fn the_pinned_setup_names_how_many_runs_go_at_once() {
        // Runs at another concurrency are noted by the baseline comparison
        // as a different setup, not compared as if nothing differed.
        assert_ne!(setup(2).label(), setup(8).label());
        assert!(setup(8).label().ends_with("concurrency 8"));
    }

    #[test]
    fn a_run_that_could_not_start_a_browser_is_a_failed_run_in_the_tally() {
        let task: Task = serde_json::from_value(json!({
            "id": "any_task",
            "covers": "a task",
            "page": "basic.html",
            "goal": "Do it.",
            "expect": {},
            "script": {"plan": [{"done": true}]}
        }))
        .unwrap();
        let row = no_chrome(&task, 3);
        assert_eq!(row.repeat, 3);
        assert!(!row.passed() && !row.marks.false_green, "{row:?}");
        assert!(row.failures[0].contains("no Chrome"), "{:?}", row.failures);
        let pin = || Pin {
            commit: "c0ffee".into(),
            worktree: "aaaa".into(),
            corpus: "bbbb".into(),
            model: "jev-latest".into(),
            setup: setup(2).label(),
        };
        let settings = Settings {
            n: 1,
            subset: false,
            save: false,
            strict: true,
        };
        let conclusion = conclude(&[Run::from(&row)], pin(), pin(), &settings, None);
        assert_eq!(conclusion.tasks["any_task"].runs, 1);
        assert_eq!(conclusion.strict_failures(), ["1 failed runs"]);
    }
}
