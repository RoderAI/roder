//! Enter and Escape on a real DOM (`keys.html`): Enter only in the focused
//! text field once it holds text, Escape only while something is open.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use super::act_on;
use super::scripted::{FieldValues, PlanDecider, find, pick};
use crate::engine::{JevActOutcome, JevStatus};
use crate::space::action_space;

fn kinds(observation: &Value, kind: &str) -> Vec<String> {
    observation["actions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|action| action["kind"] == kind)
        .map(|action| action["label"].as_str().unwrap_or_default().to_string())
        .collect()
}

#[tokio::test]
async fn enter_is_offered_only_in_the_focused_field_once_it_holds_text() {
    let harness = harness_or_skip!();
    let mut page = harness.open("keys.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    // Nothing typed, nothing open: neither key.
    assert!(kinds(&observation, "enter").is_empty());
    assert!(kinds(&observation, "key").is_empty());

    let (_, typed) = act_on(&mut page, &observation, "fill", "Message", Some("hello"))
        .await
        .unwrap();
    assert_eq!(kinds(&typed, "enter"), ["Message"]);
    let space = action_space(typed["actions"].as_array().unwrap());
    assert_eq!(space.targets_for("PRESS_ENTER").unwrap().len(), 1);

    let (outcome, sent) = act_on(&mut page, &typed, "enter", "Message", None)
        .await
        .unwrap();
    assert_eq!(outcome, JevActOutcome::done());
    assert!(
        sent["text"].as_str().unwrap().contains("Sent: hello"),
        "{sent:#}"
    );
    // The chat cleared its field, so Enter is no longer offered.
    assert!(kinds(&sent, "enter").is_empty());
}

#[tokio::test]
async fn enter_submits_a_form_with_no_button() {
    let harness = harness_or_skip!();
    let decider = Arc::new(PlanDecider::new(vec![
        pick("fill", "Search"),
        pick("enter", "Search"),
    ]));
    let text = Arc::new(FieldValues::new(&[("Search", "trail shoes")]));
    let result = harness
        .run("keys.html", decider, Some(text), Duration::from_secs(30))
        .await;
    assert_eq!(result.status, JevStatus::Done, "{result:#?}");
    let posts = harness.site.wait_for_posts(1, Duration::from_secs(5)).await;
    assert_eq!(posts.len(), 1, "{posts:?}");
    assert_eq!(posts[0].path, "/submit/search");
    assert_eq!(posts[0].field("q").as_deref(), Some("trail shoes"));
}

#[tokio::test]
async fn escape_is_offered_while_a_list_is_open_and_closes_it() {
    let harness = harness_or_skip!();
    let mut page = harness.open("keys.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let (_, open) = act_on(&mut page, &observation, "fill", "City", Some("Par"))
        .await
        .unwrap();
    assert_eq!(
        kinds(&open, "key"),
        ["Press Escape to close the open popup, menu, suggestion list or dialog"]
    );
    let escape = find(
        &open,
        "key",
        "Press Escape to close the open popup, menu, suggestion list or dialog",
    )
    .unwrap()
    .clone();
    assert_eq!(escape["id"], "press_escape");
    let space = action_space(open["actions"].as_array().unwrap());
    assert!(space.control("PRESS_ESCAPE").is_some());
    // The open list covers the button.
    let go = find(&open, "click", "Go").unwrap().clone();
    let covered = page
        .act(&go, &open, None, Duration::from_millis(100))
        .await
        .unwrap_err();
    assert!(covered.is::<crate::engine::Covered>(), "{covered:#}");
    let open = page.observe().await.unwrap();
    let escape = find(
        &open,
        "key",
        "Press Escape to close the open popup, menu, suggestion list or dialog",
    )
    .unwrap()
    .clone();

    let outcome = page
        .act(&escape, &open, None, Duration::from_millis(100))
        .await
        .unwrap();
    assert_eq!(outcome, JevActOutcome::done());
    let closed = page.observe().await.unwrap();
    assert!(kinds(&closed, "key").is_empty());
    assert_eq!(
        page.evaluate("[suggestions.hidden, city.getAttribute('aria-expanded')]")
            .await
            .unwrap(),
        json!([true, "false"])
    );
    // The button the list covered takes a click again.
    let (outcome, clicked) = act_on(&mut page, &closed, "click", "Go", None)
        .await
        .unwrap();
    assert_eq!(outcome, JevActOutcome::done());
    assert!(
        clicked["text"]
            .as_str()
            .unwrap()
            .contains("Go clicked for Par")
    );
}
