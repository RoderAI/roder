//! The gate and the banner refusal, driven through the real loop with
//! scripted fakes: the irreversible-action gate, off by default, which stops
//! a run `needs_confirmation` before dispatching an action that may not be
//! undone, and cookie-banner refusal, on by default, recorded as a step that
//! costs no model call. Off, each changes nothing.

mod support;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use roder_ext_jev::{
    JevEngine, JevEngineConfig, JevRunResult, JevStatus, JevTextValue, JevTextValueResolver,
};
use serde_json::{Value, json};
use support::{ScriptedBrowser, ScriptedDecider, labelled_page};

const RUN_TIMEOUT: Duration = Duration::from_secs(5);

async fn run(browser: ScriptedBrowser, config: JevEngineConfig) -> JevRunResult {
    let mut engine = JevEngine::start(Box::new(browser), config).await.unwrap();
    engine.run(RUN_TIMEOUT).await
}

fn config(decider: ScriptedDecider) -> JevEngineConfig {
    JevEngineConfig::new("Scripted goal", Arc::new(decider)).with_wait(Duration::ZERO)
}

fn checkout(fingerprint: &str) -> Value {
    labelled_page(
        fingerprint,
        &[
            ("cart", "click", "Add to cart"),
            ("pay", "click", "Pay now"),
            ("delete", "click", "Delete account"),
        ],
    )
}

#[tokio::test]
async fn off_by_default_every_click_is_dispatched_ungated() {
    let browser = ScriptedBrowser::new(vec![checkout("a"), checkout("b")]);
    let browser_log = browser.log();
    let decider = ScriptedDecider::new(&["pay", "DONE"]).with_irreversible(&[Some(0.99)]);
    let decider_log = decider.log();

    let result = run(browser, config(decider)).await;

    assert_eq!(result.status, JevStatus::Done);
    assert_eq!(browser_log.lock().unwrap().acts.len(), 1);
    // Banner refusal is on by default: checked before both observations.
    assert_eq!(browser_log.lock().unwrap().banner_checks, 2);
    // The gate's variant of the request was never asked for.
    assert_eq!(decider_log.lock().unwrap().gated, [false, false]);
    assert_eq!(result.decisions[0].irreversible, None);
}

#[tokio::test]
async fn banner_refusal_turned_off_never_asks_the_browser() {
    let browser = ScriptedBrowser::new(vec![checkout("a"), checkout("b")])
        .with_banners(&[Some("Reject all")]);
    let browser_log = browser.log();
    let decider = ScriptedDecider::new(&["cart", "DONE"]);

    let result = run(browser, config(decider).with_cookie_banner_refusal(false)).await;

    assert_eq!(result.status, JevStatus::Done);
    assert_eq!(browser_log.lock().unwrap().banner_checks, 0);
    assert!(result.actions.iter().all(|action| action.kind == "click"));
}

#[tokio::test]
async fn an_unauthorized_irreversible_click_ends_the_run_undispatched() {
    let browser = ScriptedBrowser::new(vec![checkout("a")]);
    let browser_log = browser.log();
    let decider = ScriptedDecider::new(&["pay"]).with_irreversible(&[Some(0.93)]);
    let decider_log = decider.log();

    let result = run(browser, config(decider).with_irreversible_gate()).await;

    assert_eq!(result.status, JevStatus::NeedsConfirmation);
    assert!(browser_log.lock().unwrap().acts.is_empty());
    assert!(result.actions.is_empty());
    assert_eq!(result.model_calls, 1);
    assert_eq!(decider_log.lock().unwrap().gated, [true]);
    assert_eq!(result.decisions[0].irreversible, Some(0.93));
    let reason = result.stopped_because.unwrap();
    assert!(
        reason.starts_with("Jev did not click \"Pay now\""),
        "{reason}"
    );
    assert!(reason.contains("(P=0.93)"), "{reason}");
    assert!(reason.contains("not authorized"), "{reason}");
    assert_eq!(
        serde_json::to_value(result.status).unwrap(),
        json!("needs_confirmation")
    );
}

