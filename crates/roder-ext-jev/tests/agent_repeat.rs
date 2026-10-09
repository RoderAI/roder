//! The repeat guard: an unsure click that repeats the click just made ends
//! the run `done` without clicking.
//!
//! Two arms. The same control again is the older rule (`agent_loop.rs`). A
//! twin is the newer: a different control with the same label, whose label
//! names a commitment, chosen below the repeat confidence right after a click
//! that changed the page. The recorded failure (`icon_by_picture`) deleted
//! Bob's budget mail at 0.99 and then his lunch mail at 0.60.

mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use roder_ext_jev::{
    JevEngine, JevEngineConfig, JevRunResult, JevStatus, JevSuppressedClick, JevSuppressedKind,
    JevTextValue, JevTextValueResolver,
};
use serde_json::{Value, json};
use support::{BrowserLog, ScriptedBrowser, ScriptedDecider, labelled_page};

const RUN_TIMEOUT: Duration = Duration::from_secs(5);

async fn run(browser: ScriptedBrowser, decider: ScriptedDecider) -> JevRunResult {
    let config = JevEngineConfig::new(
        "Delete the email from Bob about the budget.",
        Arc::new(decider),
    )
    .with_wait(Duration::ZERO);
    let mut engine = JevEngine::start(Box::new(browser), config).await.unwrap();
    engine.run(RUN_TIMEOUT).await
}

