//! The operator's limits on a run, and the usage it reports: an action
//! ceiling, a time ceiling, an allowed-origins scope, summed token usage,
//! and a step that opened a new tab.

mod support;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use roder_ext_jev::{
    JevBilled, JevDecision, JevDecisionClient, JevDecisionTransport, JevEngine, JevEngineConfig,
    JevOriginScope, JevRunResult, JevStatus, JevStop, JevStopCause, JevTextValue,
    JevTextValueResolver, JevTypeSafeDecisionClient,
};
use serde_json::{Map, Value, json};
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

/// A decision service whose replies are scripted: `true` is a reply that
/// passes validation (it picks the first target of the operation named, when
/// the page has one for it), `false` one that does not. Every reply reports
/// the same usage, and the last one scripted repeats.
struct Replies {
    script: Mutex<VecDeque<(bool, &'static str)>>,
    last: (bool, &'static str),
    calls: Arc<Mutex<usize>>,
}

impl Replies {
    fn new(script: &[(bool, &'static str)]) -> Self {
        Self {
            script: Mutex::new(script.iter().copied().collect()),
            last: *script.last().expect("at least one reply"),
            calls: Arc::default(),
        }
    }

    fn calls(&self) -> Arc<Mutex<usize>> {
        Arc::clone(&self.calls)
    }

    fn config(self) -> JevEngineConfig {
        let decision = JevTypeSafeDecisionClient::with_transport("jev-test", Arc::new(self));
        JevEngineConfig::new("Scripted goal", Arc::new(decision)).with_wait(Duration::ZERO)
    }
}

/// An answer choosing `choice` among `ids`, certain of it.
fn answer(choice: &str, ids: &Map<String, Value>) -> Value {
    let probabilities = ids
        .keys()
        .map(|id| (id.clone(), json!(f64::from(id == choice))))
        .collect::<Map<_, _>>();
    json!({"choice": choice, "confidence": 0.9, "probabilities": probabilities})
}

#[async_trait]
impl JevDecisionTransport for Replies {
    async fn decide(&self, request: &Value) -> anyhow::Result<Value> {
        *self.calls.lock().unwrap() += 1;
        let (valid, operation) = self.script.lock().unwrap().pop_front().unwrap_or(self.last);
        let usage = json!({"input_tokens": 50, "output_tokens": 1});
        let questions = &request["questions"];
        let operations = questions["operation"]["criteria"].as_object().unwrap();
        if !valid {
            // An operation the page never offered.
            return Ok(json!({
                "answers": {"operation": {"choice": "NOT_OFFERED", "confidence": 0.9,
                    "probabilities": {"NOT_OFFERED": 1.0}}},
                "usage": usage,
            }));
        }
        let mut answers = Map::new();
        answers.insert("operation".into(), answer(operation, operations));
        if let Some(targets) =
            questions[format!("{}_target", operation.to_lowercase())]["criteria"].as_object()
        {
            let first = targets.keys().next().unwrap();
            answers.insert(
                format!("{}_target", operation.to_lowercase()),
                answer(first, targets),
            );
        }
        Ok(json!({"answers": answers, "usage": usage}))
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
/// so they count in the run's usage; before, only usable ones did. A decision
/// that stays unusable is asked again twice, and every call is counted.
#[tokio::test]
async fn billed_calls_with_unusable_answers_count_in_the_usage() {
    let replies = Replies::new(&[(false, "")]);
    let calls = replies.calls();
    let result = run(
        ScriptedBrowser::new(vec![page("one", &[("next", "click")])]),
        replies.config(),
    )
    .await;
    assert_eq!(result.status, JevStatus::Error);
    assert_eq!(*calls.lock().unwrap(), 3);
    assert_eq!(result.model_calls, 3);
    assert_eq!(
        serde_json::to_value(result.usage.decision).unwrap(),
        json!({"calls": 3, "input_tokens": 150, "output_tokens": 3})
    );
    assert_eq!(
        result.stopped_because.as_deref(),
        Some(
            "The decision service gave 3 unusable replies in a row. The first: \
             Invalid browser decision response; no action executed."
        )
    );
    assert_eq!(result.stop_cause, Some(JevStopCause::DecisionUnusable));
    assert_eq!(
        serde_json::to_value(&result).unwrap()["stop_cause"],
        "decision_unusable"
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

/// One unusable reply is asked again, and the run goes on as if it had not
/// happened, its billed usage still counted.
#[tokio::test]
async fn an_unusable_reply_is_asked_again() {
    let replies = Replies::new(&[(false, ""), (true, "DONE")]);
    let calls = replies.calls();
    let result = run(
        ScriptedBrowser::new(vec![page("one", &[("next", "click")])]),
        replies.config(),
    )
    .await;
    assert_eq!(result.status, JevStatus::Done);
    assert_eq!(*calls.lock().unwrap(), 2);
    assert_eq!(result.model_calls, 2);
    assert_eq!(result.stopped_because, None);
    assert_eq!(
        serde_json::to_value(&result).unwrap()["stop_cause"],
        Value::Null
    );
    assert_eq!(result.decisions.len(), 1);
    assert_eq!(
        serde_json::to_value(result.usage.decision).unwrap(),
        json!({"calls": 2, "input_tokens": 100, "output_tokens": 2})
    );
}

/// The cap is two asks per decision: a usable reply starts the count again,
/// so scattered bad replies never add up to a stop.
#[tokio::test]
async fn a_usable_reply_starts_the_unusable_count_again() {
    let replies = Replies::new(&[
        (false, ""),
        (false, ""),
        (true, "CLICK"),
        (false, ""),
        (false, ""),
        (true, "DONE"),
    ]);
    let calls = replies.calls();
    let result = run(ScriptedBrowser::new(steps(1)), replies.config()).await;
    assert_eq!(result.status, JevStatus::Done);
    assert_eq!(*calls.lock().unwrap(), 6);
    assert_eq!(result.model_calls, 6);
    assert_eq!(result.actions.len(), 1);
}

/// A hosted client that marks its own invalid replies with
/// [`JevBilled::unusable`], each with a longer-winded reason than the last.
struct Rambling(Mutex<usize>);

#[async_trait]
impl JevDecisionClient for Rambling {
    async fn choose(
        &self,
        _observation: &Value,
        _goal: &str,
        _history: &[Value],
    ) -> anyhow::Result<JevDecision> {
        let mut asked = self.0.lock().unwrap();
        *asked += 1;
        let reason = format!("reply {asked} was\nnot   json {}", "x".repeat(400));
        Err(JevBilled::unusable(json!({"input_tokens": 9}), anyhow::anyhow!(reason)).into())
    }
}

/// The stop reason names the count and the first reason, on one line and cut
/// short: a service's text is not trusted to be short.
#[tokio::test]
async fn the_stop_names_the_first_reason_on_one_line() {
    let config = JevEngineConfig::new("Scripted goal", Arc::new(Rambling(Mutex::new(0))))
        .with_wait(Duration::ZERO);
    let result = run(
        ScriptedBrowser::new(vec![page("one", &[("next", "click")])]),
        config,
    )
    .await;
    assert_eq!(result.status, JevStatus::Error);
    assert_eq!(result.model_calls, 3);
    assert_eq!(result.stop_cause, Some(JevStopCause::DecisionUnusable));
    let reason = result.stopped_because.unwrap();
    assert!(
        reason.starts_with(
            "The decision service gave 3 unusable replies in a row. The first: \
             reply 1 was not json xxx"
        ),
        "{reason}"
    );
    assert!(
        !reason.contains('\n') && !reason.contains("reply 2"),
        "{reason}"
    );
    assert!(reason.chars().count() < 280, "{}", reason.chars().count());
    assert!(reason.ends_with('…'), "{reason}");
}

/// Only a reply the service gave that failed validation is asked again: a
/// decision that fails any other way ends the run on its first failure.
#[tokio::test]
async fn a_decision_that_fails_otherwise_is_not_asked_again() {
    for status in [None, Some(JevStatus::Unavailable)] {
        let decider = ScriptedDecider::new(&["next"]).failing_after(0, status);
        let log = decider.log();
        let result = run(
            ScriptedBrowser::new(vec![page("one", &[("next", "click")])]),
            config(decider),
        )
        .await;
        assert_eq!(log.lock().unwrap().seen.len(), 1, "{status:?}");
        assert_eq!(
            result.status,
            status.unwrap_or(JevStatus::Error),
            "{status:?}"
        );
        assert_eq!(
            serde_json::to_value(&result).unwrap()["stop_cause"],
            "stopped",
            "{status:?}"
        );
    }
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
