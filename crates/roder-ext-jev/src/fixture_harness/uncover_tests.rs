//! A target covered by a popover Jev opened itself: Jev dismisses the
//! popover (Escape, its close control, or a press outside it) and presses
//! the target in the same step, with no model call. A layer it cannot tell
//! how to dismiss, a full-page backdrop that dismisses on nothing, and a
//! consent banner are left as they are, and the run stops as before.

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use super::scripted::{PlanDecider, pick};
use crate::engine::{JevStatus, JevStopCause};

/// Open the picker, pick a day (it stays open over the results), then the
/// 7:00 PM slot under it.
fn booking_plan() -> Arc<PlanDecider> {
    Arc::new(PlanDecider::new(vec![
        pick("click", "Date: Today"),
        pick("click", "28"),
        pick("click", "7:00 PM"),
    ]))
}

async fn uncovers(close: &str, how: &str) {
    let harness = harness_or_skip!();
    let page = format!("popover.html?close={close}");
    let result = harness
        .run(&page, booking_plan(), None, Duration::from_secs(30))
        .await;
    assert_eq!(result.status, JevStatus::Done, "{close}: {result:#?}");
    assert!(
        result
            .visible_text
            .contains("Selected 7:00 PM at Luna Cafe"),
        "{close}: {}",
        result.visible_text
    );
    let slot = &result.actions[2];
    assert!(!slot.covered, "{close}: {slot:#?}");
    let uncovered = slot.uncovered.as_deref().unwrap_or_default();
    assert!(uncovered.contains(how), "{close}: {uncovered:?}");
    // Three actions, three decisions and the DONE: the dismissal cost none.
    assert_eq!(result.actions.len(), 3);
    assert_eq!(result.model_calls, 4);
}

#[tokio::test]
async fn escape_closes_the_popover_jev_opened() {
    uncovers("escape", "pressed Escape").await;
}

#[tokio::test]
async fn the_popovers_close_button_closes_it() {
    uncovers("button", "clicked \"Close\"").await;
}

#[tokio::test]
async fn a_press_outside_closes_it() {
    uncovers("outside", "clicked outside").await;
}

/// Only an unlabelled icon closes it: Jev cannot tell, tries its three ways
/// once, and the covered slot stops the run as before (a fallback trigger).
#[tokio::test]
async fn a_popover_only_an_icon_closes_stays_covered() {
    let harness = harness_or_skip!();
    let plan = Arc::new(PlanDecider::new(vec![
        pick("click", "Date: Today"),
        pick("click", "28"),
        pick("click", "7:00 PM"),
        pick("click", "7:00 PM"),
        pick("click", "7:00 PM"),
    ]));
    let result = harness
        .run(
            "popover.html?close=icon",
            plan,
            None,
            Duration::from_secs(40),
        )
        .await;
    assert_eq!(result.status, JevStatus::Blocked, "{result:#?}");
    assert_eq!(result.stop_cause, Some(JevStopCause::Covered));
    assert!(!result.visible_text.contains("Selected"));
    assert!(result.actions[2..].iter().all(|step| step.covered));
    assert!(result.actions.iter().all(|step| step.uncovered.is_none()));
}

/// A backdrop over the whole page that nothing dismisses, and a consent
/// banner with refusal off, stay: nothing is pressed.
#[tokio::test]
async fn a_backdrop_and_a_consent_banner_are_left_alone() {
    let harness = harness_or_skip!();
    let plan = || {
        Arc::new(PlanDecider::new(vec![
            pick("click", "Buy now"),
            pick("click", "Buy now"),
            pick("click", "Buy now"),
        ]))
    };
    let result = harness
        .run("covered.html", plan(), None, Duration::from_secs(30))
        .await;
    assert_eq!(result.status, JevStatus::Blocked);
    assert!(result.actions.iter().all(|step| step.covered));
    let mut page = harness.open("cookie-banner.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let show = super::scripted::find(&observation, "click", "Show more")
        .unwrap()
        .clone();
    let outcome = page
        .act(&show, &observation, None, Duration::from_millis(100))
        .await;
    assert!(
        outcome
            .as_ref()
            .is_err_and(|error| error.is::<crate::engine::Covered>()),
        "{outcome:?}"
    );
    let consent = page.evaluate("window.consent ?? null").await.unwrap();
    assert_eq!(consent, json!(null), "the banner was pressed");
    page.close().await.ok();
}
