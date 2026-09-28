//! What one observation costs on a large page (`big.html`).

use std::time::Instant;

use serde::Serialize;
use serde_json::{Value, json};

use super::write_jsonl;
use crate::page::{CONTEXT_JS, Page, SNAPSHOT_JS};

async fn median_ms(page: &mut Page, script: &str, runs: usize) -> f64 {
    let mut times = Vec::with_capacity(runs);
    for _ in 0..runs {
        let started = Instant::now();
        page.evaluate_snapshot(script).await.unwrap();
        times.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(f64::total_cmp);
    times[runs / 2]
}

#[derive(Serialize)]
struct Cost {
    cards: usize,
    elements: u64,
    actions: usize,
    snapshot_ms: f64,
    observe_ms: f64,
}

#[tokio::test]
async fn a_large_page_is_observed_quickly() {
    let harness = harness_or_skip!();
    let mut rows = Vec::new();
    for cards in [100, 400, 1000] {
        let mut page = harness.open(&format!("big.html?n={cards}")).await.unwrap();
        let observation = page.observe().await.unwrap();
        let elements = page
            .evaluate("document.getElementsByTagName('*').length")
            .await
            .unwrap()
            .as_u64()
            .unwrap_or_default();
        let snapshot_ms = median_ms(&mut page, SNAPSHOT_JS, 7).await;
        let observe_ms = median_ms(&mut page, &format!("({CONTEXT_JS})({SNAPSHOT_JS})"), 7).await;
        rows.push(Cost {
            cards,
            elements,
            actions: observation["actions"].as_array().map_or(0, Vec::len),
            snapshot_ms,
            observe_ms,
        });
        page.close().await.ok();
    }
    let path = write_jsonl("snapshot_cost", &rows).unwrap();
    eprintln!(
        "{}\nwritten to {}",
        serde_json::to_string_pretty(&json!(
            rows.iter()
                .map(|row| serde_json::to_value(row).unwrap())
                .collect::<Vec<Value>>()
        ))
        .unwrap(),
        path.display()
    );
    // Generous: a debug build under parallel tests, but not unbounded.
    assert!(rows.iter().all(|row| row.observe_ms < 3000.0));
}