#[tokio::test]
async fn a_missing_answer_fails_closed() {
    let browser = ScriptedBrowser::new(vec![checkout("a")]);
    let browser_log = browser.log();
    // No answer scripted: the client asked but got nothing usable back.
    let decider = ScriptedDecider::new(&["delete"]);

    let result = run(browser, config(decider).with_irreversible_gate()).await;

    assert_eq!(result.status, JevStatus::NeedsConfirmation);
    assert!(browser_log.lock().unwrap().acts.is_empty());
    let reason = result.stopped_because.unwrap();
    assert!(reason.contains("\"Delete account\""), "{reason}");
    assert!(reason.contains("no valid answer"), "{reason}");
}

#[tokio::test]
async fn reversible_and_unshortlisted_clicks_go_through() {
    let browser = ScriptedBrowser::new(vec![checkout("a"), checkout("b"), checkout("c")]);
    let browser_log = browser.log();
    // "Add to cart" is not asked about, so it needs no answer; "Pay now" is
    // judged reversible here.
    let decider =
        ScriptedDecider::new(&["cart", "pay", "DONE"]).with_irreversible(&[None, Some(0.2)]);

    let result = run(browser, config(decider).with_irreversible_gate()).await;

    assert_eq!(result.status, JevStatus::Done);
    let acts = browser_log.lock().unwrap().acts.clone();
    assert_eq!(
        acts.iter().map(|act| act.id.as_str()).collect::<Vec<_>>(),
        ["cart", "pay"]
    );
}

#[tokio::test]
async fn an_authorized_run_dispatches_only_a_confident_decision() {
    let browser = ScriptedBrowser::new(vec![checkout("a"), checkout("b")]);
    let browser_log = browser.log();
    let decider = ScriptedDecider::new(&["delete", "DONE"])
        .with_irreversible(&[Some(0.99)])
        .with_confidences(&[0.95]);
    let authorized = config(decider)
        .with_irreversible_gate()
        .with_irreversible_authorized();

    let result = run(browser, authorized).await;

    assert_eq!(result.status, JevStatus::Done);
    assert_eq!(browser_log.lock().unwrap().acts.len(), 1);

    let browser = ScriptedBrowser::new(vec![checkout("a")]);
    let browser_log = browser.log();
    let decider = ScriptedDecider::new(&["delete"])
        .with_irreversible(&[Some(0.99)])
        .with_confidences(&[0.8]);
    let unsure = config(decider)
        .with_irreversible_gate()
        .with_irreversible_authorized();

    let result = run(browser, unsure).await;

    assert_eq!(result.status, JevStatus::NeedsConfirmation);
    assert!(browser_log.lock().unwrap().acts.is_empty());
    let reason = result.stopped_because.unwrap();
    assert!(reason.contains("(0.80 < 0.90)"), "{reason}");
}

struct Fixed;

#[async_trait]
impl JevTextValueResolver for Fixed {
    async fn resolve(&self, _field_context: &Value) -> anyhow::Result<JevTextValue> {
        Ok(JevTextValue {
            value: "See you at 5".into(),
            model: "fixed".into(),
            latency_ms: 0,
            usage: json!({}),
        })
    }
}

