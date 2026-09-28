//! DuckDuckGo's autoconsent on real documents: it refuses a known consent
//! platform's banner before Jev reads the page, it reaches tabs Jev adopts,
//! it and `consent.js` never both act on one document, and with refusal off
//! nothing is injected. Also what it costs on pages without a banner.

use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Value, json};

use super::{act_on, write_jsonl};
use crate::page::Page;

const PLATFORM: &str = "WP Cookie Notice for GDPR (autoconsent)";

/// Ask for a refusal until one is reported or `within` passes: autoconsent
/// works in the page's own time.
async fn refusal(page: &mut Page, within: Duration) -> Option<String> {
    let deadline = Instant::now() + within;
    loop {
        if let Some(label) = page.refuse_cookie_banner().await.unwrap() {
            return Some(label);
        }
        if Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn a_known_platforms_banner_is_refused_by_autoconsent() {
    let harness = harness_or_skip!();
    let mut page = harness.open_refusing("cookie-cmp.html").await.unwrap();
    let refused = refusal(&mut page, Duration::from_secs(3)).await;
    assert_eq!(refused.as_deref(), Some(PLATFORM));
    assert_eq!(
        page.evaluate("[window.clicks, window.consent]")
            .await
            .unwrap(),
        json!([["rejected"], "rejected"])
    );
    let text = page.observe().await.unwrap()["text"].clone();
    assert!(!text.as_str().unwrap().contains("uses cookies"), "{text}");
    // Reported once per document.
    assert_eq!(page.refuse_cookie_banner().await.unwrap(), None);
    page.close().await.ok();
}

#[tokio::test]
async fn without_autoconsent_that_banner_is_left_alone() {
    let harness = harness_or_skip!();
    // Refusal off: nothing injected, and "No thanks" is not a refusal
    // consent.js reads.
    let mut page = harness.open("cookie-cmp.html").await.unwrap();
    assert_eq!(page.refuse_cookie_banner().await.unwrap(), None);
    assert_eq!(
        page.evaluate("[window.clicks, typeof window.autoconsentStandalone]")
            .await
            .unwrap(),
        json!([[], "undefined"])
    );
    page.close().await.ok();
}

#[tokio::test]
async fn autoconsent_and_consent_js_never_both_act_on_a_document() {
    let harness = harness_or_skip!();
    // "Reject all" is a refusal both would press.
    for _ in 0..5 {
        let mut page = harness
            .open_refusing("cookie-cmp.html?reject=Reject%20all")
            .await
            .unwrap();
        let refused = refusal(&mut page, Duration::from_secs(3)).await;
        eprintln!("refused by {refused:?}");
        assert!(
            matches!(refused.as_deref(), Some(PLATFORM | "Reject all")),
            "{refused:?}"
        );
        // Let a late autoconsent run, then check: one click only.
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert_eq!(page.refuse_cookie_banner().await.unwrap(), None);
        assert_eq!(
            page.evaluate("window.clicks").await.unwrap(),
            json!(["rejected"])
        );
        page.close().await.ok();
    }
}

#[tokio::test]
async fn consent_js_still_refuses_a_banner_autoconsent_does_not_know() {
    let harness = harness_or_skip!();
    let mut page = harness.open_refusing("cookie-banner.html").await.unwrap();
    let refused = refusal(&mut page, Duration::from_secs(3)).await;
    assert!(refused.is_some());
    assert_eq!(
        page.evaluate("[window.consent, window.prefs ?? false]")
            .await
            .unwrap(),
        json!(["rejected", false])
    );
    page.close().await.ok();
}

/// The bundle sets its heuristics to "tier2", which also presses a lone
/// Accept or OK; Jev holds them to refusing, so a banner that only offers
/// acceptance stays up through several of its heuristic passes.
#[tokio::test]
async fn autoconsents_heuristics_never_accept() {
    let harness = harness_or_skip!();
    let mut page = harness
        .open_refusing("cookie-accept-only.html")
        .await
        .unwrap();
    assert_eq!(
        page.evaluate("window.autoconsentStandalone.instance.config.heuristicMode")
            .await
            .unwrap(),
        json!("reject")
    );
    assert_eq!(refusal(&mut page, Duration::from_millis(2500)).await, None);
    assert_eq!(
        page.evaluate("window.consent ?? null").await.unwrap(),
        Value::Null
    );
    page.close().await.ok();
}

#[tokio::test]
async fn a_tab_jev_adopts_gets_autoconsent_too() {
    let harness = harness_or_skip!();
    let mut page = harness.open_refusing("cmp-links.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let (_, opened) = act_on(
        &mut page,
        &observation,
        "click",
        "Open the allotment diary",
        None,
    )
    .await
    .unwrap();
    assert_eq!(opened["opened_tab"], json!(true), "{opened:#}");
    assert_eq!(opened["title"], json!("Allotment diary"));
    let refused = refusal(&mut page, Duration::from_secs(5)).await;
    assert_eq!(refused.as_deref(), Some(PLATFORM));
    assert_eq!(
        page.evaluate("window.clicks").await.unwrap(),
        json!(["rejected"])
    );
    page.close().await.ok();
}

#[derive(Serialize)]
struct AutoconsentCost {
    page: String,
    elements: u64,
    /// Opening the page (load and first settle) without autoconsent.
    open_ms: f64,
    /// The same with autoconsent injected.
    open_injected_ms: f64,
    /// The first refusal check and observation after opening, without it.
    first_read_ms: f64,
    /// The same with autoconsent injected.
    first_read_injected_ms: f64,
    /// A later observation, without and with it, while autoconsent's
    /// detection may still be retrying (it tries for about 10 s).
    later_read_ms: f64,
    later_read_injected_ms: f64,
}

/// Open, first read (refusal check then observation) and a later read.
async fn timings(harness: &super::Harness, name: &str, injected: bool) -> (f64, f64, f64, u64) {
    let started = Instant::now();
    let mut page = harness
        .open_url_with(&harness.site.url(name), injected)
        .await
        .unwrap();
    let open = started.elapsed();
    let started = Instant::now();
    assert_eq!(page.refuse_cookie_banner().await.unwrap(), None, "{name}");
    page.observe().await.unwrap();
    let first = started.elapsed();
    let started = Instant::now();
    page.refuse_cookie_banner().await.unwrap();
    page.observe().await.unwrap();
    let later = started.elapsed();
    let elements = page
        .evaluate("document.getElementsByTagName('*').length")
        .await
        .unwrap()
        .as_u64()
        .unwrap_or_default();
    page.close().await.ok();
    let ms = |duration: Duration| duration.as_secs_f64() * 1000.0;
    (ms(open), ms(first), ms(later), elements)
}

fn median(mut times: Vec<f64>) -> f64 {
    times.sort_by(f64::total_cmp);
    times[times.len() / 2]
}

/// What injecting autoconsent costs a page with no banner: opening it, the
/// first read and a later read, each with and without it, five times,
/// alternating. A measurement, not a check: forty page loads in parallel
/// with the suite's timing-sensitive tests made them flaky, so it runs on
/// its own:
/// `cargo test -p roder-ext-jev what_autoconsent_costs -- --ignored --nocapture --test-threads=1`.
#[tokio::test]
#[ignore = "a measurement; run on its own with --ignored"]
async fn what_autoconsent_costs_a_page_without_a_banner() {
    let harness = harness_or_skip!();
    let mut rows = Vec::new();
    for name in [
        "no-banner.html",
        "basic.html",
        "big.html?n=400",
        "big.html?n=1000",
    ] {
        let mut runs: [Vec<Value>; 2] = [Vec::new(), Vec::new()];
        for _ in 0..5 {
            for (slot, injected) in [(0, false), (1, true)] {
                let (open, first, later, elements) = timings(&harness, name, injected).await;
                runs[slot].push(json!([open, first, later, elements]));
            }
        }
        let column = |slot: usize, index: usize| {
            median(
                runs[slot]
                    .iter()
                    .map(|run| run[index].as_f64().unwrap())
                    .collect(),
            )
        };
        rows.push(AutoconsentCost {
            page: name.to_string(),
            elements: runs[0][0][3].as_u64().unwrap_or_default(),
            open_ms: column(0, 0),
            open_injected_ms: column(1, 0),
            first_read_ms: column(0, 1),
            first_read_injected_ms: column(1, 1),
            later_read_ms: column(0, 2),
            later_read_injected_ms: column(1, 2),
        });
    }
    let path = write_jsonl("autoconsent_cost", &rows).unwrap();
    for row in &rows {
        eprintln!(
            "{:<18} {:>6} elements: open {:>6.1} -> {:>6.1} ms, first read {:>6.1} -> {:>6.1} ms, \
             later read {:>6.1} -> {:>6.1} ms",
            row.page,
            row.elements,
            row.open_ms,
            row.open_injected_ms,
            row.first_read_ms,
            row.first_read_injected_ms,
            row.later_read_ms,
            row.later_read_injected_ms
        );
    }
    eprintln!("written to {}", path.display());
    // Generous: a debug build under parallel tests, but not unbounded.
    assert!(
        rows.iter()
            .all(|row| row.open_injected_ms < 10_000.0 && row.first_read_injected_ms < 5_000.0)
    );
}
