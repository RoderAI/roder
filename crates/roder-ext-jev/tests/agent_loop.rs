//! Deterministic tests of the agent loop's current behaviour.
//!
//! These pin what the loop does today, including its rough edges (the stale
//! loop bounded only by the model-call budget), so a later change to any of
//! them shows up here as a deliberate test update. A covered target is
//! recorded as a step that changed nothing, so the stall rule, not the budget,
//! ends a run whose target stays covered. A run that stops early says why
//! with its status: a budget, a timeout, a missing value, an unavailable
//! provider, or an error. Refused actions and dialogs are in
//! `agent_outcomes.rs`.

mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use anyhow::anyhow;
use async_trait::async_trait;
use roder_ext_jev::{
    JevEngine, JevEngineConfig, JevRunResult, JevStatus, JevStop, JevTextValue,
    JevTextValueResolver,
};
use serde_json::{Value, json};
use support::{ScriptedBrowser, ScriptedDecider, page};

const RUN_TIMEOUT: Duration = Duration::from_secs(5);

async fn run(browser: ScriptedBrowser, config: JevEngineConfig, timeout: Duration) -> JevRunResult {
    let mut engine = JevEngine::start(Box::new(browser), config).await.unwrap();
    engine.run(timeout).await
}

fn config(decider: ScriptedDecider) -> JevEngineConfig {
    JevEngineConfig::new("Scripted goal", Arc::new(decider)).with_wait(Duration::ZERO)
}

#[tokio::test]
async fn a_stale_done_reobserves_and_decides_again() {
    let browser = ScriptedBrowser::new(vec![
        page("one", &[("next", "click")]),
        page("two", &[]),
        page("three", &[]),
    ])
    // predict, predict, the DONE check (stale), predict, the DONE check.
    .with_fresh(&[true, true, false, true, true]);
    let browser_log = browser.log();
    let decider = ScriptedDecider::new(&["next", "DONE", "DONE"]);
    let decider_log = decider.log();

    let result = run(browser, config(decider), RUN_TIMEOUT).await;

    assert_eq!(result.status, JevStatus::Done);
    assert_eq!(result.stopped_because, None);
    assert_eq!(result.model_calls, 3);
    assert_eq!(result.actions.len(), 1);
    assert_eq!(result.actions[0].page_changed, Some(true));
    let browser_log = browser_log.lock().unwrap();
    assert_eq!(browser_log.observes, 3);
    assert_eq!(browser_log.acts.len(), 1);
    assert_eq!(browser_log.fresh_calls, vec![None; 5]);
    // The second DONE was chosen on the page observed after the stale one.
    assert_eq!(
        decider_log.lock().unwrap().seen,
        vec![json!("one"), json!("two"), json!("three")]
    );
    assert_eq!(result.visible_text, "page three");
}

#[tokio::test]
async fn three_unchanged_fingerprints_block_the_run() {
    let browser = ScriptedBrowser::new(vec![page("same", &[("button", "click")])]);
    let browser_log = browser.log();

    let result = run(
        browser,
        config(ScriptedDecider::new(&["button"])),
        RUN_TIMEOUT,
    )
    .await;

    assert_eq!(result.status, JevStatus::Blocked);
    assert_eq!(result.stopped_because, None);
    assert_eq!(result.model_calls, 3);
    assert_eq!(result.actions.len(), 3);
    assert!(
        result
            .actions
            .iter()
            .all(|action| action.page_changed == Some(false))
    );
    assert_eq!(browser_log.lock().unwrap().acts.len(), 3);
}

#[tokio::test]
async fn an_always_missing_choice_loops_until_the_model_call_budget() {
    // Nothing but the 120-call budget bounds this loop today.
    let browser = ScriptedBrowser::new(vec![page("page", &[("real", "click")])]);
    let browser_log = browser.log();

    let result = run(
        browser,
        config(ScriptedDecider::new(&["ghost"])),
        RUN_TIMEOUT,
    )
    .await;

    assert_eq!(result.status, JevStatus::BudgetExceeded);
    assert_eq!(
        result.stopped_because.as_deref(),
        Some("Reached the demo's model-call budget")
    );
    assert_eq!(result.model_calls, 120);
    assert!(result.actions.is_empty());
    let browser_log = browser_log.lock().unwrap();
    assert!(browser_log.acts.is_empty());
    // The first observation plus one re-observe per stale choice.
    assert_eq!(browser_log.observes, 121);
}

