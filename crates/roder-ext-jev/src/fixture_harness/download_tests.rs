//! Downloads on real pages: a link to a file and a button that saves a blob
//! leave the page as it was, yet the click is a change, the file is saved
//! in Jev's folder and its text shows on the next read; saving the same file
//! again is no change. And the HTTP status a page reports is the one its
//! committed document was served with, read from the Network events.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use super::act_on;
use super::scripted::{PlanDecider, pick};
use super::site::ORDERS_CSV;
use crate::engine::{JevBrowser, JevStatus};

const RUN: Duration = Duration::from_secs(60);

fn saved(observation: &Value) -> String {
    let path = observation["downloads"][0]["path"]
        .as_str()
        .unwrap_or_else(|| panic!("no saved path: {observation:#}"));
    std::fs::read_to_string(path).unwrap()
}

#[tokio::test]
async fn a_link_to_a_file_is_saved_read_and_counted_as_a_change() {
    let harness = harness_or_skip!();
    let mut page = harness.open("export.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let (_, next) = act_on(&mut page, &observation, "click", "Download CSV", None)
        .await
        .unwrap();
    // The tab kept its page, and the click still changed something.
    assert!(
        next["url"]
            .as_str()
            .unwrap()
            .ends_with("/pages/export.html")
    );
    assert_ne!(next["fingerprint"], observation["fingerprint"]);
    let download = &next["downloads"][0];
    assert_eq!(download["name"], "orders.csv", "{next:#}");
    assert!(
        download["url"]
            .as_str()
            .unwrap()
            .ends_with("/files/orders.csv")
    );
    assert_eq!(download["size_bytes"], ORDERS_CSV.len());
    assert!(download.get("repeated").is_none(), "{download}");
    assert_eq!(saved(&next), ORDERS_CSV);
    let text = next["text"].as_str().unwrap();
    assert!(text.starts_with("[Downloaded file: orders.csv ("), "{text}");
    assert!(text.contains("1003,Grace Hopper,42.50"), "{text}");

    // The read after has no news, and the same fingerprint.
    let quiet = page.observe().await.unwrap();
    assert!(quiet.get("downloads").is_none());
    assert_eq!(quiet["fingerprint"], next["fingerprint"]);

    // Saved again: the same file, so no change.
    let (_, again) = act_on(&mut page, &quiet, "click", "Download CSV", None)
        .await
        .unwrap();
    assert_eq!(again["downloads"][0]["repeated"], true, "{again:#}");
    assert_eq!(again["downloads"][0]["path"], download["path"]);
    assert_eq!(again["fingerprint"], next["fingerprint"]);
    assert!(
        again["text"]
            .as_str()
            .unwrap()
            .starts_with("[Download: saved orders.csv again")
    );
    page.close().await.ok();
}

#[tokio::test]
async fn a_file_the_page_builds_is_saved_too() {
    let harness = harness_or_skip!();
    let mut page = harness.open("export.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let (_, next) = act_on(&mut page, &observation, "click", "Export CSV", None)
        .await
        .unwrap();
    let download = &next["downloads"][0];
    assert_eq!(download["name"], "orders-export.csv", "{next:#}");
    assert!(download["url"].as_str().unwrap().starts_with("blob:"));
    assert_eq!(saved(&next), ORDERS_CSV);
    assert_ne!(next["fingerprint"], observation["fingerprint"]);
    page.close().await.ok();
}

/// A script that saves through a link it never attaches, as FileSaver's
/// `saveAs` does, is waited for too.
#[tokio::test]
async fn a_file_saved_through_a_detached_link_is_saved() {
    let harness = harness_or_skip!();
    let mut page = harness.open("export.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let (_, next) = act_on(&mut page, &observation, "click", "Save notes", None)
        .await
        .unwrap();
    assert_eq!(next["downloads"][0]["name"], "notes.txt", "{next:#}");
    assert_eq!(saved(&next), "Ship by Friday.\n");
    page.close().await.ok();
}

/// A click that downloads nothing pays no download wait.
#[tokio::test]
async fn a_click_that_saves_nothing_reports_no_download() {
    let harness = harness_or_skip!();
    let mut page = harness.open("inert.html").await.unwrap();
    let mut observation = page.observe().await.unwrap();
    // The faster of two clicks, so a loaded machine's slow read is not
    // taken for the wait.
    let mut fastest = Duration::MAX;
    for _ in 0..2 {
        let started = std::time::Instant::now();
        let (_, next) = act_on(&mut page, &observation, "click", "Retry", None)
            .await
            .unwrap();
        fastest = fastest.min(started.elapsed());
        assert!(next.get("downloads").is_none(), "{next:#}");
        assert_eq!(next["fingerprint"], observation["fingerprint"]);
        observation = next;
    }
    // Well under the 1.5 s an unresolved navigation could wait.
    assert!(fastest < Duration::from_millis(1400), "{fastest:?}");
    page.close().await.ok();
}

/// The loop: one click saves the export, the step changed the page, and
/// the run's result lists the file; clicking it again changes nothing, so
/// the stall rule still ends a run that keeps saving the same file.
#[tokio::test]
async fn the_loop_records_a_download_and_stalls_on_saving_it_again() {
    let harness = harness_or_skip!();
    let decider = Arc::new(PlanDecider::new(vec![pick("click", "Download CSV")]));
    let result = harness.run("export.html", decider, None, RUN).await;
    assert_eq!(result.status, JevStatus::Done, "{result:#?}");
    assert_eq!(result.actions[0].page_changed, Some(true));
    assert_eq!(result.actions[0].downloads[0].name, "orders.csv");
    assert_eq!(result.downloads.len(), 1);
    assert!(result.visible_text.contains("1003,Grace Hopper,42.50"));
    let value = serde_json::to_value(&result).unwrap();
    assert_eq!(value["downloads"][0]["name"], json!("orders.csv"));

    let plan = (0..5).map(|_| pick("click", "Download CSV")).collect();
    let result = harness
        .run("export.html", Arc::new(PlanDecider::new(plan)), None, RUN)
        .await;
    assert_eq!(result.status, JevStatus::Blocked, "{result:#?}");
    let changed = result
        .actions
        .iter()
        .map(|action| action.page_changed)
        .collect::<Vec<_>>();
    assert_eq!(
        changed,
        [Some(true), Some(false), Some(false), Some(false)],
        "{result:#?}"
    );
    assert!(result.actions[1].downloads[0].repeated);
    assert_eq!(result.downloads.len(), 1, "{result:#?}");
}

/// The status is the committed document's, from the browser: a script
/// that hides the page's navigation entry does not change it, and a file
/// the page linked to, served 200, does not either.
#[tokio::test]
async fn the_http_status_is_the_committed_documents() {
    let harness = harness_or_skip!();
    let mut page = harness.open("export.html?status=503").await.unwrap();
    page.evaluate("performance.getEntriesByType = () => []")
        .await
        .unwrap();
    let facts = page.describe().await.unwrap().unwrap();
    assert_eq!(facts.http_status, Some(503));
    let observation = page.observe().await.unwrap();
    let (_, next) = act_on(&mut page, &observation, "click", "Download CSV", None)
        .await
        .unwrap();
    assert_eq!(next["downloads"][0]["name"], "orders.csv");
    let facts = page.describe().await.unwrap().unwrap();
    assert_eq!(facts.http_status, Some(503));
    page.goto(&harness.site.url("export.html")).await.unwrap();
    let facts = page.describe().await.unwrap().unwrap();
    assert_eq!(facts.http_status, Some(200));
    page.close().await.ok();
}
