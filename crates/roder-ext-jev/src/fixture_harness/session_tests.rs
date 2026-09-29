//! A thread's calls share one tab: counted in Chrome's own target list
//! across several calls (continued, navigated, a new tab, the tab cap, a
//! popup, a reset, closing, and a tab the user closed or moved).

use std::sync::Arc;

use serde_json::{Value, json};

use super::scripted::{PlanDecider, pick};
use super::sessions::{call, targets, test_sessions};
use crate::engine::JevDecisionClient;

fn done() -> Arc<dyn JevDecisionClient> {
    Arc::new(PlanDecider::new(Vec::new()))
}

fn clicks(label: &'static str) -> Arc<dyn JevDecisionClient> {
    Arc::new(PlanDecider::new(vec![pick("click", label)]))
}

fn note(result: &Value) -> &str {
    result["session"]["tab_note"].as_str().unwrap_or_default()
}

#[tokio::test]
async fn second_call_continues_in_same_tab() {
    let harness = harness_or_skip!();
    let sessions = test_sessions();
    let before = harness.page_targets().await;
    let counter = harness.site.url("counter.html");

    let first = call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Add one", "url": counter}),
        clicks("Add one"),
    )
    .await
    .unwrap();
    assert_eq!(first["status"], "done", "{first:#}");
    assert_eq!(note(&first), "new");
    assert_eq!(first["session"]["tab"], "t1");
    let tab = targets(&sessions, "t");
    assert_eq!(tab.len(), 1);
    assert_eq!(harness.settled_page_targets(before + 1).await, before + 1);

    // url "": no new tab, no reload: the count the first call left is there.
    let second = call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Add one more", "url": ""}),
        clicks("Add one"),
    )
    .await
    .unwrap();
    assert_eq!(note(&second), "continued", "{second:#}");
    assert_eq!(targets(&sessions, "t"), tab);
    assert!(
        second["visible_text"]
            .as_str()
            .unwrap()
            .contains("Count:\n2"),
        "{second:#}"
    );
    assert_eq!(second["session"]["call"], 2);
    assert_eq!(second["session"]["earlier_calls"][0]["goal"], "Add one");
    assert_eq!(second["session"]["totals"]["actions"], 2);

    // The same url again is not reloaded either.
    let third = call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Add one more", "url": counter}),
        clicks("Add one"),
    )
    .await
    .unwrap();
    assert_eq!(note(&third), "continued", "{third:#}");
    assert!(
        third["visible_text"]
            .as_str()
            .unwrap()
            .contains("Count:\n3"),
        "{third:#}"
    );
    assert_eq!(targets(&sessions, "t"), tab);
    assert_eq!(harness.page_targets().await, before + 1);
}

#[tokio::test]
async fn url_navigates_in_same_tab_and_waits_for_the_new_document() {
    let harness = harness_or_skip!();
    let sessions = test_sessions();
    let before = harness.page_targets().await;
    call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Read", "url": harness.site.url("counter.html")}),
        done(),
    )
    .await
    .unwrap();
    let tab = targets(&sessions, "t");

    // A slow page: the tab must not read the counter page as the new one.
    let slow = harness.site.url("basic.html?delay=1200");
    let moved = call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Read", "url": slow}),
        done(),
    )
    .await
    .unwrap();
    assert_eq!(note(&moved), "navigated", "{moved:#}");
    assert_eq!(moved["title"], "Basic controls", "{moved:#}");
    assert_eq!(moved["url"], slow);
    assert_eq!(targets(&sessions, "t"), tab);
    assert_eq!(harness.page_targets().await, before + 1);
}

