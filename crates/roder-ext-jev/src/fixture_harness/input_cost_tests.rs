//! What one click and one fill cost on a still page: the act alone
//! (freshness check, hit test, pointer and key input), without the settle or
//! the read after it. Rows go to `target/jev-fixtures/input_cost.jsonl`; run
//! with `--nocapture` to see the medians.

use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::json;

use super::scripted::find;
use super::{millis, write_jsonl};

const RUNS: usize = 15;

#[derive(Debug, Serialize)]
struct CostRow {
    kind: &'static str,
    run: usize,
    act_ms: f64,
}

fn median(rows: &[CostRow], kind: &str) -> f64 {
    let mut values = rows
        .iter()
        .filter(|row| row.kind == kind)
        .map(|row| row.act_ms)
        .collect::<Vec<_>>();
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

#[tokio::test]
async fn one_click_and_one_fill_cost() {
    let harness = harness_or_skip!();
    let mut page = harness.open("input-cost.html").await.unwrap();
    let mut rows = Vec::new();
    for run in 0..RUNS {
        for (kind, label, text) in [("click", "Count", None), ("fill", "Note", Some("hello"))] {
            let observation = page.observe().await.unwrap();
            let action = find(&observation, kind, label).unwrap().clone();
            let started = Instant::now();
            page.act(&action, &observation, text, Duration::from_millis(100))
                .await
                .unwrap();
            rows.push(CostRow {
                kind,
                run,
                act_ms: millis(started.elapsed()),
            });
            page.settle().await;
        }
    }
    let (click, fill) = (median(&rows, "click"), median(&rows, "fill"));
    eprintln!("input cost: click act median {click:.1} ms, fill act median {fill:.1} ms");
    write_jsonl("input_cost", &rows).unwrap();
    assert_eq!(page.evaluate("window.clicks").await.unwrap(), json!(RUNS));
    assert_eq!(
        page.evaluate("document.querySelector('input').value")
            .await
            .unwrap(),
        json!("hello")
    );
}
