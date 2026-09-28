//! Cookie-banner refusal (`consent.js`) against real documents: what it
//! clicks, what it leaves alone, and what it costs on a page without a
//! banner.

use std::time::Instant;

use serde::Serialize;
use serde_json::{Value, json};

use super::write_jsonl;

#[tokio::test]
async fn a_banners_reject_button_is_clicked_once_per_document() {
    let harness = harness_or_skip!();
    let mut page = harness.open("cookie-banner.html").await.unwrap();

    let refused = page.refuse_cookie_banner().await.unwrap();
    assert_eq!(refused.as_deref(), Some("Reject all"));
    assert_eq!(
        page.evaluate("window.consent").await.unwrap(),
        json!("rejected")
    );
    assert_eq!(
        page.evaluate("window.prefs ?? false").await.unwrap(),
        json!(false)
    );
    let observation = page.observe().await.unwrap();
    assert!(
        !observation["text"]
            .as_str()
            .unwrap()
            .contains("We use cookies"),
        "{}",
        observation["text"]
    );
    // Once per document: a second check does nothing.
    assert_eq!(page.refuse_cookie_banner().await.unwrap(), None);

    // A new document is checked again.
    page.evaluate("location.reload()").await.ok();
    page.settle_page(None).await;
    assert_eq!(
        page.refuse_cookie_banner().await.unwrap().as_deref(),
        Some("Reject all")
    );
    page.close().await.ok();
}

#[tokio::test]
async fn a_banner_that_only_offers_acceptance_is_left_alone() {
    let harness = harness_or_skip!();
    let mut page = harness.open("cookie-accept-only.html").await.unwrap();
    assert_eq!(page.refuse_cookie_banner().await.unwrap(), None);
    assert_eq!(
        page.evaluate("window.consent ?? null").await.unwrap(),
        Value::Null
    );
    let text = page.observe().await.unwrap()["text"].clone();
    assert!(
        text.as_str().unwrap().contains("This site uses cookies"),
        "{text}"
    );
    page.close().await.ok();
}

#[tokio::test]
async fn a_page_without_a_banner_is_left_alone() {
    let harness = harness_or_skip!();
    let mut page = harness.open("no-banner.html").await.unwrap();
    assert_eq!(page.refuse_cookie_banner().await.unwrap(), None);
    assert_eq!(
        page.evaluate("window.declined ?? false").await.unwrap(),
        json!(false)
    );
    page.close().await.ok();
}

/// What `consent.js` pressed for one `cookie-cases.html` case.
async fn case(harness: &super::Harness, name: &str) -> (Option<String>, Value) {
    let mut page = harness
        .open(&format!("cookie-cases.html?case={name}"))
        .await
        .unwrap();
    let refused = page.refuse_cookie_banner().await.unwrap();
    let clicked = page.evaluate("window.clicked").await.unwrap();
    page.close().await.ok();
    (refused, clicked)
}

#[tokio::test]
async fn only_an_unambiguous_refusal_inside_the_banner_is_pressed() {
    let harness = harness_or_skip!();
    for name in ["two", "outside", "invite", "settings", "link", "twice"] {
        let (refused, clicked) = case(&harness, name).await;
        assert_eq!(refused, None, "{name}");
        assert_eq!(clicked, json!([]), "{name}");
    }
    for (name, label) in [
        ("german", "Alle ablehnen"),
        ("ranked", "Reject all"),
        ("shadow", "Reject all"),
        ("backdrop", "Decline"),
    ] {
        let (refused, clicked) = case(&harness, name).await;
        assert_eq!(refused.as_deref(), Some(label), "{name}");
        assert_eq!(clicked, json!([label]), "{name}");
    }
}

#[derive(Serialize)]
struct BannerCost {
    page: String,
    elements: u64,
    /// The first check of a document: the whole search.
    first_ms: f64,
    /// A later check of the same document: the once-per-document guard.
    repeat_ms: f64,
}

/// The time the check adds to a page with no banner: the first read of each
/// document pays the search, every later one a single evaluate.
#[tokio::test]
async fn the_check_is_cheap_on_a_page_without_a_banner() {
    let harness = harness_or_skip!();
    let mut rows = Vec::new();
    for name in [
        "no-banner.html",
        "basic.html",
        "big.html?n=400",
        "big.html?n=1000",
    ] {
        let mut firsts = Vec::new();
        let mut repeats = Vec::new();
        let mut elements = 0;
        for _ in 0..5 {
            let mut page = harness.open(name).await.unwrap();
            elements = page
                .evaluate("document.getElementsByTagName('*').length")
                .await
                .unwrap()
                .as_u64()
                .unwrap_or_default();
            let started = Instant::now();
            assert_eq!(page.refuse_cookie_banner().await.unwrap(), None, "{name}");
            firsts.push(started.elapsed().as_secs_f64() * 1000.0);
            let started = Instant::now();
            assert_eq!(page.refuse_cookie_banner().await.unwrap(), None, "{name}");
            repeats.push(started.elapsed().as_secs_f64() * 1000.0);
            page.close().await.ok();
        }
        rows.push(BannerCost {
            page: name.to_string(),
            elements,
            first_ms: median(firsts),
            repeat_ms: median(repeats),
        });
    }
    let path = write_jsonl("cookie_banner_cost", &rows).unwrap();
    for row in &rows {
        eprintln!(
            "{:<18} {:>6} elements: first check {:>6.1} ms, later checks {:>5.1} ms",
            row.page, row.elements, row.first_ms, row.repeat_ms
        );
    }
    eprintln!("written to {}", path.display());
    // Generous: a debug build under parallel tests, but not unbounded.
    assert!(
        rows.iter()
            .all(|row| row.first_ms < 1000.0 && row.repeat_ms < 250.0)
    );
}

fn median(mut times: Vec<f64>) -> f64 {
    times.sort_by(f64::total_cmp);
    times[times.len() / 2]
}
