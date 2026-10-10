//! What a page held that Jev was not offered, on real pages: the snapshot
//! counts the controls its caps and its reach left out (one per control, not
//! one per action), and a call on a 300-option select tells the caller how
//! many options no choice could take.

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use super::scripted::PlanDecider;
use super::sessions::{call, test_sessions};
use crate::report::tool_text;

#[tokio::test]
async fn the_snapshot_counts_controls_it_left_out_not_actions() {
    let harness = harness_or_skip!();
    let mut page = harness.open("past-the-caps.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let kept = observation["actions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|action| action["node"].is_i64())
        .count();
    assert_eq!(kept, 250, "the snapshot keeps 250 controls");
    // Five buttons, a 12-option select and three text fields (a fill and an
    // "Open" click each) are 9 controls, though 23 actions.
    assert_eq!(observation["omitted_actions"], json!(9), "{observation:#}");
}

#[tokio::test]
async fn a_call_on_a_300_option_select_says_how_many_options_were_not_offered() {
    let harness = harness_or_skip!();
    let sessions = test_sessions();
    let args = json!({"goal": "Look at the form.", "url": harness.site.url("long-select.html")});
    let mut data = call(
        &harness,
        &sessions,
        "omitted-select",
        args,
        Arc::new(PlanDecider::new(Vec::new())),
    )
    .await
    .unwrap();
    assert_eq!(data["status"], json!("done"), "{data:#}");
    // A choice takes 255 targets; the select has 300 options.
    assert_eq!(
        data["omitted"],
        json!({"controls": 0, "options": 45}),
        "{data:#}"
    );
    let text = tool_text(&mut data);
    let header = text.split("-----").next().unwrap();
    assert!(
        header.contains("45 select options"),
        "the header names them: {text}"
    );
}

#[tokio::test]
async fn a_run_on_a_page_past_the_caps_reports_the_controls_it_was_not_offered() {
    let harness = harness_or_skip!();
    let result = harness
        .run(
            "past-the-caps.html",
            Arc::new(PlanDecider::new(Vec::new())),
            None,
            Duration::from_secs(30),
        )
        .await;
    let data = serde_json::to_value(&result).unwrap();
    assert_eq!(
        data["omitted"],
        json!({"controls": 9, "options": 0}),
        "{data:#}"
    );
}

#[tokio::test]
async fn a_page_within_the_caps_reports_nothing_omitted() {
    let harness = harness_or_skip!();
    let result = harness
        .run(
            "basic.html",
            Arc::new(PlanDecider::new(Vec::new())),
            None,
            Duration::from_secs(30),
        )
        .await;
    let data = serde_json::to_value(&result).unwrap();
    assert!(data.get("omitted").is_none(), "{data:#}");
}
