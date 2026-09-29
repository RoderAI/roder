//! Frames Jev reads but cannot act in (`page/frames.rs`), and what a page
//! says of itself besides its observation (`describe`): a booking widget
//! served from another site, a sandboxed frame in the page's own process,
//! frames too small or hidden to read, and a page's HTTP status.

use serde_json::Value;

use crate::engine::JevBrowser;

fn frames(observation: &Value) -> Vec<(String, String)> {
    observation["frames"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|frame| {
            (
                frame["origin"].as_str().unwrap_or_default().to_string(),
                frame["text"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

fn offers(observation: &Value, label: &str) -> bool {
    observation["actions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action["label"] == label)
}

pub(crate) fn today_iso() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

#[tokio::test]
async fn oopif_frame_text_is_read() {
    let harness = harness_or_skip!();
    let query = format!(
        "reserve-search.html?date={}&seats=3&q=Mission%20District",
        today_iso()
    );
    let mut page = harness.open(&query).await.unwrap();
    let observation = page.observe().await.unwrap();
    assert!(frames(&observation).is_empty(), "{observation:#}");
    let slot = observation["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|action| {
            action["label"] == "8:15 PM Dining Room"
                && action["context"]
                    .as_str()
                    .is_some_and(|context| context.starts_with("Angie's Pizza"))
        })
        .cloned()
        .expect("Angie's 8:15 PM slot");
    page.act(
        &slot,
        &observation,
        None,
        std::time::Duration::from_millis(100),
    )
    .await
    .unwrap();
    // The widget loads in its frame after the click; read until it shows.
    let mut observation = page.observe().await.unwrap();
    for _ in 0..20 {
        if !frames(&observation).is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        observation = page.observe().await.unwrap();
    }
    let read = frames(&observation);
    assert_eq!(read.len(), 1, "{observation:#}");
    let (origin, text) = &read[0];
    assert!(origin.starts_with("http://localhost:"), "{origin}");
    for shown in [
        "Complete Your Reservation",
        "Angie's Pizza",
        "8:15 PM",
        "3 Guests",
    ] {
        assert!(text.contains(shown), "{shown}: {text}");
    }
    let page_text = observation["text"].as_str().unwrap();
    assert!(
        page_text.contains(&format!("[frame {origin}] Complete Your Reservation")),
        "{page_text}"
    );
    // Read, never offered: the widget's buttons stay out of reach.
    assert!(!offers(&observation, "Reserve Now"), "{observation:#}");
    assert!(!offers(&observation, "Log in"), "{observation:#}");
    assert!(offers(&observation, "Close"), "{observation:#}");
    page.close().await.unwrap();
    assert!(harness.site.posts().is_empty());
}

#[tokio::test]
async fn inprocess_frame_text_is_read() {
    let harness = harness_or_skip!();
    // Another port on 127.0.0.1: another origin, the same site.
    let other = harness.with_new_site().await;
    let src = other
        .site
        .url("reserve-widget.html?name=Kuma&date=2026-09-28&time=8:15%20PM&seats=3");
    let host = format!(
        "frame-host.html?src={}",
        src.replace('%', "%25")
            .replace('&', "%26")
            .replace('?', "%3F")
    );
    let mut page = harness.open(&host).await.unwrap();
    let mut observation = page.observe().await.unwrap();
    for _ in 0..20 {
        if !frames(&observation).is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        observation = page.observe().await.unwrap();
    }
    let read = frames(&observation);
    assert_eq!(read.len(), 1, "{observation:#}");
    assert_eq!(read[0].0, other.site.origin());
    assert!(read[0].1.contains("Complete Your Reservation"), "{read:?}");
    assert!(read[0].1.contains("3 Guests"), "{read:?}");
    assert!(!offers(&observation, "Reserve Now"), "{observation:#}");
    page.close().await.unwrap();
}

#[tokio::test]
async fn sandboxed_frame_text_is_read_and_small_or_hidden_frames_ignored() {
    let harness = harness_or_skip!();
    let mut page = harness.open("frame-sandboxed.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let read = frames(&observation);
    assert_eq!(read.len(), 1, "{observation:#}");
    assert_eq!(read[0].0, "about:srcdoc");
    assert!(
        read[0]
            .1
            .contains("Sandboxed widget: table for 2 at 7:30 PM"),
        "{read:?}"
    );
    assert!(read[0].1.contains("Widget action"), "{read:?}");
    let text = observation["text"].as_str().unwrap();
    assert!(!text.contains("Tiny frame text"), "{text}");
    assert!(!text.contains("Hidden frame text"), "{text}");
    assert!(!offers(&observation, "Widget action"), "{observation:#}");
    assert!(offers(&observation, "Page action"), "{observation:#}");
    page.close().await.unwrap();

    // `frames.html`'s cross-origin frame is 90 px tall: too small to read.
    let mut page = harness.open("frames.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    assert!(frames(&observation).is_empty(), "{observation:#}");
    page.close().await.unwrap();
}

#[tokio::test]
async fn describe_reports_the_http_status_and_headings() {
    let harness = harness_or_skip!();
    let mut page = harness.open("access-denied.html?status=403").await.unwrap();
    let facts = page.describe().await.unwrap().unwrap();
    assert_eq!(facts.http_status, Some(403));
    assert_eq!(facts.headings, ["Access Denied"]);
    page.close().await.unwrap();
    let mut page = harness.open("reserve-search.html?seats=3").await.unwrap();
    let facts = page.describe().await.unwrap().unwrap();
    assert_eq!(facts.http_status, Some(200));
    assert_eq!(
        facts.headings.first().map(String::as_str),
        Some("Marina Grill")
    );
    assert!(facts.headings.len() >= 6, "{:?}", facts.headings);
    page.close().await.unwrap();
}

#[tokio::test]
async fn frame_text_can_be_turned_off() {
    let harness = harness_or_skip!();
    let mut page = harness.open("frame-sandboxed.html").await.unwrap();
    page.set_frame_text(false);
    let observation = page.observe().await.unwrap();
    assert!(frames(&observation).is_empty(), "{observation:#}");
    page.close().await.unwrap();
}