#[tokio::test]
async fn new_tab_keeps_first_and_caps_at_three() {
    let harness = harness_or_skip!();
    let sessions = test_sessions();
    let before = harness.page_targets().await;
    let url = harness.site.url("basic.html");
    call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Read", "url": url}),
        done(),
    )
    .await
    .unwrap();
    let first = targets(&sessions, "t")[0].clone();
    for (n, page) in ["landing.html", "counter.html"].into_iter().enumerate() {
        let opened = call(
            &harness,
            &sessions,
            "t",
            json!({"goal": "Read", "url": harness.site.url(page), "tab": "new"}),
            done(),
        )
        .await
        .unwrap();
        assert_eq!(note(&opened), "new");
        assert_eq!(opened["session"]["tabs_open"], n + 2, "{opened:#}");
    }
    assert_eq!(targets(&sessions, "t")[0], first);
    assert_eq!(harness.settled_page_targets(before + 3).await, before + 3);

    // A fourth tab closes the oldest: three stay open.
    let fourth = call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Read", "url": harness.site.url("tabs.html"), "tab": "new"}),
        done(),
    )
    .await
    .unwrap();
    assert_eq!(fourth["session"]["tabs_open"], 3);
    assert_eq!(fourth["session"]["tab"], "t4");
    let open = targets(&sessions, "t");
    assert!(!open.contains(&first), "{open:?}");
    assert_eq!(harness.settled_page_targets(before + 3).await, before + 3);

    // Continuing goes on in the newest tab.
    let next = call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Read", "url": ""}),
        done(),
    )
    .await
    .unwrap();
    assert_eq!(next["title"], "Reports");
    assert_eq!(harness.page_targets().await, before + 3);
}

#[tokio::test]
async fn popup_adopted_and_kept_across_calls() {
    let harness = harness_or_skip!();
    let sessions = test_sessions();
    let before = harness.page_targets().await;
    let opened = call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Open the Q3 report", "url": harness.site.url("tabs.html")}),
        clicks("Open the Q3 report"),
    )
    .await
    .unwrap();
    assert_eq!(opened["title"], "Report Q3", "{opened:#}");
    assert_eq!(opened["session"]["tab"], "t2");
    assert_eq!(opened["session"]["tabs"][1]["opened_by"], "t1");
    assert_eq!(harness.settled_page_targets(before + 2).await, before + 2);

    let next = call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Approve it", "url": ""}),
        clicks("Approve report"),
    )
    .await
    .unwrap();
    assert_eq!(note(&next), "continued");
    assert!(
        next["visible_text"]
            .as_str()
            .unwrap()
            .contains("Report approved."),
        "{next:#}"
    );

    // The report closes itself: back on the list, the tab that opened it.
    let back = call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Close it", "url": ""}),
        clicks("Close this tab"),
    )
    .await
    .unwrap();
    assert_eq!(back["title"], "Reports", "{back:#}");
    assert_eq!(back["session"]["tab"], "t1");
    assert_eq!(harness.settled_page_targets(before + 1).await, before + 1);
}

#[tokio::test]
async fn reset_closes_and_restarts() {
    let harness = harness_or_skip!();
    let sessions = test_sessions();
    let before = harness.page_targets().await;
    call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Read", "url": harness.site.url("basic.html")}),
        done(),
    )
    .await
    .unwrap();
    call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Read", "url": harness.site.url("landing.html"), "tab": "new"}),
        done(),
    )
    .await
    .unwrap();
    let old = targets(&sessions, "t");
    let reset = call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Read", "url": harness.site.url("counter.html"), "tab": "reset"}),
        done(),
    )
    .await
    .unwrap();
    assert_eq!(note(&reset), "new");
    assert_eq!(reset["session"]["tabs_open"], 1);
    assert_eq!(reset["title"], "Counter");
    let now = targets(&sessions, "t");
    assert!(!old.contains(&now[0]));
    assert_eq!(harness.settled_page_targets(before + 1).await, before + 1);
}