#[tokio::test]
async fn the_sixty_first_action_exceeds_the_action_budget() {
    // A fresh fingerprint every step keeps the stall rule out of the way.
    let pages = (0..=61)
        .map(|step| page(&format!("step-{step}"), &[("next", "click")]))
        .collect::<Vec<_>>();
    let browser = ScriptedBrowser::new(pages);
    let browser_log = browser.log();

    let result = run(
        browser,
        config(ScriptedDecider::new(&["next"])),
        RUN_TIMEOUT,
    )
    .await;

    // Upstream raises here, and Jev used to report it as blocked.
    assert_eq!(result.status, JevStatus::BudgetExceeded);
    assert_eq!(
        result.stopped_because.as_deref(),
        Some("Stopped at the 60-action demo budget")
    );
    assert_eq!(result.model_calls, 61);
    assert_eq!(result.actions.len(), 60);
    assert_eq!(result.actions.last().unwrap().step, 60);
    assert_eq!(browser_log.lock().unwrap().acts.len(), 60);
}

#[tokio::test]
async fn a_fill_without_a_text_resolver_needs_input_and_never_acts() {
    let browser = ScriptedBrowser::new(vec![page("form", &[("email", "fill")])]);
    let browser_log = browser.log();

    let result = run(
        browser,
        config(ScriptedDecider::new(&["email"])),
        RUN_TIMEOUT,
    )
    .await;

    assert_eq!(result.status, JevStatus::NeedsInput);
    // It names the field it had no value for.
    assert!(
        result
            .stopped_because
            .as_deref()
            .unwrap()
            .starts_with("TYPE_TEXT into \"Label email\" needs a text model"),
        "{:?}",
        result.stopped_because
    );
    assert_eq!(result.model_calls, 1);
    assert_eq!(result.text_calls, 0);
    assert!(result.actions.is_empty());
    let browser_log = browser_log.lock().unwrap();
    assert!(browser_log.acts.is_empty());
    // The field itself was checked for freshness before giving up.
    assert_eq!(browser_log.fresh_calls, vec![None, Some("email".into())]);
}

struct FailingResolver;

#[async_trait]
impl JevTextValueResolver for FailingResolver {
    async fn resolve(&self, _field_context: &Value) -> anyhow::Result<JevTextValue> {
        Err(anyhow!("resolver unavailable"))
    }
}

#[tokio::test]
async fn a_resolver_error_ends_the_run_without_acting() {
    let browser = ScriptedBrowser::new(vec![page("form", &[("email", "fill")])]);
    let browser_log = browser.log();
    let config =
        config(ScriptedDecider::new(&["email"])).with_text_resolver(Arc::new(FailingResolver));

    let result = run(browser, config, RUN_TIMEOUT).await;

    // A plain error, not a typed stop, ends the run as `error`.
    assert_eq!(result.status, JevStatus::Error);
    assert_eq!(
        result.stopped_because.as_deref(),
        Some("resolver unavailable")
    );
    assert_eq!(result.model_calls, 1);
    assert_eq!(result.text_calls, 0);
    assert!(result.actions.is_empty());
    assert!(browser_log.lock().unwrap().acts.is_empty());
}

#[tokio::test]
async fn a_slow_act_times_out_without_recording_the_action() {
    let browser = ScriptedBrowser::new(vec![page("slow", &[("button", "click")])])
        .with_act_delay(Duration::from_secs(10));
    let browser_log = browser.log();
    let started = Instant::now();

    let result = run(
        browser,
        config(ScriptedDecider::new(&["button"])),
        Duration::from_millis(50),
    )
    .await;

    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(result.status, JevStatus::TimedOut);
    assert_eq!(
        result.stopped_because.as_deref(),
        Some("Jev browser task timed out")
    );
    assert_eq!(result.model_calls, 1);
    // The act started but was cancelled, so the trace has no action; the
    // elapsed time runs to the timeout.
    assert_eq!(browser_log.lock().unwrap().acts.len(), 1);
    assert!(result.actions.is_empty());
    assert!(result.elapsed_ms >= 50, "{}", result.elapsed_ms);
}

