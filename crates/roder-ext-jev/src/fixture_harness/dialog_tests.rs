//! JavaScript dialogs on a real page: each is answered inside the DevTools
//! call that meets it, so a click whose handler calls `confirm()` returns at
//! once instead of after the 30 s call timeout, and the next observation
//! reports it before the page text.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;

use super::act_on;
use super::scripted::{PlanDecider, pick};
use crate::engine::{JevActOutcome, JevStatus};

/// Far under the 30 s a blocked renderer used to cost, and over anything a
/// busy machine takes for a click and a settle (a click and settle took
/// 10.9 s once at a load average near 60).
const ANSWERED_WITHIN: Duration = Duration::from_secs(20);

#[tokio::test]
async fn an_alert_is_accepted_and_reported_once() {
    let harness = harness_or_skip!();
    let mut page = harness.open("dialogs.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let started = Instant::now();
    let (outcome, next) = act_on(&mut page, &observation, "click", "Save", None)
        .await
        .unwrap();
    assert!(
        started.elapsed() < ANSWERED_WITHIN,
        "{:?}",
        started.elapsed()
    );
    assert_eq!(outcome, JevActOutcome::done());
    assert_eq!(
        next["dialogs"],
        json!([{"type": "alert", "message": "Saved.", "accepted": true}])
    );
    let text = next["text"].as_str().unwrap();
    assert!(
        text.starts_with("[alert dialog, accepted by Jev] Saved.\n"),
        "{text}"
    );
    // The handler ran on past the alert.
    assert!(text.contains("\nSaved\n"), "{text}");
    // The fingerprint is the page's own: the same page without the note.
    let again = page.observe().await.unwrap();
    assert!(again.get("dialogs").is_none(), "{again:#}");
    assert_eq!(again["fingerprint"], next["fingerprint"]);
}

#[tokio::test]
async fn a_confirm_and_a_prompt_are_dismissed() {
    let harness = harness_or_skip!();
    let mut page = harness.open("dialogs.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let started = Instant::now();
    let (_, next) = act_on(&mut page, &observation, "click", "Delete draft", None)
        .await
        .unwrap();
    assert!(
        started.elapsed() < ANSWERED_WITHIN,
        "{:?}",
        started.elapsed()
    );
    assert_eq!(next["dialogs"][0]["type"], "confirm");
    assert_eq!(next["dialogs"][0]["accepted"], false);
    assert_eq!(
        next["dialogs"][0]["message"],
        "Delete the draft \"Q3 notes\"? This cannot be undone."
    );
    assert_eq!(
        page.evaluate("[window.deleted ?? false, window.kept]")
            .await
            .unwrap(),
        json!([false, 1])
    );

    let (_, next) = act_on(&mut page, &next, "click", "Rename", None)
        .await
        .unwrap();
    assert_eq!(next["dialogs"][0]["type"], "prompt");
    assert_eq!(next["dialogs"][0]["accepted"], false);
    assert_eq!(
        page.evaluate("window.renamed === null").await.unwrap(),
        json!(true)
    );
}

#[tokio::test]
async fn a_dialog_opened_later_is_caught_by_the_next_call() {
    let harness = harness_or_skip!();
    let mut page = harness.open("dialogs.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let (_, first) = act_on(&mut page, &observation, "click", "Save later", None)
        .await
        .unwrap();
    // The alert opens 300 ms after the click, usually once the settle has
    // returned and nothing waits on the page; whatever call comes next
    // answers it. On a loaded machine the settle is still running and does.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let started = Instant::now();
    let later = page.observe().await.unwrap();
    assert!(
        started.elapsed() < ANSWERED_WITHIN,
        "{:?}",
        started.elapsed()
    );
    let reported = [&first, &later]
        .iter()
        .filter_map(|read| read.get("dialogs"))
        .collect::<Vec<_>>();
    assert_eq!(
        reported,
        vec![&json!([{"type": "alert", "message": "Saved later.", "accepted": true}])]
    );
    assert!(later["text"].as_str().unwrap().contains("Saved later\n"));
}

#[tokio::test]
async fn leaving_past_beforeunload_is_accepted() {
    let harness = harness_or_skip!();
    let mut page = harness.open("dialogs.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let (_, landed) = act_on(&mut page, &observation, "click", "Leave drafts", None)
        .await
        .unwrap();
    assert!(
        landed["url"]
            .as_str()
            .unwrap()
            .ends_with("/pages/landing.html"),
        "{landed:#}"
    );
    assert_eq!(landed["dialogs"][0]["type"], "beforeunload");
    assert_eq!(landed["dialogs"][0]["accepted"], true);
}

#[tokio::test]
async fn a_run_blocked_by_a_dismissed_confirm_says_so() {
    let harness = harness_or_skip!();
    let decider = Arc::new(PlanDecider::new(vec![pick("click", "Delete draft"); 5]));
    let result = harness
        .run("dialogs.html", decider, None, Duration::from_secs(30))
        .await;

    // A dismissed confirm changes nothing, so three of them stall the run.
    assert_eq!(result.status, JevStatus::Blocked, "{result:#?}");
    assert_eq!(result.actions.len(), 3);
    for action in &result.actions {
        assert_eq!(action.page_changed, Some(false), "{action:?}");
        assert_eq!(action.dialogs.len(), 1, "{action:?}");
        assert_eq!(action.dialogs[0].kind, "confirm");
        assert!(!action.dialogs[0].accepted);
    }
    let reason = result.stopped_because.as_deref().unwrap_or_default();
    assert!(
        reason.starts_with(
            "The page asked \"Delete the draft \\\"Q3 notes\\\"? This cannot be undone.\" \
             in a confirm dialog and Jev dismissed it"
        ),
        "{reason}"
    );
    // The tool result carries each step's dialogs.
    let serialized = serde_json::to_value(&result.actions[0]).unwrap();
    assert_eq!(serialized["dialogs"][0]["type"], "confirm");
}
