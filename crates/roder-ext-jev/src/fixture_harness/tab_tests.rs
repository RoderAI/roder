//! Following tabs an action opens (`tabs.html`, `report.html`).

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use super::act_on;
use super::scripted::{PlanDecider, pick};
use crate::engine::JevStatus;

#[tokio::test]
async fn a_tab_opened_by_a_link_or_a_script_is_followed_and_closed_with_the_page() {
    let harness = harness_or_skip!();
    let before = harness.page_targets().await;
    let mut page = harness.open("tabs.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    assert!(observation.get("opened_tab").is_none());

    // target=_blank, with and without an opener, and window.open.
    for (label, name) in [
        ("Open the Q3 report", "Q3"),
        ("Open the Q4 report", "Q4"),
        ("Open the annual report", "ANNUAL"),
    ] {
        let start = page.observe().await.unwrap();
        let start = if start["title"] == json!("Reports") {
            start
        } else {
            // Go back to the list: close the report, returning to its opener.
            let (_, back) = act_on(&mut page, &start, "click", "Close this tab", None)
                .await
                .unwrap();
            back
        };
        assert_eq!(start["title"], json!("Reports"), "{start:#}");
        let (_, opened) = act_on(&mut page, &start, "click", label, None)
            .await
            .unwrap();
        assert_eq!(opened["opened_tab"], json!(true), "{label}: {opened:#}");
        assert_eq!(opened["title"], json!(format!("Report {name}")), "{label}");
        assert!(
            opened["text"]
                .as_str()
                .unwrap()
                .contains(&format!("Total for {name}: 42 units")),
            "{opened:#}"
        );
        // The adopted tab is acted on like the first one.
        let (_, approved) = act_on(&mut page, &opened, "click", "Approve report", None)
            .await
            .unwrap();
        assert!(approved.get("opened_tab").is_none());
        assert!(
            approved["text"]
                .as_str()
                .unwrap()
                .contains("Report approved."),
            "{approved:#}"
        );
    }
    // A report that closed itself handed the run back to the list.
    let last = page.observe().await.unwrap();
    let (_, back) = act_on(&mut page, &last, "click", "Close this tab", None)
        .await
        .unwrap();
    assert_eq!(back["title"], json!("Reports"), "{back:#}");

    page.close().await.unwrap();
    assert_eq!(harness.settled_page_targets(before).await, before);
}

#[tokio::test]
async fn the_loop_records_the_new_tab_and_carries_on_there() {
    let harness = harness_or_skip!();
    let before = harness.page_targets().await;
    let decider = Arc::new(PlanDecider::new(vec![
        pick("click", "Open the Q3 report"),
        pick("click", "Approve report"),
    ]));
    let result = harness
        .run("tabs.html", decider, None, Duration::from_secs(30))
        .await;
    assert_eq!(result.status, JevStatus::Done, "{result:#?}");
    assert_eq!(result.actions.len(), 2);
    assert!(result.actions[0].opened_tab);
    assert_eq!(result.actions[0].page_changed, Some(true));
    assert!(result.actions[0].url.ends_with("report.html?name=q3"));
    assert!(!result.actions[1].opened_tab);
    assert!(result.visible_text.contains("Report approved."));
    let record = serde_json::to_value(&result.actions[0]).unwrap();
    assert_eq!(record["opened_tab"], json!(true));
    assert!(
        serde_json::to_value(&result.actions[1])
            .unwrap()
            .get("opened_tab")
            .is_none()
    );
    // Both tabs were closed with the run.
    assert_eq!(harness.settled_page_targets(before).await, before);
}

/// A tab whose setup fails is not where the run goes on: it used to become
/// the current tab before its setup ran, so a failure left the run reading a
/// half-prepared target. A shared worker can be attached to but has no
/// viewport to set, so its setup fails.
#[tokio::test]
async fn a_tab_whose_setup_fails_is_not_adopted() {
    let harness = harness_or_skip!();
    let mut page = harness.open("worker.html").await.unwrap();
    let mut connection = harness.connect().await.unwrap();
    // The worker of this test's own page: the Chrome is shared.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let worker = loop {
        let targets = connection
            .call("Target.getTargets", json!({}), None)
            .await
            .unwrap();
        let found = targets["targetInfos"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|target| {
                target["type"] == "shared_worker"
                    && target["url"]
                        .as_str()
                        .is_some_and(|url| url.contains(harness.site.origin()))
            })
            .and_then(|target| target["targetId"].as_str())
            .map(str::to_string);
        if let Some(found) = found {
            break found;
        }
        assert!(tokio::time::Instant::now() < deadline, "{targets:#}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert!(page.adopt(&worker).await.is_err());
    // The run is still on its own tab.
    let observation = page.observe().await.expect("observe the page's own tab");
    assert_eq!(observation["title"], json!("Worker host"));
    assert!(observation.get("opened_tab").is_none());
    page.close().await.ok();
}

/// Closing a tab Chrome no longer has succeeds: the tab is closed. A repeat
/// close used to fail with "No target with given id" and was reported as an
/// error although the tab had gone.
#[tokio::test]
async fn closing_a_tab_that_is_already_gone_succeeds() {
    let harness = harness_or_skip!();
    let before = harness.page_targets().await;
    let mut page = harness.open("basic.html").await.unwrap();
    let target = page.target_id().to_string();
    page.close().await.expect("close the tab");
    let mut connection = harness.connect().await.unwrap();
    crate::page::close_target(&mut connection, &target)
        .await
        .expect("close it again");
    page.close().await.expect("close the page again");
    assert_eq!(harness.settled_page_targets(before).await, before);
}