#[tokio::test]
async fn a_target_that_stays_covered_blocks_after_three_tries() {
    let browser =
        ScriptedBrowser::new(vec![page("modal", &[("buy", "click")])]).with_covered(&[true; 10]);
    let browser_log = browser.log();

    let result = run(browser, config(ScriptedDecider::new(&["buy"])), RUN_TIMEOUT).await;

    assert_eq!(result.status, JevStatus::Blocked);
    assert_eq!(result.stopped_because, None);
    // Before, each covered try was a stale page: no history entry, and a new
    // decision every time until the 120-call budget.
    assert_eq!(result.model_calls, 3);
    assert_eq!(result.actions.len(), 3);
    assert!(
        result
            .actions
            .iter()
            .all(|action| action.covered && action.page_changed == Some(false))
    );
    let browser_log = browser_log.lock().unwrap();
    assert_eq!(browser_log.acts.len(), 3);
    // The first observation plus one after each covered try.
    assert_eq!(browser_log.observes, 4);
}

#[tokio::test]
async fn done_after_a_covered_try_is_blocked_not_done() {
    let browser =
        ScriptedBrowser::new(vec![page("modal", &[("buy", "click")])]).with_covered(&[true]);
    let browser_log = browser.log();

    let result = run(
        browser,
        config(ScriptedDecider::new(&["buy", "DONE"])),
        RUN_TIMEOUT,
    )
    .await;

    // The hosted model answered DONE here at 0.67 confidence: the click it
    // asked for was never dispatched, so the goal cannot have been met.
    assert_eq!(result.status, JevStatus::Blocked);
    assert_eq!(result.model_calls, 2);
    assert_eq!(result.actions.len(), 1);
    assert!(result.actions[0].covered);
    assert_eq!(browser_log.lock().unwrap().acts.len(), 1);
}

#[tokio::test]
async fn an_unsure_repeat_of_a_click_that_worked_ends_done_without_clicking() {
    let pages = vec![
        page("shop", &[("add", "click")]),
        page("shop-with-cart", &[("add", "click")]),
    ];
    let browser = ScriptedBrowser::new(pages);
    let browser_log = browser.log();
    let decider = ScriptedDecider::new(&["add", "add"]).with_confidences(&[1.0, 0.5]);

    let result = run(browser, config(decider), RUN_TIMEOUT).await;

    assert_eq!(result.status, JevStatus::Done);
    assert_eq!(result.stopped_because, None);
    assert_eq!(result.model_calls, 2);
    assert_eq!(result.actions.len(), 1);
    assert_eq!(browser_log.lock().unwrap().acts.len(), 1);
}

#[tokio::test]
async fn a_confident_repeat_of_a_click_that_worked_clicks_again() {
    let pages = vec![
        page("count-0", &[("plus", "click")]),
        page("count-1", &[("plus", "click")]),
        page("count-2", &[("plus", "click")]),
    ];
    let browser = ScriptedBrowser::new(pages);
    let browser_log = browser.log();
    let decider =
        ScriptedDecider::new(&["plus", "plus", "DONE"]).with_confidences(&[1.0, 0.8, 0.5]);

    let result = run(browser, config(decider), RUN_TIMEOUT).await;

    assert_eq!(result.status, JevStatus::Done);
    assert_eq!(result.actions.len(), 2);
    assert_eq!(browser_log.lock().unwrap().acts.len(), 2);
}

struct CountingResolver(AtomicUsize);

#[async_trait]
impl JevTextValueResolver for CountingResolver {
    async fn resolve(&self, _field_context: &Value) -> anyhow::Result<JevTextValue> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(JevTextValue {
            value: "ada@example.com".into(),
            model: "counting".into(),
            latency_ms: 0,
            usage: json!({}),
        })
    }
}

