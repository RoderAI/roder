//! What the loop records besides executing an action: a step the page
//! refused is recorded and the run goes on, dialogs the page opened are
//! recorded on the step before them, and a blocked run whose page asked for
//! a confirmation Jev declined says so.

mod support;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use roder_ext_jev::{
    JevEngine, JevEngineConfig, JevRunResult, JevStatus, JevTextValue, JevTextValueResolver,
};
use serde_json::{Value, json};
use support::{ScriptedBrowser, ScriptedDecider, page, page_with_dialogs};

const RUN_TIMEOUT: Duration = Duration::from_secs(5);

async fn run(browser: ScriptedBrowser, config: JevEngineConfig) -> JevRunResult {
    let mut engine = JevEngine::start(Box::new(browser), config).await.unwrap();
    engine.run(RUN_TIMEOUT).await
}

fn config(decider: ScriptedDecider) -> JevEngineConfig {
    JevEngineConfig::new("Scripted goal", Arc::new(decider)).with_wait(Duration::ZERO)
}

struct FixedResolver;

#[async_trait]
impl JevTextValueResolver for FixedResolver {
    async fn resolve(&self, _field_context: &Value) -> anyhow::Result<JevTextValue> {
        Ok(JevTextValue {
            value: "SPRING".into(),
            model: "fixed".into(),
            latency_ms: 0,
            usage: json!({}),
        })
    }
}

#[tokio::test]
async fn a_refused_action_is_recorded_and_the_run_goes_on() {
    let browser = ScriptedBrowser::new(vec![
        page("size-medium", &[("large", "select"), ("small", "select")]),
        page(
            "size-medium-note",
            &[("large", "select"), ("small", "select")],
        ),
        page("size-small", &[("large", "select")]),
    ])
    .with_refusals(&[Some("The page kept \"Medium\" instead of \"Large\"."), None]);
    let browser_log = browser.log();

    let result = run(
        browser,
        config(ScriptedDecider::new(&["large", "small", "DONE"])),
    )
    .await;

    // Before, a refused select bailed and lost the executed action.
    assert_eq!(result.status, JevStatus::Done);
    assert_eq!(result.stopped_because, None);
    assert_eq!(result.actions.len(), 2);
    assert_eq!(
        result.actions[0].refused.as_deref(),
        Some("The page kept \"Medium\" instead of \"Large\".")
    );
    assert_eq!(result.actions[1].refused, None);
    assert_eq!(browser_log.lock().unwrap().acts.len(), 2);
    let serialized = serde_json::to_value(&result.actions).unwrap();
    assert_eq!(
        serialized[0]["refused"],
        json!("The page kept \"Medium\" instead of \"Large\".")
    );
    assert!(serialized[1].get("refused").is_none());
}

#[tokio::test]
async fn an_action_the_page_keeps_refusing_stalls_the_run() {
    let browser = ScriptedBrowser::new(vec![page("form", &[("promo", "fill")])])
        .with_refusals(&[Some("The field shows \"\", not the typed text."); 5]);
    let config =
        config(ScriptedDecider::new(&["promo"])).with_text_resolver(Arc::new(FixedResolver));

    let result = run(browser, config).await;

    assert_eq!(result.status, JevStatus::Blocked);
    assert_eq!(result.stopped_because, None);
    assert_eq!(result.actions.len(), 3);
    assert!(result.actions.iter().all(|action| action.refused.is_some()
        && action.page_changed == Some(false)
        && action.text.as_deref() == Some("SPRING")));
}

fn confirm(message: &str) -> Value {
    json!([{"type": "confirm", "message": message, "accepted": false}])
}

#[tokio::test]
async fn a_dismissed_confirm_is_recorded_and_explains_a_blocked_run() {
    let browser = ScriptedBrowser::new(vec![
        page("drafts", &[("delete", "click")]),
        page_with_dialogs(
            "drafts",
            &[("delete", "click")],
            confirm("Delete Q3 notes?"),
        ),
    ]);

    let result = run(
        browser,
        config(ScriptedDecider::new(&["delete", "BLOCKED"])),
    )
    .await;

    assert_eq!(result.status, JevStatus::Blocked);
    assert_eq!(result.actions.len(), 1);
    let dialogs = &result.actions[0].dialogs;
    assert_eq!(dialogs.len(), 1);
    assert_eq!(
        (
            dialogs[0].kind.as_str(),
            dialogs[0].message.as_str(),
            dialogs[0].accepted
        ),
        ("confirm", "Delete Q3 notes?", false)
    );
    assert_eq!(
        result.stopped_because.as_deref(),
        Some(
            "The page asked \"Delete Q3 notes?\" in a confirm dialog and Jev dismissed it: \
             Jev never accepts a confirm or answers a prompt. If the task needs it accepted, \
             do that step yourself."
        )
    );
    // The note reaches the model as page text.
    assert!(result.visible_text.contains("Delete Q3 notes?"));
}

#[tokio::test]
async fn an_accepted_alert_is_recorded_but_explains_nothing() {
    let alert = json!([{"type": "alert", "message": "Saved.", "accepted": true}]);
    let browser = ScriptedBrowser::new(vec![
        page("draft", &[("save", "click")]),
        page_with_dialogs("draft-saved", &[("save", "click")], alert),
    ]);

    let result = run(browser, config(ScriptedDecider::new(&["save", "BLOCKED"]))).await;

    assert_eq!(result.status, JevStatus::Blocked);
    assert_eq!(result.stopped_because, None);
    assert!(result.actions[0].dialogs[0].accepted);
}

#[tokio::test]
async fn a_dialog_seen_on_a_stale_reread_goes_on_the_step_before_it() {
    let browser = ScriptedBrowser::new(vec![
        page("one", &[("next", "click")]),
        page("two", &[("next", "click")]),
        page_with_dialogs("three", &[("next", "click")], confirm("Leave?")),
    ])
    // predict, predict (stale: re-read), predict, the DONE check.
    .with_fresh(&[true, false, true, true]);

    let result = run(browser, config(ScriptedDecider::new(&["next", "DONE"]))).await;

    assert_eq!(result.status, JevStatus::Done);
    // A done run is not explained by a dialog.
    assert_eq!(result.stopped_because, None);
    assert_eq!(result.actions.len(), 1);
    assert_eq!(result.actions[0].dialogs.len(), 1);
    assert_eq!(result.actions[0].dialogs[0].message, "Leave?");
}
