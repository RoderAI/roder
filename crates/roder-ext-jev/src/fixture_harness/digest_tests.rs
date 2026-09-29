//! The text the caller reads, measured on real pages through the session
//! layer the tool uses: a small page, two large ones, and the reservation
//! flow whose booking widget opens in a frame of another site. Each stays
//! within the digest's 8,000 characters and 120 lines; the sizes are
//! written to `target/jev-fixtures/digest_sizes.jsonl`.
//!
//! `JEV_WRITE_DIGEST_GOLDEN=1` rewrites the golden reservation digest
//! (`tests/fixtures/digest_reserve.{json,txt}`) from this run, with ports
//! and timings made stable.

use std::sync::Arc;

use serde::Serialize;
use serde_json::{Value, json};

use super::evals::{Step, StepDecider};
use super::sessions::{call, test_sessions};
use crate::report::digest;

const NOW: &str = "Mon 2026-09-28, 17:42 local time (America/Los_Angeles, UTC-07:00)";

#[derive(Serialize)]
struct Size {
    page: String,
    chars: usize,
    lines: usize,
    controls: usize,
    text_chars: usize,
}

fn plan(steps: Value) -> Arc<StepDecider> {
    let steps: Vec<Step> = serde_json::from_value(steps).unwrap();
    Arc::new(StepDecider::new(&steps))
}

fn measure(page: &str, data: &Value) -> (Size, String) {
    let text = digest(data, NOW);
    let size = Size {
        page: page.to_string(),
        chars: text.chars().count(),
        lines: text.lines().count(),
        controls: data["controls"].as_array().map_or(0, Vec::len),
        text_chars: data["visible_text"]
            .as_str()
            .map_or(0, |text| text.chars().count()),
    };
    (size, text)
}

#[tokio::test]
async fn result_text_stays_within_bounds_on_small_and_large_pages() {
    let harness = harness_or_skip!();
    let sessions = test_sessions();
    let mut sizes = Vec::new();
    for (n, page) in ["basic.html", "big.html", "long.html"].iter().enumerate() {
        let thread = format!("digest-{n}");
        let args = json!({"goal": "Look at the page.", "url": harness.site.url(page)});
        let data = call(&harness, &sessions, &thread, args, plan(json!([])))
            .await
            .unwrap();
        let (size, text) = measure(page, &data);
        assert_eq!(data["status"], json!("done"), "{text}");
        sizes.push(size);
    }

    // The reservation flow: results, then a slot in the next call.
    let results = harness
        .site
        .url("reserve-search.html?seats=3&q=Mission%20District");
    let args = json!({"goal": "Show the results.", "url": results});
    let first = call(&harness, &sessions, "reserve", args, plan(json!([])))
        .await
        .unwrap();
    let (size, text) = measure("reserve-search.html", &first);
    assert!(
        text.contains(
            "Angie's Pizza: 5:00 PM Dining Room · 5:15 PM Dining Room · 8:15 PM Dining Room"
        ),
        "{text}"
    );
    sizes.push(size);
    let args = json!({"goal": "Pick the 8:15 PM Dining Room slot at Angie's Pizza.", "url": ""});
    let slot = plan(json!([{"click": "8:15 PM Dining Room", "context": "Angie's Pizza"}]));
    let second = call(&harness, &sessions, "reserve", args, slot)
        .await
        .unwrap();
    let (size, text) = measure("reserve-search.html + slot", &second);
    assert!(
        text.contains("same tab as before (t1, continued from where call 1 ended)"),
        "{text}"
    );
    assert!(text.contains("Frame http://localhost:"), "{text}");
    assert!(text.contains("Complete Your Reservation"), "{text}");
    assert!(
        text.contains(" 1. click \"8:15 PM Dining Room\" (in: Angie's Pizza) → page changed"),
        "{text}"
    );
    sizes.push(size);

    eprintln!(
        "{:<28} {:>6} {:>6} {:>9} {:>11}",
        "page", "chars", "lines", "controls", "page text"
    );
    for size in &sizes {
        eprintln!(
            "{:<28} {:>6} {:>6} {:>9} {:>11}",
            size.page, size.chars, size.lines, size.controls, size.text_chars
        );
        assert!(size.chars <= 8_000, "{} chars on {}", size.chars, size.page);
        assert!(size.lines <= 120, "{} lines on {}", size.lines, size.page);
    }
    let path = super::write_jsonl("digest_sizes", &sizes).unwrap();
    eprintln!("wrote {}", path.display());
    if std::env::var("JEV_WRITE_DIGEST_GOLDEN").as_deref() == Ok("1") {
        write_golden(harness.site.origin(), second);
    }
}

/// Save the reservation call's data, ports and timings made stable, and
/// its digest, as the golden files `report::tests` compares against.
fn write_golden(origin: &str, data: Value) {
    let port = origin.rsplit(':').next().unwrap();
    let mut raw = data.to_string().replace(&format!(":{port}"), ":8000");
    // The browser block names the test Chrome's own port.
    let mut data: Value = serde_json::from_str(&raw).unwrap();
    data["elapsed_ms"] = json!(3200);
    data["browser"] = json!({"foreground": true, "launched_by_roder": false});
    for action in data["actions"].as_array_mut().into_iter().flatten() {
        action["elapsed_ms"] = json!(3100);
    }
    data["session"]["earlier_calls"][0]["goal"] = json!("Show the results.");
    raw = serde_json::to_string_pretty(&data).unwrap();
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    std::fs::write(dir.join("digest_reserve.json"), raw + "\n").unwrap();
    std::fs::write(dir.join("digest_reserve.txt"), digest(&data, NOW) + "\n").unwrap();
}

#[tokio::test]
async fn a_refused_page_ends_the_call_before_any_decision() {
    let harness = harness_or_skip!();
    let sessions = test_sessions();
    let args =
        json!({"goal": "Find a table.", "url": harness.site.url("access-denied.html?status=403")});
    let data = call(
        &harness,
        &sessions,
        "refused",
        args,
        plan(json!([{"click": "Search"}])),
    )
    .await
    .unwrap();
    assert_eq!(data["status"], json!("access_denied"), "{data:#}");
    assert_eq!(data["model_calls"], json!(0));
    assert_eq!(data["page"]["http_status"], json!(403));
    let text = digest(&data, NOW);
    assert!(text.starts_with("Jev: access denied."), "{text}");
    assert!(
        text.contains("Title (page-supplied): Access Denied   HTTP 403"),
        "{text}"
    );
    assert!(
        text.contains(
            "Why Jev stopped: The site refused automated access (HTTP 403, \"Access Denied\")"
        ),
        "{text}"
    );
    assert!(!text.contains("canvas drawings"), "{text}");
    // The same page served 200 with a link to act on is an ordinary page.
    let args = json!({"goal": "Look.", "url": harness.site.url("basic.html")});
    let data = call(&harness, &sessions, "refused", args, plan(json!([])))
        .await
        .unwrap();
    assert_eq!(data["status"], json!("done"), "{data:#}");
}