#[tokio::test]
async fn enter_after_a_fill_is_gated_the_same_way() {
    let typed = labelled_page(
        "typed",
        &[
            ("message", "fill", "Message"),
            ("enter-message", "enter", "Message"),
        ],
    );
    let browser = ScriptedBrowser::new(vec![
        labelled_page("empty", &[("message", "fill", "Message")]),
        typed,
    ]);
    let browser_log = browser.log();
    let decider =
        ScriptedDecider::new(&["message", "enter-message"]).with_irreversible(&[None, Some(0.9)]);

    let result = run(
        browser,
        config(decider)
            .with_irreversible_gate()
            .with_text_resolver(Arc::new(Fixed)),
    )
    .await;

    assert_eq!(result.status, JevStatus::NeedsConfirmation);
    // The fill ran; the Enter that would send it did not.
    let acts = browser_log.lock().unwrap().acts.clone();
    assert_eq!(acts.len(), 1);
    assert_eq!(acts[0].id, "message");
    let reason = result.stopped_because.unwrap();
    assert!(
        reason.starts_with("Jev did not press Enter in \"Message\""),
        "{reason}"
    );
}

#[tokio::test]
async fn a_page_that_changed_under_the_gate_is_decided_again() {
    // The decision's page moved on before the gate stopped the run, so the
    // loop observes again and decides on the new page instead.
    let browser =
        ScriptedBrowser::new(vec![checkout("a"), checkout("b")]).with_fresh(&[true, false]);
    let browser_log = browser.log();
    let decider = ScriptedDecider::new(&["pay", "DONE"]).with_irreversible(&[Some(0.95)]);

    let result = run(browser, config(decider).with_irreversible_gate()).await;

    assert_eq!(result.status, JevStatus::Done);
    assert!(browser_log.lock().unwrap().acts.is_empty());
    assert_eq!(result.model_calls, 2);
}

#[tokio::test]
async fn a_refused_banner_is_a_step_that_costs_no_model_call_or_budget() {
    let browser = ScriptedBrowser::new(vec![
        labelled_page("banner", &[("go", "click", "Continue")]),
        labelled_page("after", &[("go", "click", "Continue")]),
    ])
    .with_banners(&[Some("Reject all")]);
    let browser_log = browser.log();

    let result = run(
        browser,
        config(ScriptedDecider::new(&["go", "DONE"])).with_max_actions(1),
    )
    .await;

    assert_eq!(
        result.status,
        JevStatus::Done,
        "{:?}",
        result.stopped_because
    );
    assert_eq!(result.model_calls, 2);
    let records = result
        .actions
        .iter()
        .map(|action| format!("{} {}", action.kind, action.action))
        .collect::<Vec<_>>();
    assert_eq!(
        records,
        [
            "cookie_banner refused cookie banner: Reject all",
            "click Continue"
        ]
    );
    assert_eq!(result.actions[0].step, 1);
    assert_eq!(result.actions[0].page_changed, None);
    assert_eq!(result.actions[1].page_changed, Some(true));
    // Checked before every observation: the first and the one after the click.
    assert_eq!(browser_log.lock().unwrap().banner_checks, 2);
    assert_eq!(browser_log.lock().unwrap().acts.len(), 1);
}

#[tokio::test]
async fn a_banner_refused_after_a_step_leaves_that_steps_record_intact() {
    let dialogs = json!([{"type": "alert", "message": "Saved", "accepted": true}]);
    let mut after = labelled_page("after", &[("go", "click", "Continue")]);
    after["dialogs"] = dialogs;
    let browser = ScriptedBrowser::new(vec![
        labelled_page("before", &[("go", "click", "Continue")]),
        after,
    ])
    .with_banners(&[None, Some("Decline")]);

    let result = run(browser, config(ScriptedDecider::new(&["go", "DONE"]))).await;

    assert_eq!(result.status, JevStatus::Done);
    assert_eq!(result.actions.len(), 2);
    let click = &result.actions[0];
    assert_eq!(click.kind, "click");
    assert_eq!(click.page_changed, Some(true));
    assert_eq!(click.dialogs.len(), 1);
    assert_eq!(result.actions[1].action, "refused cookie banner: Decline");
    assert!(result.actions[1].dialogs.is_empty());
    let serialized = serde_json::to_value(&result.actions[1]).unwrap();
    assert_eq!(serialized["kind"], json!("cookie_banner"));
}
