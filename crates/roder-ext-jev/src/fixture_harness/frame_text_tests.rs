//! Frames Jev reads but cannot act in (`page/frames.rs`), and what a page
//! says of itself besides its observation (`describe`): a booking widget
//! served from another site, a frame of another origin in the page's own
//! process, a sandboxed frame, frames too small or hidden to read, the
//! sessions Chrome keeps for frames of another process, a frame whose
//! renderer stops answering, and a page's HTTP status.

use std::time::{Duration, Instant};

use serde_json::Value;

use crate::engine::JevBrowser;
use crate::page::{FRAME_READ, FRAME_RETRY};

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

/// Observe `page`, timed.
async fn timed_observe(page: &mut crate::page::Page) -> (Value, Duration) {
    let started = Instant::now();
    let observation = page.observe().await.unwrap();
    (observation, started.elapsed())
}

#[tokio::test]
async fn a_frame_of_another_process_is_read_on_the_session_it_was_given() {
    let harness = harness_or_skip!();
    let mut page = harness
        .open("frame-stuck.html?ad=none&order=4417")
        .await
        .unwrap();
    let mut observation = page.observe().await.unwrap();
    for _ in 0..20 {
        if !frames(&observation).is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        observation = page.observe().await.unwrap();
    }
    let read = frames(&observation);
    assert_eq!(read.len(), 1, "{observation:#}");
    assert!(read[0].0.starts_with("http://localhost:"), "{read:?}");
    assert!(read[0].1.contains("Order 4417 · Shipped"), "{read:?}");
    // Chrome attached the frame once; every read uses that session.
    let sessions = page.frame_sessions();
    assert_eq!(sessions.len(), 1, "{sessions:?}");
    let again = page.observe().await.unwrap();
    assert_eq!(frames(&again), read);
    assert_eq!(page.frame_sessions(), sessions);
    // The frame goes, and its session with it.
    page.evaluate("document.getElementById('order').remove()")
        .await
        .unwrap();
    let gone = page.observe().await.unwrap();
    assert!(frames(&gone).is_empty(), "{gone:#}");
    assert!(page.frame_sessions().is_empty());
    page.close().await.unwrap();
}

#[tokio::test]
async fn a_frame_that_stops_answering_costs_its_deadline_once_then_the_short_one() {
    let harness = harness_or_skip!();
    let mut page = harness.open("frame-stuck.html").await.unwrap();
    // The advertisement starts spinning 50 ms after it loads.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(page.frame_sessions().len(), 1);
    let (observation, first) = timed_observe(&mut page).await;
    assert!(frames(&observation).is_empty(), "{observation:#}");
    assert!(offers(&observation, "Subscribe"), "{observation:#}");
    assert!(
        first >= FRAME_READ && first < FRAME_READ + Duration::from_secs(3),
        "first read took {first:?}"
    );
    for _ in 0..2 {
        let (observation, later) = timed_observe(&mut page).await;
        assert!(frames(&observation).is_empty(), "{observation:#}");
        assert!(
            later >= FRAME_RETRY && later < FRAME_RETRY + Duration::from_secs(2),
            "a later read took {later:?}"
        );
    }
    page.close().await.unwrap();
}

#[tokio::test]
async fn a_live_frame_is_read_beside_one_that_stops_answering() {
    let harness = harness_or_skip!();
    let mut page = harness
        .open("frame-stuck.html?ad=sandboxed&order=4417")
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (observation, first) = timed_observe(&mut page).await;
    assert!(first < FRAME_READ + Duration::from_secs(3), "{first:?}");
    let read = frames(&observation);
    assert_eq!(read.len(), 1, "{observation:#}");
    assert!(read[0].1.contains("Order 4417 · Shipped"), "{read:?}");
    assert!(
        observation["text"]
            .as_str()
            .unwrap()
            .contains("Order 4417 · Shipped"),
        "{observation:#}"
    );
    let (again, later) = timed_observe(&mut page).await;
    assert_eq!(frames(&again), read);
    assert!(later < FRAME_RETRY + Duration::from_secs(2), "{later:?}");
    page.close().await.unwrap();
}
