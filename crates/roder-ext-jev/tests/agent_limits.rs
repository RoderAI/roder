//! The operator's limits on a run, and the usage it reports: an action
//! ceiling, a time ceiling, an allowed-origins scope, summed token usage,
//! and a step that opened a new tab.

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use roder_ext_jev::{
    JevBilled, JevDecisionTransport, JevEngine, JevEngineConfig, JevOriginScope, JevRunResult,
    JevStatus, JevStop, JevTextValue, JevTextValueResolver, JevTypeSafeDecisionClient,
};
use serde_json::{Value, json};
use support::{ScriptedBrowser, ScriptedDecider, page};

const RUN_TIMEOUT: Duration = Duration::from_secs(5);

async fn run(browser: ScriptedBrowser, config: JevEngineConfig) -> JevRunResult {
    let mut engine = JevEngine::start(Box::new(browser), config).await.unwrap();
    engine.run(RUN_TIMEOUT).await
}

fn config(decider: ScriptedDecider) -> JevEngineConfig {
    JevEngineConfig::new("Scripted goal", Arc::new(decider)).with_wait(Duration::ZERO)
}

fn steps(count: usize) -> Vec<Value> {
    (0..=count)
        .map(|step| page(&format!("step-{step}"), &[("next", "click")]))
        .collect()
}

fn at(url: &str, fingerprint: &str) -> Value {
    let mut observation = page(fingerprint, &[("next", "click")]);
    observation["url"] = json!(url);
    observation
}

#[tokio::test]
async fn an_action_ceiling_replaces_the_default_budget() {
    let browser = ScriptedBrowser::new(steps(5));
    let log = browser.log();
    let result = run(
        browser,
        config(ScriptedDecider::new(&["next"])).with_max_actions(3),
    )
    .await;
    assert_eq!(result.status, JevStatus::BudgetExceeded);
    assert_eq!(
        result.stopped_because.as_deref(),
        Some("Stopped at the 3-action demo budget")
    );
    assert_eq!(log.lock().unwrap().acts.len(), 3);
}

