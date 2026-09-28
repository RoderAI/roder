//! Scrolling inside a box (`regions.html`): each box on screen that scrolls
//! on its own is offered to scroll down or up, says where it is, and a
//! scroll moves only that box.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use super::act_on;
use super::scripted::{PlanDecider, find, pick};
use crate::engine::{JevActOutcome, JevStatus};
use crate::space::action_space;

fn scrolls(observation: &Value) -> Vec<(String, String, String)> {
    observation["actions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|action| action["kind"] == "scroll" && action["node"].is_i64())
        .map(|action| {
            (
                action["label"].as_str().unwrap_or_default().to_string(),
                action["direction"].as_str().unwrap_or_default().to_string(),
                action["scrolled"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

#[tokio::test]
async fn boxes_that_scroll_on_their_own_are_offered_and_scrolled() {
    let harness = harness_or_skip!();
    let mut page = harness.open("regions.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    // The terms box and the textarea, each at its top; not the box that
    // hides its overflow.
    let offered = scrolls(&observation);
    assert_eq!(
        offered,
        [
            ("Terms of service".into(), "down".into(), "top".into()),
            ("Notes".into(), "down".into(), "top".into()),
        ],
        "{offered:?}"
    );
    // They are target operations of their own; the textarea is one element
    // that can be typed into and scrolled.
    let space = action_space(observation["actions"].as_array().unwrap());
    assert_eq!(space.targets_for("SCROLL_REGION_DOWN").unwrap().len(), 2);
    let notes = space
        .elements
        .iter()
        .find(|element| element["label"] == "Notes")
        .unwrap();
    assert_eq!(
        notes["operations"],
        json!(["TYPE_TEXT", "CLICK", "SCROLL_REGION_DOWN"])
    );

    let (outcome, next) = act_on(&mut page, &observation, "scroll", "Terms of service", None)
        .await
        .unwrap();
    assert_eq!(outcome, JevActOutcome::done());
    // Only the box moved, and it can now go both ways.
    assert_eq!(
        page.evaluate("[scrollY, terms.scrollTop > 0, notes.scrollTop]")
            .await
            .unwrap(),
        json!([0, true, 0])
    );
    assert_eq!(
        scrolls(&next)[..2],
        [
            ("Terms of service".into(), "down".into(), "60%".into()),
            ("Terms of service".into(), "up".into(), "60%".into()),
        ]
    );
}

#[tokio::test]
async fn a_run_scrolls_a_box_to_its_end_to_unlock_a_button() {
    let harness = harness_or_skip!();
    let decider = Arc::new(PlanDecider::new(vec![
        pick("scroll", "Terms of service"),
        pick("scroll", "Terms of service"),
        pick("click", "Agree"),
    ]));
    let result = harness
        .run("regions.html", decider, None, Duration::from_secs(30))
        .await;
    assert_eq!(result.status, JevStatus::Done, "{result:#?}");
    assert!(result.visible_text.contains("Agreed"), "{result:#?}");
    assert!(
        result
            .actions
            .iter()
            .all(|action| action.page_changed == Some(true)),
        "{result:#?}"
    );
}

#[tokio::test]
async fn agree_is_offered_only_once_the_box_reached_its_end() {
    let harness = harness_or_skip!();
    let mut page = harness.open("regions.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    assert!(find(&observation, "click", "Agree").is_none());
    // Page text is what the box shows, not what it has scrolled out of view.
    let text = observation["text"].as_str().unwrap();
    assert!(
        text.contains("Clause 1.") && !text.contains("End of terms."),
        "{text}"
    );
    let (_, next) = act_on(&mut page, &observation, "scroll", "Terms of service", None)
        .await
        .unwrap();
    let (_, last) = act_on(&mut page, &next, "scroll", "Terms of service", None)
        .await
        .unwrap();
    assert!(find(&last, "click", "Agree").is_some(), "{last:#}");
    let text = last["text"].as_str().unwrap();
    assert!(
        !text.contains("Clause 1.") && text.contains("End of terms."),
        "{text}"
    );
    // At its end the box offers only to scroll back up.
    assert_eq!(
        scrolls(&last)[0],
        ("Terms of service".into(), "up".into(), "bottom".into())
    );
}
