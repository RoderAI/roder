//! What the final page held that Jev was not offered, in the run's result.
//!
//! The snapshot caps its controls (100 below the fold, 250 in all), drops
//! those no scroll reaches, and counts them in `omitted_actions`; a choice
//! takes at most 255 targets per operation, so the options of a long select
//! past that are never put to the chooser. Neither was ever reported, so a
//! caller could not tell a short list from a cut one.

mod support;

use std::sync::Arc;
use std::time::Duration;

use roder_ext_jev::{JevEngine, JevEngineConfig, JevOmitted, JevRunResult, JevStatus};
use serde_json::{Value, json};
use support::{ScriptedBrowser, ScriptedDecider, page};

async fn run(pages: Vec<Value>, choices: &[&str]) -> JevRunResult {
    let config = JevEngineConfig::new("Scripted goal", Arc::new(ScriptedDecider::new(choices)))
        .with_wait(Duration::ZERO);
    let mut engine = JevEngine::start(Box::new(ScriptedBrowser::new(pages)), config)
        .await
        .unwrap();
    engine.run(Duration::from_secs(5)).await
}

/// A native select as the snapshot reports it: one action per option, all
/// on the same node.
fn select(node: usize, field: &str, options: usize) -> Vec<Value> {
    (1..=options)
        .map(|n| {
            json!({
                "id": format!("s{node}_{n}"), "kind": "select", "role": "combobox",
                "node": node, "label": format!("{field} → Option {n}"),
                "value": format!("o{n}"), "current_value": "",
            })
        })
        .collect()
}

/// A page with one button, the given selects and `omitted_actions` as the
/// snapshot would count them.
fn observation(fingerprint: &str, selects: &[(&str, usize)], omitted: Option<u64>) -> Value {
    let mut observation = page(fingerprint, &[("go", "click")]);
    for (node, (field, options)) in selects.iter().enumerate() {
        observation["actions"]
            .as_array_mut()
            .unwrap()
            .extend(select(node + 2, field, *options));
    }
    if let Some(omitted) = omitted {
        observation["omitted_actions"] = json!(omitted);
    }
    observation
}

fn data(result: &JevRunResult) -> Value {
    serde_json::to_value(result).unwrap()
}

#[tokio::test]
async fn a_300_option_select_reports_the_options_no_choice_could_take() {
    let result = run(
        vec![observation("one", &[("Country", 300)], Some(0))],
        &["DONE"],
    )
    .await;

    assert_eq!(result.status, JevStatus::Done, "{result:#?}");
    // 255 are offered, the last 45 are not.
    assert_eq!(
        result.omitted,
        JevOmitted {
            controls: 0,
            options: 45
        }
    );
    assert_eq!(
        data(&result)["omitted"],
        json!({"controls": 0, "options": 45})
    );
}

#[tokio::test]
async fn a_second_long_select_counts_with_the_first_and_with_the_snapshots_own_cuts() {
    // 200 options fill most of the 255; the second select gets 55 of its 200.
    let result = run(
        vec![observation(
            "two",
            &[("Country", 200), ("Language", 200)],
            Some(12),
        )],
        &["DONE"],
    )
    .await;

    assert_eq!(
        data(&result)["omitted"],
        json!({"controls": 12, "options": 145})
    );
}

#[tokio::test]
async fn a_page_that_lost_nothing_has_no_omitted_field() {
    let within = run(
        vec![observation("three", &[("Country", 255)], Some(0))],
        &["DONE"],
    )
    .await;
    assert!(
        data(&within).get("omitted").is_none(),
        "{:#?}",
        data(&within)
    );

    // A page that reports no count (the scripted pages elsewhere) is the same.
    let silent = run(vec![observation("four", &[], None)], &["DONE"]).await;
    assert!(
        data(&silent).get("omitted").is_none(),
        "{:#?}",
        data(&silent)
    );
}

#[tokio::test]
async fn the_count_is_the_final_pages_not_an_earlier_ones() {
    // The first page lost seven controls; the page the click led to lost none.
    let result = run(
        vec![
            observation("before", &[], Some(7)),
            observation("after", &[], Some(0)),
        ],
        &["go", "DONE"],
    )
    .await;

    assert_eq!(result.status, JevStatus::Done, "{result:#?}");
    assert_eq!(result.actions.len(), 1);
    assert!(
        data(&result).get("omitted").is_none(),
        "{:#?}",
        data(&result)
    );
}