#[tokio::test]
async fn a_covered_fill_is_recorded_as_typing_nothing_and_then_retried() {
    let browser = ScriptedBrowser::new(vec![
        page("form", &[("email", "fill")]),
        // Scrolled to the field before finding it covered: a new fingerprint
        // that must still not count as the fill taking effect.
        page("form-scrolled", &[("email", "fill")]),
        page("sent", &[]),
    ])
    .with_covered(&[true]);
    let browser_log = browser.log();
    let resolver = Arc::new(CountingResolver(AtomicUsize::new(0)));
    let config = config(ScriptedDecider::new(&["email", "email", "DONE"]))
        .with_text_resolver(resolver.clone());

    let result = run(browser, config, RUN_TIMEOUT).await;

    assert_eq!(result.status, JevStatus::Done);
    assert_eq!(result.model_calls, 3);
    let steps = result
        .actions
        .iter()
        .map(|action| (action.covered, action.text.as_deref(), action.page_changed))
        .collect::<Vec<_>>();
    assert_eq!(
        steps,
        vec![
            (true, None, Some(false)),
            (false, Some("ada@example.com"), Some(true)),
        ]
    );
    // The covered attempt's value is not reused: the field is asked again.
    assert_eq!(result.text_calls, 2);
    assert_eq!(resolver.0.load(Ordering::SeqCst), 2);
    assert_eq!(browser_log.lock().unwrap().acts.len(), 2);
}

/// The text helper's `{"text": null}`: the goal holds no value for the field.
struct MissingValueResolver;

#[async_trait]
impl JevTextValueResolver for MissingValueResolver {
    async fn resolve(&self, _field_context: &Value) -> anyhow::Result<JevTextValue> {
        Err(JevStop::new(JevStatus::NeedsInput, "no value for Email").into())
    }
}

#[tokio::test]
async fn a_value_the_goal_lacks_needs_input_without_acting() {
    let browser = ScriptedBrowser::new(vec![page("form", &[("email", "fill")])]);
    let browser_log = browser.log();
    let config =
        config(ScriptedDecider::new(&["email"])).with_text_resolver(Arc::new(MissingValueResolver));

    let result = run(browser, config, RUN_TIMEOUT).await;

    assert_eq!(result.status, JevStatus::NeedsInput);
    assert_eq!(
        result.stopped_because.as_deref(),
        Some("no value for Email")
    );
    assert!(result.actions.is_empty());
    assert!(browser_log.lock().unwrap().acts.is_empty());
}

#[tokio::test]
async fn an_unavailable_provider_ends_the_run_and_keeps_the_trace() {
    let browser = ScriptedBrowser::new(vec![
        page("one", &[("next", "click")]),
        page("two", &[("next", "click")]),
    ]);
    let browser_log = browser.log();
    let decider = ScriptedDecider::new(&["next"]).failing_after(1, Some(JevStatus::Unavailable));

    let result = run(browser, config(decider), RUN_TIMEOUT).await;

    assert_eq!(result.status, JevStatus::Unavailable);
    assert_eq!(result.stopped_because.as_deref(), Some("scripted stop"));
    // The step before the failure is still reported, on the page it reached.
    assert_eq!(result.model_calls, 1);
    assert_eq!(result.actions.len(), 1);
    assert_eq!(result.visible_text, "page two");
    assert_eq!(browser_log.lock().unwrap().acts.len(), 1);
}

#[tokio::test]
async fn an_untyped_decision_failure_is_an_error() {
    let browser = ScriptedBrowser::new(vec![page("page", &[("button", "click")])]);
    let decider = ScriptedDecider::new(&["button"]).failing_after(0, None);

    let result = run(browser, config(decider), RUN_TIMEOUT).await;

    assert_eq!(result.status, JevStatus::Error);
    assert_eq!(result.stopped_because.as_deref(), Some("scripted failure"));
    assert_eq!(result.model_calls, 0);
}

#[tokio::test]
async fn a_stop_cannot_claim_the_run_is_done() {
    // A hosted client that stops with `done` has not finished the goal.
    let browser = ScriptedBrowser::new(vec![page("page", &[("button", "click")])]);
    let decider = ScriptedDecider::new(&["button"]).failing_after(0, Some(JevStatus::Done));

    let result = run(browser, config(decider), RUN_TIMEOUT).await;

    assert_eq!(result.status, JevStatus::Error);
}

#[tokio::test]
async fn statuses_serialize_in_snake_case() {
    let browser = ScriptedBrowser::new(vec![page("form", &[("email", "fill")])]);
    let result = run(
        browser,
        config(ScriptedDecider::new(&["email"])),
        RUN_TIMEOUT,
    )
    .await;
    let value = serde_json::to_value(&result).unwrap();
    assert_eq!(value["status"], json!("needs_input"));
}
