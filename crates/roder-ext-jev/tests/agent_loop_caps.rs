//! The run-wide caps on a run that goes round in circles.
//!
//! Upstream's stall rule needs three executed actions in a row that changed
//! nothing. It misses a menu that opens and closes (every step changes the
//! page), a wait between no-ops, and a page that never holds still, where
//! each decision goes stale and nothing is recorded. Each of those used to
//! run to the 60-action or 120-call budget, or the timeout, which forfeits
//! the fallback. Three caps end such a run `blocked`, so it falls back like a
//! stall does:
//!
//! - the fourth identical (page, control) pair is `looped`;
//! - three stale decisions in a row, each leaving nothing new on the page, are
//!   `unsettled`; and
//! - six waits in a row that changed nothing are `looped`, and an unchanged
//!   wait no longer hides a stall.

mod support;

use std::sync::Arc;
use std::time::Duration;

use roder_ext_jev::{JevEngine, JevEngineConfig, JevRunResult, JevStatus, JevStopCause};
use serde_json::{Value, json};
use support::{ScriptedBrowser, ScriptedDecider, labelled_page, page};

const RUN_TIMEOUT: Duration = Duration::from_secs(5);

async fn run(browser: ScriptedBrowser, decider: ScriptedDecider) -> JevRunResult {
    let config = JevEngineConfig::new("Scripted goal", Arc::new(decider)).with_wait(Duration::ZERO);
    let mut engine = JevEngine::start(Box::new(browser), config).await.unwrap();
    engine.run(RUN_TIMEOUT).await
}

/// A menu that is closed (`open` offered) or open (`close` offered).
fn menu(open: bool) -> Value {
    match open {
        false => labelled_page("menu-closed", &[("open", "click", "Open menu")]),
        true => labelled_page("menu-open", &[("close", "click", "Close menu")]),
    }
}

/// The page after `n` presses of the menu button, alternating from closed.
fn menu_pages(len: usize) -> Vec<Value> {
    (0..len).map(|n| menu(n % 2 == 1)).collect()
}

/// What a model that keeps pressing the menu button chooses.
fn menu_choices(len: usize) -> Vec<&'static str> {
    (0..len)
        .map(|n| if n % 2 == 0 { "open" } else { "close" })
        .collect()
}

/// A page whose only change is a digit, as a clock or a countdown changes it.
fn ticking(n: usize) -> Value {
    page(&format!("tick-{n}"), &[("go", "click")])
}

/// `fresh()` answers for a page that goes stale at every decision: the
/// check before the decision passes, the one before DONE fails.
fn always_stale() -> Vec<bool> {
    [true, false].repeat(200)
}

#[tokio::test]
async fn an_open_close_cycle_ends_looped() {
    let browser = ScriptedBrowser::new(menu_pages(70));
    let browser_log = browser.log();

    let result = run(browser, ScriptedDecider::new(&menu_choices(70))).await;

    // Every step changed the page, so the stall rule never fired and only the
    // 60-action budget ended this run. The fourth time Jev would press "Open
    // menu" on the closed page it stops instead, before pressing it.
    assert_eq!(result.status, JevStatus::Blocked, "{result:#?}");
    assert_eq!(result.stop_cause, Some(JevStopCause::Looped));
    assert_eq!(result.actions.len(), 6);
    assert_eq!(result.model_calls, 7);
    assert_eq!(browser_log.lock().unwrap().acts.len(), 6);
    assert!(
        result
            .actions
            .iter()
            .all(|action| action.page_changed == Some(true))
    );
    // It names the control and says how often it was chosen already.
    let reason = result.stopped_because.as_deref().unwrap();
    assert!(reason.contains("\"Open menu\""), "{reason}");
    assert!(reason.contains("3 times"), "{reason}");
    assert_eq!(
        serde_json::to_value(&result).unwrap()["stop_cause"],
        json!("looped")
    );
}

#[tokio::test]
async fn the_stall_rule_still_fires_before_the_pair_counter() {
    let browser = ScriptedBrowser::new(vec![page("same", &[("button", "click")])]);

    let result = run(browser, ScriptedDecider::new(&["button"])).await;

    // The same pair three times in a row, with nothing changing: stalled.
    assert_eq!(result.status, JevStatus::Blocked);
    assert_eq!(result.stop_cause, Some(JevStopCause::Stalled));
    assert_eq!(result.model_calls, 3);
    assert_eq!(result.stopped_because, None);
}