/// A page with one click per `(id, label, row)`, each named by its row.
fn mailbox(fingerprint: &str, controls: &[(&str, &str, &str)]) -> Value {
    let mut observation = labelled_page(
        fingerprint,
        &controls
            .iter()
            .map(|(id, _, _)| (*id, "click", ""))
            .collect::<Vec<_>>(),
    );
    for (action, (_, label, row)) in observation["actions"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .zip(controls)
    {
        action["label"] = json!(label);
        action["context"] = json!(row);
    }
    observation
}

/// The recorded failure's pages: two delete icons, then one, then none.
fn bob_pages(first: &str, second: &str) -> Vec<Value> {
    vec![
        mailbox(
            "inbox-2",
            &[("e6", first, "Bob Budget"), ("e9", second, "Bob Lunch")],
        ),
        mailbox("inbox-1", &[("e9", second, "Bob Lunch")]),
        mailbox("inbox-0", &[]),
    ]
}

fn clicked(log: &Mutex<BrowserLog>) -> Vec<String> {
    let log = log.lock().unwrap();
    log.acts.iter().map(|act| act.id.clone()).collect()
}

#[tokio::test]
async fn an_unsure_delete_on_a_twin_row_ends_done_without_clicking() {
    let browser = ScriptedBrowser::new(bob_pages("Delete", "Delete"));
    let log = browser.log();
    // The DONE is there for a loop that clicks again; this one never asks.
    let decider = ScriptedDecider::new(&["e6", "e9", "DONE"]).with_confidences(&[1.0, 0.6, 0.5]);

    let result = run(browser, decider).await;

    assert_eq!(clicked(&log), ["e6"], "{result:#?}");
    assert_eq!(result.status, JevStatus::Done);
    assert_eq!(result.stopped_because, None);
    assert_eq!(result.model_calls, 2);
    assert_eq!(result.actions.len(), 1);
    assert_eq!(result.actions[0].choice, "e6");
    // The caller is told which click was not made, and why it was read as
    // a repeat: which row it sat in and which row the click before it hit.
    assert_eq!(
        result.suppressed_click,
        Some(JevSuppressedClick {
            kind: JevSuppressedKind::TwinControl,
            label: "Delete".into(),
            context: Some("Bob Lunch".into()),
            previous_context: Some("Bob Budget".into()),
            confidence: 0.6,
        })
    );
    let data = serde_json::to_value(&result).unwrap();
    assert_eq!(
        data["suppressed_click"],
        json!({
            "kind": "twin_control", "label": "Delete", "context": "Bob Lunch",
            "previous_context": "Bob Budget", "confidence": 0.6,
        })
    );
}

#[tokio::test]
async fn a_confident_delete_on_a_twin_row_clicks_it() {
    let browser = ScriptedBrowser::new(bob_pages("Delete", "Delete"));
    let log = browser.log();
    let decider = ScriptedDecider::new(&["e6", "e9", "DONE"]).with_confidences(&[1.0, 0.8, 0.5]);

    let result = run(browser, decider).await;

    assert_eq!(result.status, JevStatus::Done);
    assert_eq!(result.model_calls, 3);
    assert_eq!(result.actions.len(), 2);
    assert_eq!(clicked(&log), ["e6", "e9"]);
    // Nothing was held back, so nothing is reported.
    assert_eq!(result.suppressed_click, None);
    assert!(
        serde_json::to_value(&result)
            .unwrap()
            .get("suppressed_click")
            .is_none()
    );
}

#[tokio::test]
async fn a_delete_on_a_twin_row_at_the_repeat_confidence_clicks_it() {
    let browser = ScriptedBrowser::new(bob_pages("Delete", "Delete"));
    let log = browser.log();
    let decider = ScriptedDecider::new(&["e6", "e9", "DONE"]).with_confidences(&[1.0, 0.7, 0.5]);

    let result = run(browser, decider).await;

    assert_eq!(result.status, JevStatus::Done);
    assert_eq!(clicked(&log), ["e6", "e9"]);
}

#[tokio::test]
async fn an_unsure_twin_whose_label_commits_nothing_clicks_as_before() {
    let browser = ScriptedBrowser::new(bob_pages("Open", "Open"));
    let log = browser.log();
    let decider = ScriptedDecider::new(&["e6", "e9", "DONE"]).with_confidences(&[1.0, 0.6, 0.5]);

    let result = run(browser, decider).await;

    assert_eq!(result.status, JevStatus::Done);
    assert_eq!(result.model_calls, 3);
    assert_eq!(clicked(&log), ["e6", "e9"]);
}

#[tokio::test]
async fn an_unsure_delete_under_another_label_clicks_as_before() {
    // Only a control the page labels the same is a twin.
    let browser = ScriptedBrowser::new(bob_pages("Delete", "Delete forever"));
    let log = browser.log();
    let decider = ScriptedDecider::new(&["e6", "e9", "DONE"]).with_confidences(&[1.0, 0.6, 0.5]);

    let result = run(browser, decider).await;

    assert_eq!(result.status, JevStatus::Done);
    assert_eq!(clicked(&log), ["e6", "e9"]);
}

#[tokio::test]
async fn an_unsure_delete_after_a_click_that_changed_nothing_clicks() {
    // The first click did nothing, so the second is not a repeat of a click
    // that took effect.
    let page = mailbox(
        "inbox-2",
        &[
            ("e6", "Delete", "Bob Budget"),
            ("e9", "Delete", "Bob Lunch"),
        ],
    );
    let browser = ScriptedBrowser::new(vec![page.clone(), page, mailbox("inbox-1", &[])]);
    let log = browser.log();
    let decider = ScriptedDecider::new(&["e6", "e9", "DONE"]).with_confidences(&[1.0, 0.6, 0.5]);

    let result = run(browser, decider).await;

    assert_eq!(result.status, JevStatus::Done);
    assert_eq!(result.actions[0].page_changed, Some(false));
    assert_eq!(clicked(&log), ["e6", "e9"]);
}

#[tokio::test]
async fn an_unsure_delete_after_a_different_click_in_between_clicks() {
    // The guard looks at the step just before: a click on Search in between
    // makes the later delete a fresh choice.
    let delete_both = |fingerprint| {
        mailbox(
            fingerprint,
            &[
                ("e6", "Delete", "Bob Budget"),
                ("e8", "Search", ""),
                ("e9", "Delete", "Bob Lunch"),
            ],
        )
    };
    let browser = ScriptedBrowser::new(vec![
        delete_both("inbox-2"),
        mailbox(
            "inbox-1",
            &[("e8", "Search", ""), ("e9", "Delete", "Bob Lunch")],
        ),
        mailbox(
            "inbox-1-search",
            &[("e8", "Search", ""), ("e9", "Delete", "Bob Lunch")],
        ),
        mailbox("inbox-0", &[("e8", "Search", "")]),
    ]);
    let log = browser.log();
    let decider =
        ScriptedDecider::new(&["e6", "e8", "e9", "DONE"]).with_confidences(&[1.0, 1.0, 0.6, 0.5]);

    let result = run(browser, decider).await;

    assert_eq!(result.status, JevStatus::Done);
    assert_eq!(clicked(&log), ["e6", "e8", "e9"]);
}

#[tokio::test]
async fn an_unsure_repeat_of_the_same_control_still_ends_done() {
    // The older arm, unchanged: the same id and the same label.
    let browser = ScriptedBrowser::new(vec![
        mailbox("inbox-2", &[("e6", "Delete", "Bob Budget")]),
        mailbox("inbox-1", &[("e6", "Delete", "Bob Budget")]),
    ]);
    let log = browser.log();
    let decider = ScriptedDecider::new(&["e6", "e6"]).with_confidences(&[1.0, 0.5]);

    let result = run(browser, decider).await;

    assert_eq!(result.status, JevStatus::Done);
    assert_eq!(result.model_calls, 2);
    assert_eq!(clicked(&log), ["e6"]);
    let held = result.suppressed_click.expect("the held click is reported");
    assert_eq!(held.kind, JevSuppressedKind::SameControl);
    assert_eq!(held.label, "Delete");
    assert_eq!(held.context.as_deref(), Some("Bob Budget"));
    assert_eq!(held.confidence, 0.5);
}

/// A password field the run types into, then two deletes whose second row
/// name shows the password back.
struct Password;

#[async_trait]
impl JevTextValueResolver for Password {
    async fn resolve(&self, _field_context: &Value) -> anyhow::Result<JevTextValue> {
        Ok(JevTextValue {
            value: "hunter2-secret".into(),
            model: "fixed".into(),
            latency_ms: 0,
            usage: json!({}),
        })
    }
}

#[tokio::test]
async fn what_is_reported_of_the_click_is_scrubbed_and_cut() {
    let long_row = format!("Row hunter2-secret {}", "tail ".repeat(100));
    let mut first = mailbox("form", &[("pw", "Password", ""), ("e6", "Delete", "Row A")]);
    first["actions"][0]["kind"] = json!("fill");
    first["actions"][0]["input_type"] = json!("password");
    let browser = ScriptedBrowser::new(vec![
        first,
        mailbox(
            "typed",
            &[("e6", "Delete", "Row A"), ("e9", "Delete", &long_row)],
        ),
        mailbox("deleted", &[("e9", "Delete", &long_row)]),
    ]);
    let log = browser.log();
    let decider =
        ScriptedDecider::new(&["pw", "e6", "e9", "DONE"]).with_confidences(&[1.0, 1.0, 0.6, 0.5]);
    let config = JevEngineConfig::new("Delete the rows.", Arc::new(decider))
        .with_wait(Duration::ZERO)
        .with_text_resolver(Arc::new(Password));
    let mut engine = JevEngine::start(Box::new(browser), config).await.unwrap();

    let result = engine.run(RUN_TIMEOUT).await;

    assert_eq!(clicked(&log), ["pw", "e6"], "{result:#?}");
    let held = result.suppressed_click.expect("the held click is reported");
    let context = held.context.clone().expect("its row");
    assert!(context.starts_with("Row [secret] tail"), "{context}");
    assert!(
        context.chars().count() <= 120,
        "{} chars",
        context.chars().count()
    );
    assert!(context.ends_with('…'), "{context}");
    assert!(!format!("{held:?}").contains("hunter2"));
}
