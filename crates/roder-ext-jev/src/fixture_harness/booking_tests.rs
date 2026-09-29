//! Causes the live booking benchmark (`scripts/jev-booking-bench.sh`) found,
//! reproduced on fixture pages through the real session layer.

use std::sync::Arc;

use serde_json::json;

use super::scripted::PlanDecider;
use super::sessions::{call, targets, test_sessions};
use crate::engine::JevDecisionClient;

fn done() -> Arc<dyn JevDecisionClient> {
    Arc::new(PlanDecider::new(Vec::new()))
}

/// Run 1: the first site refused access, and the caller asked for a new tab
/// for the next site, which left the refused page open beside it. A new tab
/// asked for after a refused page loads in that tab instead.
#[tokio::test]
async fn a_new_tab_after_a_refused_page_reuses_its_tab() {
    let harness = harness_or_skip!();
    let sessions = test_sessions();
    let before = harness.page_targets().await;
    let refused = call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Find a table", "url": harness.site.url("access-denied.html?status=403")}),
        done(),
    )
    .await
    .unwrap();
    assert_eq!(refused["status"], "access_denied", "{refused:#}");
    let tab = targets(&sessions, "t");

    let next = harness.site.url("reserve.html");
    let moved = call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Find a table", "url": next, "tab": "new"}),
        done(),
    )
    .await
    .unwrap();
    assert_eq!(moved["status"], "done", "{moved:#}");
    assert_eq!(moved["session"]["tab_note"], "navigated", "{moved:#}");
    assert_eq!(moved["session"]["tab"], "t1", "{moved:#}");
    assert!(
        moved["session"]["tab_detail"]
            .as_str()
            .unwrap()
            .contains("refused access"),
        "{moved:#}"
    );
    assert_eq!(moved["session"]["tabs_open"], 1);
    assert_eq!(targets(&sessions, "t"), tab);
    assert_eq!(harness.page_targets().await, before + 1);

    // A page that did not refuse access keeps its tab: `new` opens a second.
    let beside = call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Read", "url": harness.site.url("basic.html"), "tab": "new"}),
        done(),
    )
    .await
    .unwrap();
    assert_eq!(beside["session"]["tab_note"], "new", "{beside:#}");
    assert_eq!(beside["session"]["tabs_open"], 2, "{beside:#}");
}

/// Run 5: the result grouped a results map's links under the last
/// restaurant card of the list before it, as if they were that restaurant's
/// options. A heading names the controls of its own item, not controls
/// after the list the item sits in.
#[tokio::test]
async fn a_control_after_a_list_of_cards_is_not_the_last_cards() {
    let harness = harness_or_skip!();
    let sessions = test_sessions();
    let data = call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Look at the results", "url": harness.site.url("reserve-search.html?seats=3")}),
        done(),
    )
    .await
    .unwrap();
    let section = |label: &str| {
        data["controls"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|control| control["label"] == label)
            .map(|control| control["section"].as_str().unwrap_or_default().to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(section("Load more"), [""], "{data:#}");
    assert_eq!(section("9:00 PM Bar"), ["Kuma on Valencia"], "{data:#}");
    assert!(
        section("8:15 PM Dining Room").contains(&"Angie's Pizza".to_string()),
        "{data:#}"
    );
    // The text groups them the same way.
    let text = crate::report::digest(&data, "Mon 2026-09-28, 17:42 local time");
    assert!(!text.contains("Tartine Supper: Load more"), "{text}");
}

/// Run 6: loading the next site in the tab that showed a refused page came
/// back `net::ERR_ABORTED`, and the call ended `blocked` at once, although
/// the same address loaded in a fresh tab. A reused tab tries an abandoned
/// load once more; a fresh tab, where nothing else can abandon it, does not.
#[tokio::test]
async fn a_load_abandoned_once_in_a_reused_tab_is_tried_again() {
    let harness = harness_or_skip!();
    let sessions = test_sessions();
    let first = call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Read", "url": harness.site.url("basic.html")}),
        done(),
    )
    .await
    .unwrap();
    assert_eq!(first["status"], "done", "{first:#}");

    let flaky = harness.site.path_url("/fail/abort-once/counter.html");
    let moved = call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Read", "url": flaky}),
        done(),
    )
    .await
    .unwrap();
    assert_eq!(moved["status"], "done", "{moved:#}");
    assert_eq!(moved["session"]["tab_note"], "navigated", "{moved:#}");
    assert!(
        moved["visible_text"].as_str().unwrap().contains("Count"),
        "{moved:#}"
    );
    assert_eq!(harness.site.failure_hits("/fail/abort-once/counter"), 2);

    // A load abandoned every time is tried twice, then reported.
    let dead = call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Read", "url": harness.site.path_url("/fail/no-content")}),
        done(),
    )
    .await
    .unwrap();
    assert_eq!(dead["status"], "blocked", "{dead:#}");
    assert_eq!(dead["stopped_because"], "could not load (net::ERR_ABORTED)");
    assert_eq!(harness.site.failure_hits("/fail/no-content"), 2);

    // A fresh tab does not retry it.
    let fresh = call(
        &harness,
        &sessions,
        "fresh",
        json!({"goal": "Read", "url": harness.site.path_url("/fail/abort-once/basic.html")}),
        done(),
    )
    .await
    .unwrap();
    assert_eq!(fresh["status"], "blocked", "{fresh:#}");
    assert_eq!(harness.site.failure_hits("/fail/abort-once/basic"), 1);
}