#[tokio::test]
async fn pages_that_only_change_in_their_numbers_are_progress_to_the_pair_counter() {
    // A counter: the same control on a page that differs only in a digit,
    // twelve times. The corpus's `step_budget` task is this; the pair counter
    // compares the page's fingerprint as it is, so real numeric progress
    // never trips it.
    let pages = (0..30)
        .map(|n| page(&format!("count-{n}"), &[("add", "click")]))
        .collect();
    let mut choices = vec!["add"; 12];
    choices.push("DONE");

    let result = run(ScriptedBrowser::new(pages), ScriptedDecider::new(&choices)).await;

    assert_eq!(result.status, JevStatus::Done, "{result:#?}");
    assert_eq!(result.stop_cause, None);
    assert_eq!(result.actions.len(), 12);
}

#[tokio::test]
async fn three_presses_of_each_button_are_not_a_loop() {
    // "Open menu" three times on the closed page and "Close menu" three times
    // on the open one: no pair is chosen a fourth time.
    let result = run(
        ScriptedBrowser::new(menu_pages(12)),
        ScriptedDecider::new(&["open", "close", "open", "close", "open", "close", "DONE"]),
    )
    .await;

    assert_eq!(result.status, JevStatus::Done, "{result:#?}");
    assert_eq!(result.stop_cause, None);
    assert_eq!(result.actions.len(), 6);
}

#[tokio::test]
async fn a_long_label_in_the_reason_is_cut_to_one_line() {
    let label = format!("Open\n{} menu", "very ".repeat(80));
    let pages = (0..70)
        .map(|n| match n % 2 {
            0 => labelled_page("menu-closed", &[("open", "click", &label)]),
            _ => menu(true),
        })
        .collect();

    let result = run(
        ScriptedBrowser::new(pages),
        ScriptedDecider::new(&menu_choices(70)),
    )
    .await;

    assert_eq!(result.stop_cause, Some(JevStopCause::Looped), "{result:#?}");
    let reason = result.stopped_because.as_deref().unwrap();
    assert!(!reason.contains('\n'), "{reason}");
    assert!(reason.contains("Open very very"), "{reason}");
    assert!(reason.contains('…'), "{reason}");
    assert!(reason.chars().count() < 400, "{reason}");
}

#[tokio::test]
async fn six_unchanged_waits_end_looped() {
    let browser = ScriptedBrowser::new(vec![page("slow", &[("wait", "wait"), ("go", "click")])]);
    let browser_log = browser.log();

    let result = run(browser, ScriptedDecider::new(&["wait"])).await;

    // A wait that changes nothing never counted as a stalled step, so this
    // ran to the 60-action budget.
    assert_eq!(result.status, JevStatus::Blocked, "{result:#?}");
    assert_eq!(result.stop_cause, Some(JevStopCause::Looped));
    assert_eq!(result.actions.len(), 6);
    assert_eq!(result.model_calls, 6);
    assert_eq!(browser_log.lock().unwrap().acts.len(), 6);
    let reason = result.stopped_because.as_deref().unwrap();
    assert!(reason.contains("waited 6 times in a row"), "{reason}");
}

#[tokio::test]
async fn a_wait_that_changes_the_page_starts_the_count_again() {
    // Five idle waits, one that changes the page, five more, then DONE.
    // (A page with no control of its own is read again before it is believed,
    // which would shift the script.)
    let controls = [("wait", "wait"), ("go", "click")];
    let mut pages = vec![page("w0", &controls); 6];
    pages.extend(vec![page("w1", &controls); 6]);
    let mut choices = vec!["wait"; 11];
    choices.push("DONE");
    // The page is read once more per act: 5 unchanged, 1 changed, 5 unchanged.
    let result = run(ScriptedBrowser::new(pages), ScriptedDecider::new(&choices)).await;

    assert_eq!(result.status, JevStatus::Done, "{result:#?}");
    assert_eq!(result.stop_cause, None);
    assert_eq!(result.actions.len(), 11);
    let changed = result
        .actions
        .iter()
        .map(|action| action.page_changed)
        .collect::<Vec<_>>();
    assert_eq!(changed[..5], [Some(false); 5]);
    assert_eq!(changed[5], Some(true));
    assert_eq!(changed[6..], [Some(false); 5]);
}

