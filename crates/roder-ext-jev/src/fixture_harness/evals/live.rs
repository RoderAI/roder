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
//!   2) sets how many tasks run at once; `JEV_EVAL_STRICT=1` fails the test
//!   on any failed task instead of only reporting it.
//!
//! Each run writes `target/jev-evals/live-<unix seconds>.jsonl`, one line per
//! task, graded on outcomes only (the plan-specific `script` checks do not
//! apply to a model), plus per-step telemetry.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use futures::StreamExt;
use serde_json::{Value, json};

use super::text_sources::{Recorded, Supervised};
use super::variants::{StepTelemetry, VariantTransport, Variants};
use super::{
    FallbackRow, Outcome, Row, Task, TaskValues, load_tasks, run_task, table, validate, write_rows,
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
}

impl LiveSetup {
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

async fn run_one(setup: &LiveSetup, task: &Task) -> Option<Row> {
    let harness = Harness::start().await?;
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
    Some(
        match run_task(
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
                let failures = task.expect.grade(&outcome);
                let mut row = Row::new(task, "live", names, &outcome, failures);
                if setup.fallback.is_some() {
                    row.fallback = Some(FallbackRow::grade(task, &outcome, row.pass));
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
        },
    )
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
    };
    if Harness::start().await.is_none() {
        eprintln!("skipping: no Chrome binary found (set JEV_CHROME_BINARY)");
        return;
    }
    let concurrency = env("JEV_EVAL_CONCURRENCY")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(2)
        .max(1);
    let setup = &setup;
    let rows = futures::stream::iter(&tasks)
        .map(|task| run_one(setup, task))
        .buffer_unordered(concurrency)
        .filter_map(|row| async move { row })
        .collect::<Vec<_>>()
        .await;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let path = write_rows(&format!("live-{stamp}"), &rows).unwrap();
    eprintln!(
        "model {}, text {}, fallback {}, variants {:?}, gate {}, cookie refusal {}\n{}rows: {}",
        setup.model,
        setup
            .text_model
            .as_ref()
            .map_or("task values".to_string(), TextModel::label),
        setup
            .fallback
            .as_ref()
            .map_or("off".to_string(), |model| model.label()),
        setup.variants.names(),
        setup.confirm_irreversible,
        !setup.no_cookie_banner_refusal,
        table(&tasks, &rows),
        path.display()
    );
    if env("JEV_EVAL_STRICT").as_deref() == Some("1") {
        assert!(rows.iter().all(|row| row.pass), "some live tasks failed");
    }
}