#[tokio::test]
async fn close_closes_all_owned_tabs_only() {
    let harness = harness_or_skip!();
    let sessions = test_sessions();
    let unrelated = harness.open("landing.html").await.unwrap();
    let before = harness.page_targets().await;
    call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Open Q3", "url": harness.site.url("tabs.html")}),
        clicks("Open the Q3 report"),
    )
    .await
    .unwrap();
    assert_eq!(harness.settled_page_targets(before + 2).await, before + 2);

    let closed = call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "", "url": "", "tab": "close"}),
        done(),
    )
    .await
    .unwrap();
    assert_eq!(closed["status"], "closed");
    assert_eq!(closed["tabs_closed"], 2);
    assert!(sessions.peek("t").is_none());
    assert_eq!(harness.settled_page_targets(before).await, before);
    // The tab Jev never owned is still there, and a call now needs a url.
    let mut connection = harness.connect().await.unwrap();
    let listed = connection
        .call("Target.getTargets", json!({}), None)
        .await
        .unwrap();
    assert!(listed.to_string().contains(unrelated.target_id()));
    let error = call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Read", "url": ""}),
        done(),
    )
    .await
    .unwrap_err()
    .to_string();
    assert_eq!(error, crate::runner::NO_TAB_YET);
}

#[tokio::test]
async fn closed_tab_is_reopened_at_last_url() {
    let harness = harness_or_skip!();
    let sessions = test_sessions();
    let before = harness.page_targets().await;
    let url = harness.site.url("basic.html");
    call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Read", "url": url}),
        done(),
    )
    .await
    .unwrap();
    let old = targets(&sessions, "t");
    // The user closes Jev's tab between calls.
    harness.close_target(&old[0]).await.unwrap();
    assert_eq!(harness.settled_page_targets(before).await, before);

    let reopened = call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Read", "url": ""}),
        done(),
    )
    .await
    .unwrap();
    assert_eq!(note(&reopened), "reopened", "{reopened:#}");
    let detail = reopened["session"]["tab_detail"].as_str().unwrap();
    assert!(
        detail.starts_with("t1 was closed; opened a new tab at"),
        "{detail}"
    );
    assert!(
        detail.contains("anything entered there before was lost"),
        "{detail}"
    );
    assert_eq!(reopened["title"], "Basic controls");
    assert_eq!(reopened["session"]["tab"], "t2");
    assert_ne!(targets(&sessions, "t"), old);
    assert_eq!(harness.settled_page_targets(before + 1).await, before + 1);
}

#[tokio::test]
async fn a_closed_popup_returns_to_its_opener_without_reloading() {
    let harness = harness_or_skip!();
    let sessions = test_sessions();
    call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Open Q3", "url": harness.site.url("tabs.html")}),
        clicks("Open the Q3 report"),
    )
    .await
    .unwrap();
    let owned = targets(&sessions, "t");
    harness.close_target(&owned[1]).await.unwrap();

    let back = call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Read", "url": ""}),
        done(),
    )
    .await
    .unwrap();
    assert_eq!(note(&back), "reopened", "{back:#}");
    assert!(
        back["session"]["tab_detail"]
            .as_str()
            .unwrap()
            .starts_with("t2 was closed; went on in t1"),
        "{back:#}"
    );
    assert_eq!(back["title"], "Reports");
    assert_eq!(targets(&sessions, "t"), owned[..1]);
}

#[tokio::test]
async fn user_navigation_between_calls_is_reported() {
    let harness = harness_or_skip!();
    let sessions = test_sessions();
    call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Read", "url": harness.site.url("basic.html")}),
        done(),
    )
    .await
    .unwrap();
    let target = targets(&sessions, "t")[0].clone();
    // The user takes the tab somewhere else.
    let elsewhere = harness.site.url("landing.html");
    let mut connection = harness.connect().await.unwrap();
    let attached = connection
        .call(
            "Target.attachToTarget",
            json!({"targetId": target, "flatten": true}),
            None,
        )
        .await
        .unwrap();
    let session = attached["sessionId"].as_str().unwrap().to_string();
    connection
        .call("Page.navigate", json!({"url": elsewhere}), Some(&session))
        .await
        .unwrap();
    drop(connection);
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let next = call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Read", "url": ""}),
        done(),
    )
    .await
    .unwrap();
    assert_eq!(note(&next), "continued");
    assert_eq!(next["session"]["moved_to"], elsewhere, "{next:#}");
    assert_eq!(next["title"], "Landing");
    assert_eq!(targets(&sessions, "t"), [target]);
}