#[tokio::test]
async fn an_unchanged_wait_does_not_hide_a_stall() {
    let browser =
        ScriptedBrowser::new(vec![page("same", &[("button", "click"), ("wait", "wait")])]);

    let result = run(
        browser,
        ScriptedDecider::new(&["button", "wait", "button", "wait", "button", "DONE"]),
    )
    .await;

    // Three clicks that changed nothing, with waits between them: stalled on
    // the third click. A wait used to break the run of three.
    assert_eq!(result.status, JevStatus::Blocked, "{result:#?}");
    assert_eq!(result.stop_cause, Some(JevStopCause::Stalled));
    assert_eq!(result.actions.len(), 5);
    assert_eq!(result.model_calls, 5);
}

#[tokio::test]
async fn a_page_that_never_holds_still_ends_unsettled_within_three_decisions() {
    // An on-screen ticker inside the form being filled: every look at the page
    // differs by a digit, the click's guard no longer matches, and the
    // decision goes stale. Nothing is recorded, so it used to go on until the
    // 120-call budget (or the timeout, which does not fall back).
    let pages = (0..200).map(ticking).collect();
    let browser = ScriptedBrowser::new(pages).with_fresh(&always_stale());
    let browser_log = browser.log();

    let result = run(browser, ScriptedDecider::new(&["DONE"])).await;

    assert_eq!(result.status, JevStatus::Blocked, "{result:#?}");
    assert_eq!(result.stop_cause, Some(JevStopCause::Unsettled));
    assert_eq!(result.model_calls, 3);
    assert!(result.actions.is_empty());
    assert!(browser_log.lock().unwrap().acts.is_empty());
    let reason = result.stopped_because.as_deref().unwrap();
    assert!(
        reason.contains("3 decisions in a row went stale"),
        "{reason}"
    );
    // It gives the last stale message.
    assert!(
        reason.contains("Page changed since the decision. Choose again."),
        "{reason}"
    );
    assert_eq!(
        serde_json::to_value(&result).unwrap()["stop_cause"],
        json!("unsettled")
    );
}

#[tokio::test]
async fn a_stale_decision_that_leaves_a_new_page_is_not_counted() {
    // Two stale decisions on a ticking page, one on a page that really
    // changed (the words differ), two more on a ticking page, then a fresh
    // DONE: no three in a row without progress.
    let pages = vec![
        ticking(0),
        ticking(1),
        ticking(2),
        // The words differ here: the page really changed.
        page("loaded-0", &[("go", "click")]),
        page("loaded-1", &[("go", "click")]),
        page("loaded-2", &[("go", "click")]),
    ];
    let browser = ScriptedBrowser::new(pages)
        // Decisions 1 to 5 go stale at DONE; the sixth is fresh.
        .with_fresh(&[
            true, false, true, false, true, false, true, false, true, false, true, true,
        ]);

    let result = run(browser, ScriptedDecider::new(&["DONE"])).await;

    assert_eq!(result.status, JevStatus::Done, "{result:#?}");
    assert_eq!(result.stop_cause, None);
    assert_eq!(result.model_calls, 6);
}

#[tokio::test]
async fn a_recorded_step_starts_the_stale_count_again() {
    // Two stale decisions, a click that is carried out, two more stale
    // decisions, then DONE: never three in a row.
    let pages = vec![
        ticking(0),
        ticking(1),
        ticking(2),
        // After the click.
        page("after-0", &[("go", "click")]),
        page("after-1", &[("go", "click")]),
        page("after-2", &[("go", "click")]),
    ];
    let browser = ScriptedBrowser::new(pages).with_fresh(&[
        true, false, // decision 1: DONE is stale
        true, false, // decision 2: DONE is stale
        true,  // decision 3: the click goes through (a scripted act asks nothing)
        true, false, // decision 4: DONE is stale
        true, false, // decision 5: DONE is stale
        true, true, // decision 6: DONE stands
    ]);
    let browser_log = browser.log();

    let result = run(
        browser,
        ScriptedDecider::new(&["DONE", "DONE", "go", "DONE", "DONE", "DONE"]),
    )
    .await;

    assert_eq!(result.status, JevStatus::Done, "{result:#?}");
    assert_eq!(result.stop_cause, None);
    assert_eq!(result.actions.len(), 1);
    assert_eq!(result.model_calls, 6);
    assert_eq!(browser_log.lock().unwrap().acts.len(), 1);
}
