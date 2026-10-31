//! The text helper against the configured live text model: relative dates.
//!
//! Opt-in and `#[ignore]`d, like the live eval tiers. Each case is a native
//! date field whose goal names a day relative to today ("tomorrow", "next
//! Monday"), asked as of each day of one fixed week, so the answer is known
//! and the run does not depend on the clock. It reports how many values
//! were right and the text-call latency, and writes one row per call to
//! `$CARGO_TARGET_DIR/jev-evals/text-dates-<unix>.jsonl`.
//!
//! ```text
//! OPENROUTER_API_KEY=… JEV_TEXT_DATES_N=2 cargo test -p roder-ext-jev --lib \
//!   relative_dates_live -- --ignored --nocapture
//! ```

use std::time::Instant;

use chrono::{Datelike, Duration, NaiveDate, Weekday};
use serde_json::{Value, json};

use super::{TextHelper, field_context};
use crate::engine::JevTextValueResolver;

/// A goal and the day it names, as of `today`.
struct Case {
    goal: &'static str,
    day: fn(NaiveDate) -> NaiveDate,
}

/// The first `weekday` strictly after `today`.
fn next(today: NaiveDate, weekday: Weekday) -> NaiveDate {
    today
        .iter_days()
        .skip(1)
        .find(|day| day.weekday() == weekday)
        .expect("a day within a week")
}

const CASES: [Case; 3] = [
    Case {
        goal: "Book a table for two for tomorrow.",
        day: |today| today + Duration::days(1),
    },
    Case {
        goal: "Book a table for two for next Monday.",
        day: |today| next(today, Weekday::Mon),
    },
    Case {
        goal: "Book a table for two for the coming Friday.",
        day: |today| next(today, Weekday::Fri),
    },
];

fn percentile(sorted: &[u64], share: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = ((sorted.len() as f64 * share).ceil() as usize).clamp(1, sorted.len());
    sorted[rank - 1]
}

#[tokio::test]
#[ignore = "calls the live text model; needs OPENROUTER_API_KEY or JEV_TEXT_MODEL_API_KEY"]
async fn relative_dates_live() {
    let model = crate::runner::resolve_text_model(None)
        .await
        .unwrap()
        .expect("no text model is configured");
    let label = model.model.clone();
    let helper = TextHelper::new(model);
    let runs = std::env::var("JEV_TEXT_DATES_N")
        .ok()
        .and_then(|raw| raw.parse::<usize>().ok())
        .unwrap_or(1)
        .max(1);
    let week = NaiveDate::from_ymd_opt(2026, 9, 27).unwrap();
    let page = json!({"title": "Book a table", "text": "Book a table\nDate\nGuests\nBook"});
    let field = json!({"label": "Date", "role": "textbox", "value": "", "input_type": "date"});
    let mut rows = Vec::new();
    for _ in 0..runs {
        for today_is in week.iter_days().take(7) {
            for case in &CASES {
                let context = field_context(case.goal, &field, &page, &[], today_is);
                let expected = (case.day)(today_is).format("%Y-%m-%d").to_string();
                let started = Instant::now();
                let written = helper.resolve(&context).await;
                let latency_ms = started.elapsed().as_millis() as u64;
                let (value, error) = match written {
                    Ok(written) => (Some(written.value), None),
                    Err(error) => (None, Some(error.to_string())),
                };
                rows.push(json!({
                    "today": today_is.format("%Y-%m-%d (%A)").to_string(),
                    "goal": case.goal,
                    "expected": expected,
                    "value": value,
                    "error": error,
                    "correct": value.as_deref() == Some(expected.as_str()),
                    "latency_ms": latency_ms,
                }));
            }
        }
    }
    let correct = rows.iter().filter(|row| row["correct"] == true).count();
    let mut latencies = rows
        .iter()
        .filter_map(|row| row["latency_ms"].as_u64())
        .collect::<Vec<_>>();
    latencies.sort_unstable();
    let summary = json!({
        "model": label,
        "calls": rows.len(),
        "correct": correct,
        "latency_p50_ms": percentile(&latencies, 0.5),
        "latency_p95_ms": percentile(&latencies, 0.95),
        "wrong": rows.iter().filter(|row| row["correct"] != true).cloned().collect::<Vec<Value>>(),
    });
    let dir = std::path::PathBuf::from(
        std::env::var("CARGO_TARGET_DIR")
            .unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../target").into()),
    )
    .join("jev-evals");
    std::fs::create_dir_all(&dir).unwrap();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let path = dir.join(format!("text-dates-{stamp}.jsonl"));
    let lines = rows.iter().map(Value::to_string).collect::<Vec<_>>();
    std::fs::write(&path, lines.join("\n") + "\n").unwrap();
    println!("rows: {}", path.display());
    println!("{}", serde_json::to_string_pretty(&summary).unwrap());
}
