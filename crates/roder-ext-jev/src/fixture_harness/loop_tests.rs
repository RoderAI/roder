//! The caps on a run going round in circles, on real pages: a menu that
//! opens and closes again (every step changes the page, so the stall rule
//! never fires), and a form with a ticker inside it (every click is stale by
//! the time it is pressed, and nothing is ever recorded). The view the
//! stale-decision cap compares is exact: a ticker the page's words do not
//! show ends the run, and a clock whose digits are on the page does not.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use super::fallback_tests::{ceilings, fall_back};
use super::scripted::{PlanDecider, pick};
use crate::engine::{JevStatus, JevStopCause};
use crate::fallback::FallbackMode;

const RUN: Duration = Duration::from_secs(30);

#[tokio::test]
async fn a_menu_that_opens_and_closes_ends_looped_before_the_sixtieth_action() {
    let harness = harness_or_skip!();
    let plan = (0..60)
        .map(|n| pick("click", ["Open menu", "Close menu"][n % 2]))
        .collect();

    let result = harness
        .run(
            "toggle-menu.html",
            Arc::new(PlanDecider::new(plan)),
            None,
            RUN,
        )
        .await;

    assert_eq!(result.status, JevStatus::Blocked, "{result:#?}");
    assert_eq!(result.stop_cause, Some(JevStopCause::Looped));
    assert_eq!(result.actions.len(), 6);
    assert_eq!(result.model_calls, 7);
    assert!(
        result
            .actions
            .iter()
            .all(|action| action.page_changed == Some(true)),
        "{result:#?}"
    );
    let reason = result.stopped_because.as_deref().unwrap();
    assert!(reason.contains("\"Open menu\""), "{reason}");
}

#[tokio::test]
async fn a_ticker_in_the_form_ends_unsettled_without_pressing_anything() {
    let harness = harness_or_skip!();
    let plan = vec![pick("click", "Renew")];
    let decider = Arc::new(PlanDecider::new(plan));

    let result = harness
        .run("ticker-form.html", decider.clone(), None, RUN)
        .await;

    assert_eq!(result.status, JevStatus::Blocked, "{result:#?}");
    assert_eq!(result.stop_cause, Some(JevStopCause::Unsettled));
    // Three decisions, each stale, and not one press.
    assert_eq!(result.model_calls, 3, "{result:#?}");
    assert!(result.actions.is_empty(), "{result:#?}");
    assert_eq!(decider.chosen.lock().unwrap().len(), 3);
    let reason = result.stopped_because.as_deref().unwrap();
    assert!(reason.contains("went stale"), "{reason}");
}

#[tokio::test]
async fn a_clock_whose_digits_are_on_the_page_is_progress_to_the_stale_cap() {
    let harness = harness_or_skip!();
    let decider = Arc::new(PlanDecider::new(vec![pick("click", "Renew")]));

    let result = harness
        .run("clock-form.html", decider.clone(), None, RUN)
        .await;

    // Every look differs from the one before in its digits alone, and the
    // click is stale at each. The cap used to treat those looks as the same
    // page and end the run at the third decision; now it does not, and the
    // run goes on until the clock holds still and the click goes through.
    assert_eq!(result.status, JevStatus::Done, "{result:#?}");
    assert_eq!(result.stop_cause, None, "{result:#?}");
    assert!(result.model_calls > 3, "{result:#?}");
    assert_eq!(result.actions.len(), 1, "{result:#?}");
    assert!(
        result.visible_text.contains("Session ends in "),
        "{result:#?}"
    );
    let chosen = decider.chosen.lock().unwrap();
    assert!(chosen.len() > 3, "{chosen:?}");
}

const WAIT: &str = "Wait for the page to update";

#[tokio::test]
async fn six_waits_that_change_nothing_end_looped() {
    let harness = harness_or_skip!();
    let plan = (0..60).map(|_| pick("wait", WAIT)).collect();

    let result = harness
        .run("inert.html", Arc::new(PlanDecider::new(plan)), None, RUN)
        .await;

    assert_eq!(result.status, JevStatus::Blocked, "{result:#?}");
    assert_eq!(result.stop_cause, Some(JevStopCause::Looped));
    assert_eq!(result.actions.len(), 6);
    assert!(
        result
            .actions
            .iter()
            .all(|action| action.page_changed == Some(false)),
        "{result:#?}"
    );
    let reason = result.stopped_because.as_deref().unwrap();
    assert!(reason.contains("waited 6 times in a row"), "{reason}");
}

#[tokio::test]
async fn waits_between_clicks_that_change_nothing_do_not_hide_the_stall() {
    let harness = harness_or_skip!();
    let plan = vec![
        pick("click", "Retry"),
        pick("wait", WAIT),
        pick("click", "Retry"),
        pick("wait", WAIT),
        pick("click", "Retry"),
    ];

    let result = harness
        .run("inert.html", Arc::new(PlanDecider::new(plan)), None, RUN)
        .await;

    assert_eq!(result.status, JevStatus::Blocked, "{result:#?}");
    assert_eq!(result.stop_cause, Some(JevStopCause::Stalled));
    assert_eq!(result.actions.len(), 5);
    assert_eq!(result.stopped_because, None);
}

/// A looped run is handed on like a stalled one: the full browser tools go on
/// in the same tab and the result says why.
#[tokio::test]
async fn a_menu_that_looped_falls_back_like_a_stall_does() {
    let harness = harness_or_skip!();
    let plan = (0..60)
        .map(|n| {
            let label = ["Open menu", "Close menu"][n % 2];
            json!({"click": label})
        })
        .collect::<Vec<_>>();

    let result = fall_back(
        &harness,
        "toggle-menu.html",
        Value::Array(plan),
        // Six presses leave the menu closed.
        json!([
            {"tool": "click", "args": {"ref_of": "Open menu"}},
            {"say": "DONE: the account menu is open"}
        ]),
        &ceilings(FallbackMode::Auto),
    )
    .await;

    assert_eq!(result["jev_status"], "blocked", "{result:#}");
    assert_eq!(result["stop_cause"], "looped", "{result:#}");
    assert_eq!(result["fallback"]["ran"], true, "{result:#}");
    assert_eq!(result["fallback"]["trigger"]["kind"], "looped");
    assert_eq!(result["status"], "done", "{result:#}");
    assert!(
        result["visible_text"].as_str().unwrap().contains("Billing"),
        "{result:#}"
    );
}
