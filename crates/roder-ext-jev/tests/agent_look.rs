//! Looking before deciding: a page that refused automated access ends the
//! run before any decision, an empty first read is read again, and a
//! BLOCKED about a page that shows nothing is checked once more.

mod support;

use std::sync::Arc;
use std::time::Duration;

use roder_ext_jev::{JevEngine, JevEngineConfig, JevPageFacts, JevRunResult, JevStatus};
use serde_json::{Value, json};
use support::{ScriptedBrowser, ScriptedDecider, page};

async fn run(browser: ScriptedBrowser, config: JevEngineConfig) -> JevRunResult {
    let mut engine = JevEngine::start(Box::new(browser), config).await.unwrap();
    engine.run(Duration::from_secs(20)).await
}

fn config(decider: ScriptedDecider) -> JevEngineConfig {
    JevEngineConfig::new("Book a table", Arc::new(decider)).with_wait(Duration::ZERO)
}

fn facts(status: u16) -> JevPageFacts {
    JevPageFacts {
        http_status: Some(status),
        headings: vec!["Access Denied".into()],
        frames: Vec::new(),
    }
}

fn denied() -> Value {
    let mut denied = page("denied", &[]);
    denied["title"] = json!("Access Denied");
    denied["text"] = json!("You don't have permission to access this server.");
    denied
}

#[tokio::test]
async fn access_denied_spends_no_decisions() {
    let browser = ScriptedBrowser::new(vec![denied()]).with_facts(&[facts(403)]);
    let decider = ScriptedDecider::new(&["DONE"]);
    let decider_log = decider.log();
    let result = run(browser, config(decider)).await;
    assert_eq!(result.status, JevStatus::AccessDenied);
    assert_eq!(result.model_calls, 0);
    assert!(decider_log.lock().unwrap().seen.is_empty());
    assert_eq!(
        result.stopped_because.as_deref(),
        Some("The site refused automated access (HTTP 403, \"Access Denied\")")
    );
    assert_eq!(result.page.as_ref().unwrap().http_status, Some(403));
    let value = serde_json::to_value(&result).unwrap();
    assert_eq!(value["status"], json!("access_denied"));
    assert_eq!(value["page"]["headings"], json!(["Access Denied"]));
}

#[tokio::test]
async fn a_page_with_controls_and_a_200_is_not_a_block() {
    let mut article = page("article", &[("more", "click")]);
    article["title"] = json!("How a captcha works");
    let browser = ScriptedBrowser::new(vec![article]).with_facts(&[JevPageFacts {
        http_status: Some(200),
        ..JevPageFacts::default()
    }]);
    let result = run(browser, config(ScriptedDecider::new(&["DONE"]))).await;
    assert_eq!(result.status, JevStatus::Done);
    assert_eq!(result.model_calls, 1);
}

#[tokio::test]
async fn a_blocked_run_that_ends_on_a_refusal_is_named_for_it() {
    // The first page loads; the click leads to a rate-limit wall.
    let browser = ScriptedBrowser::new(vec![page("search", &[("go", "click")]), denied()])
        .with_facts(&[
            JevPageFacts {
                http_status: Some(200),
                ..JevPageFacts::default()
            },
            facts(429),
        ]);
    let result = run(browser, config(ScriptedDecider::new(&["go", "BLOCKED"]))).await;
    assert_eq!(result.status, JevStatus::AccessDenied);
    assert!(
        result
            .stopped_because
            .as_deref()
            .is_some_and(|reason| reason.contains("HTTP 429")),
        "{:?}",
        result.stopped_because
    );
    assert_eq!(result.actions.len(), 1);
}

#[tokio::test]
async fn empty_first_observation_is_read_again() {
    let mut blank = page("blank", &[]);
    blank["title"] = json!("");
    blank["text"] = json!("");
    let browser = ScriptedBrowser::new(vec![blank.clone(), blank, page("app", &[("go", "click")])]);
    let log = browser.log();
    let decider = ScriptedDecider::new(&["go", "DONE"]);
    let decider_log = decider.log();
    let result = run(browser, config(decider)).await;
    assert_eq!(result.status, JevStatus::Done);
    assert_eq!(result.actions.len(), 1);
    // Three reads before the first decision, which saw the rendered page.
    assert_eq!(decider_log.lock().unwrap().seen[0], json!("app"));
    assert!(log.lock().unwrap().observes >= 4);
}

#[tokio::test]
async fn a_blocked_on_an_empty_page_is_checked_once() {
    let mut blank = page("blank", &[]);
    blank["text"] = json!("");
    // Blank at the start and through its re-reads, then rendered only
    // when the BLOCKED is checked.
    let browser = ScriptedBrowser::new(vec![
        blank.clone(),
        blank.clone(),
        blank.clone(),
        blank,
        page("late", &[("go", "click")]),
    ]);
    let decider = ScriptedDecider::new(&["BLOCKED", "go", "DONE"]);
    let result = run(browser, config(decider)).await;
    // The start's re-reads count as the check: the BLOCKED stands.
    assert_eq!(result.status, JevStatus::Blocked);
    assert_eq!(result.model_calls, 1);
}

#[tokio::test]
async fn a_blocked_on_a_page_gone_blank_is_read_again_once() {
    let mut blank = page("blank", &[]);
    blank["text"] = json!("");
    let browser = ScriptedBrowser::new(vec![
        page("list", &[("open", "click")]),
        blank,
        page("detail", &[("book", "click")]),
    ]);
    let decider = ScriptedDecider::new(&["open", "BLOCKED", "DONE"]);
    let decider_log = decider.log();
    let result = run(browser, config(decider)).await;
    assert_eq!(result.status, JevStatus::Done);
    assert_eq!(result.model_calls, 3);
    let seen = decider_log.lock().unwrap().seen.clone();
    assert_eq!(seen, [json!("list"), json!("blank"), json!("detail")]);
}