#[tokio::test]
async fn a_time_ceiling_cuts_a_longer_timeout() {
    let browser = ScriptedBrowser::new(steps(200)).with_act_delay(Duration::from_millis(50));
    let started = Instant::now();
    let mut engine = JevEngine::start(
        Box::new(browser),
        config(ScriptedDecider::new(&["next"])).with_max_duration(Duration::from_millis(300)),
    )
    .await
    .unwrap();
    let result = engine.run(Duration::from_secs(30)).await;
    assert_eq!(result.status, JevStatus::TimedOut);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn a_page_outside_the_scope_ends_the_run_blocked_after_the_step() {
    let browser = ScriptedBrowser::new(vec![
        at("https://shop.example.com/", "one"),
        at("https://evil.test/landing", "two"),
        at("https://shop.example.com/", "three"),
    ]);
    let log = browser.log();
    let scope = JevOriginScope::any()
        .narrow(&["https://*.example.com"])
        .unwrap();
    let result = run(
        browser,
        config(ScriptedDecider::new(&["next"])).with_scope(scope),
    )
    .await;
    assert_eq!(result.status, JevStatus::Blocked);
    assert_eq!(
        result.stopped_because.as_deref(),
        Some(
            "The page went outside the allowed origins: https://evil.test is not in \
             https://*.example.com"
        )
    );
    assert_eq!(result.actions.len(), 1);
    assert_eq!(result.actions[0].url, "https://evil.test/landing");
    assert_eq!(result.actions[0].page_changed, Some(true));
    assert_eq!(log.lock().unwrap().acts.len(), 1);
}

#[tokio::test]
async fn a_start_page_outside_the_scope_makes_no_decision() {
    let browser = ScriptedBrowser::new(vec![at("https://evil.test/", "one")]);
    let decider = ScriptedDecider::new(&["next"]);
    let decisions = decider.log();
    let scope = JevOriginScope::any()
        .narrow(&["https://example.com"])
        .unwrap();
    let result = run(browser, config(decider).with_scope(scope)).await;
    assert_eq!(result.status, JevStatus::Blocked);
    assert!(
        result
            .stopped_because
            .as_deref()
            .is_some_and(|reason| reason.contains("https://evil.test is not in")),
        "{:?}",
        result.stopped_because
    );
    assert_eq!(result.model_calls, 0);
    assert!(decisions.lock().unwrap().seen.is_empty());
}

struct Tokens;

#[async_trait]
impl JevTextValueResolver for Tokens {
    async fn resolve(&self, _field: &Value) -> anyhow::Result<JevTextValue> {
        Ok(JevTextValue {
            value: "Ada".into(),
            model: "text".into(),
            latency_ms: 1,
            usage: json!({"prompt_tokens": 20, "completion_tokens": 2}),
        })
    }
}

#[tokio::test]
async fn usage_is_summed_and_unknown_is_never_zero() {
    let pages = vec![
        page("one", &[("name", "fill")]),
        page("two", &[("name", "fill")]),
        page("three", &[]),
    ];
    let decider = ScriptedDecider::new(&["name", "name", "DONE"])
        .with_usage(json!({"input_tokens": 100, "output_tokens": 4}));
    let result = run(
        ScriptedBrowser::new(pages),
        config(decider).with_text_resolver(Arc::new(Tokens)),
    )
    .await;
    assert_eq!(result.status, JevStatus::Done);
    let usage = serde_json::to_value(result.usage).unwrap();
    assert_eq!(
        usage,
        json!({
            "decision": {"calls": 3, "input_tokens": 300, "output_tokens": 12},
            "text": {"calls": 2, "input_tokens": 40, "output_tokens": 4},
        })
    );
    // A decision service that reports nothing is unknown, not free.
    let result = run(
        ScriptedBrowser::new(vec![page("one", &[])]),
        config(ScriptedDecider::new(&["DONE"])),
    )
    .await;
    let usage = serde_json::to_value(&result).unwrap()["usage"].clone();
    assert_eq!(
        usage["decision"],
        json!({"calls": 1, "input_tokens": "unknown", "output_tokens": "unknown"})
    );
    assert_eq!(usage["text"]["input_tokens"], json!(0));
}

#[tokio::test]
async fn a_step_that_opened_a_tab_is_recorded() {
    let mut opened = page("report", &[]);
    opened["opened_tab"] = json!(true);
    let browser = ScriptedBrowser::new(vec![page("list", &[("open", "click")]), opened]);
    let result = run(browser, config(ScriptedDecider::new(&["open", "DONE"]))).await;
    assert_eq!(result.status, JevStatus::Done);
    assert!(result.actions[0].opened_tab);
    let record = serde_json::to_value(&result.actions[0]).unwrap();
    assert_eq!(record["opened_tab"], json!(true));
}

/// A decision service answer that fails validation.
struct InvalidAnswer;

#[async_trait]
impl JevDecisionTransport for InvalidAnswer {
    async fn decide(&self, _request: &Value) -> anyhow::Result<Value> {
        Ok(json!({
            "answers": {"operation": {"choice": "NOT_OFFERED", "confidence": 0.9,
                "probabilities": {"NOT_OFFERED": 1.0}}},
            "usage": {"input_tokens": 50, "output_tokens": 1},
        }))
    }
}

/// A text model that answered, and was billed, without a value.
struct NoValue;

#[async_trait]
impl JevTextValueResolver for NoValue {
    async fn resolve(&self, _field: &Value) -> anyhow::Result<JevTextValue> {
        Err(JevBilled::new(
            json!({"prompt_tokens": 20, "completion_tokens": 2}),
            JevStop::new(JevStatus::NeedsInput, "The goal gives no value").into(),
        )
        .into())
    }
}

/// Calls that were answered but could not be used were billed all the same,
/// so they count in the run's usage; before, only usable ones did.
#[tokio::test]
async fn billed_calls_with_unusable_answers_count_in_the_usage() {
    let decision = JevTypeSafeDecisionClient::with_transport("jev-test", Arc::new(InvalidAnswer));
    let invalid =
        JevEngineConfig::new("Scripted goal", Arc::new(decision)).with_wait(Duration::ZERO);
    let result = run(
        ScriptedBrowser::new(vec![page("one", &[("next", "click")])]),
        invalid,
    )
    .await;
    assert_eq!(result.status, JevStatus::Error);
    assert_eq!(
        result.stopped_because.as_deref(),
        Some("Invalid TypeSafe response; no action executed.")
    );
    assert_eq!(result.model_calls, 1);
    assert_eq!(
        serde_json::to_value(result.usage.decision).unwrap(),
        json!({"calls": 1, "input_tokens": 50, "output_tokens": 1})
    );

    let decider = ScriptedDecider::new(&["name"])
        .with_usage(json!({"input_tokens": 100, "output_tokens": 4}));
    let result = run(
        ScriptedBrowser::new(vec![page("form", &[("name", "fill")])]),
        config(decider).with_text_resolver(Arc::new(NoValue)),
    )
    .await;
    assert_eq!(result.status, JevStatus::NeedsInput);
    assert_eq!(result.text_calls, 1);
    assert_eq!(
        serde_json::to_value(result.usage.text).unwrap(),
        json!({"calls": 1, "input_tokens": 20, "output_tokens": 2})
    );
}

/// An operator's huge action ceiling does not overflow the model-call
/// budget (twice the actions) into a panic.
#[tokio::test]
async fn a_huge_action_ceiling_does_not_overflow() {
    let result = run(
        ScriptedBrowser::new(steps(2)),
        config(ScriptedDecider::new(&["next", "next", "DONE"])).with_max_actions(usize::MAX),
    )
    .await;
    assert_eq!(result.status, JevStatus::Done);
    assert_eq!(result.actions.len(), 2);
}
