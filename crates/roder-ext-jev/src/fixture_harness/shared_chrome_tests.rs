//! One throwaway Chrome for the whole test process, and what keeps tests
//! apart on it.

use std::time::Duration;

use serde_json::{Value, json};

use super::{Harness, act_on};

/// Every target Chrome lists, over a connection of the test's own.
async fn listed(harness: &Harness) -> Value {
    let mut connection = harness.connect().await.expect("connect to Chrome");
    connection
        .call("Target.getTargets", json!({}), None)
        .await
        .expect("list targets")
}

/// The id of the window holding `target`.
async fn window_of(harness: &Harness, target: &str) -> Value {
    let mut connection = harness.connect().await.expect("connect to Chrome");
    let window = connection
        .call(
            "Browser.getWindowForTarget",
            json!({"targetId": target}),
            None,
        )
        .await
        .expect("find the target's window");
    window["windowId"].clone()
}

#[tokio::test]
async fn tests_in_one_process_share_one_chrome() {
    let first = harness_or_skip!();
    let second = harness_or_skip!();
    let mut page = first.open("basic.html").await.unwrap();

    // One browser: the second test's connection lists the first one's tab.
    let targets = listed(&second).await;
    assert!(
        targets.to_string().contains(page.target_id()),
        "the second test's Chrome does not list the first test's tab, so each started its own: \
         {targets:#}"
    );
    page.close().await.unwrap();
}

#[tokio::test]
async fn a_test_counts_only_the_tabs_it_owns() {
    let first = harness_or_skip!();
    let second = harness_or_skip!();
    let mut mine = first.open("basic.html").await.unwrap();
    let mut also_mine = first.open("landing.html").await.unwrap();
    let mut theirs = second.open("basic.html").await.unwrap();

    // The same page open in another test's tab is not this test's.
    assert_eq!(first.settled_page_targets(2).await, 2);
    assert_eq!(second.settled_page_targets(1).await, 1);
    let owned = first.owned_pages().await;
    assert!(
        owned
            .iter()
            .all(|page| page["targetId"] != json!(theirs.target_id())),
        "{owned:#?}"
    );

    mine.close().await.unwrap();
    assert_eq!(first.settled_page_targets(1).await, 1);
    assert_eq!(second.page_targets().await, 1);
    also_mine.close().await.unwrap();
    theirs.close().await.unwrap();
    assert_eq!(first.settled_page_targets(0).await, 0);
    assert_eq!(second.settled_page_targets(0).await, 0);
}

/// A page a test's tab opens is in the test's context, so it is the test's.
#[tokio::test]
async fn a_tab_a_page_opens_belongs_to_the_test_whose_page_opened_it() {
    let first = harness_or_skip!();
    let second = harness_or_skip!();
    let mut page = first.open("tabs.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let (_, opened) = act_on(&mut page, &observation, "click", "Open the Q3 report", None)
        .await
        .unwrap();
    assert_eq!(opened["opened_tab"], json!(true), "{opened:#}");

    assert_eq!(first.settled_page_targets(2).await, 2);
    assert_eq!(second.page_targets().await, 0);
    // Closing the page closes the tab it opened too.
    page.close().await.unwrap();
    assert_eq!(first.settled_page_targets(0).await, 0);
}

/// Before the shared Chrome, a test's tabs went with its Chrome, so a test
/// that failed before closing a page left nothing behind.
#[tokio::test]
async fn the_tabs_a_test_leaves_open_are_closed_when_it_ends() {
    let watching = harness_or_skip!();
    let left = {
        let leaving = harness_or_skip!();
        let first = leaving.open("basic.html").await.unwrap();
        let second = leaving.open("landing.html").await.unwrap();
        [
            first.target_id().to_string(),
            second.target_id().to_string(),
        ]
    };

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let targets = listed(&watching).await.to_string();
        if left.iter().all(|id| !targets.contains(id)) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "tabs left open by a finished test are still there: {targets}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Cookies belong to a host, not a port, so tests that each serve from their
/// own port on 127.0.0.1 would see each other's without a context apiece.
#[tokio::test]
async fn cookies_and_storage_do_not_cross_tests() {
    let first = harness_or_skip!();
    let second = harness_or_skip!();
    let mut mine = first.open("basic.html").await.unwrap();
    let mut theirs = second.open("basic.html").await.unwrap();
    mine.evaluate("document.cookie = 'jev_fixture_test=1; path=/'")
        .await
        .unwrap();
    assert_eq!(
        mine.evaluate("document.cookie").await.unwrap(),
        json!("jev_fixture_test=1")
    );
    assert_eq!(theirs.evaluate("document.cookie").await.unwrap(), json!(""));
    mine.close().await.unwrap();
    theirs.close().await.unwrap();
}

/// A window has one active tab and one focus, and tests raise their tabs, so
/// tests that shared a window failed intermittently when many ran at once.
/// Each test's tabs are in a window of the test's own.
#[tokio::test]
async fn each_test_has_a_window_of_its_own() {
    let first = harness_or_skip!();
    let second = harness_or_skip!();
    let mut mine = first.open("basic.html").await.unwrap();
    let mut also_mine = first.open("landing.html").await.unwrap();
    let mut theirs = second.open("basic.html").await.unwrap();

    let mine_window = window_of(&first, mine.target_id()).await;
    assert_eq!(
        mine_window,
        window_of(&first, also_mine.target_id()).await,
        "one test's tabs share a window"
    );
    assert_ne!(
        mine_window,
        window_of(&second, theirs.target_id()).await,
        "two tests' tabs are in one window"
    );
    mine.close().await.unwrap();
    also_mine.close().await.unwrap();
    theirs.close().await.unwrap();
}
