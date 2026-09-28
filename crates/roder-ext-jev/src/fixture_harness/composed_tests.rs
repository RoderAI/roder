//! Controls inside open shadow roots (`shadow.html`) and same-origin frames
//! (`frames.html`): offered, named, read as page text, and clickable and
//! fillable where they are. Closed shadow roots and cross-origin frames stay
//! out of reach.

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use super::act_on;
use super::scripted::{FieldValues, PlanDecider, find, pick};
use crate::engine::{JevActOutcome, JevStatus};

fn labels(observation: &Value) -> Vec<String> {
    observation["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|action| {
            format!(
                "{} {}",
                action["kind"].as_str().unwrap_or_default(),
                action["label"].as_str().unwrap_or_default()
            )
        })
        .collect()
}

#[tokio::test]
async fn shadow_controls_are_offered_named_and_acted_on() {
    let harness = harness_or_skip!();
    let mut page = harness.open("shadow.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let offered = labels(&observation);
    // Both cards' buttons, told apart by their card's heading.
    let carts = observation["actions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|action| action["label"] == "Add to cart")
        .map(|action| action["context"].as_str().unwrap_or_default().to_string())
        .collect::<Vec<_>>();
    assert_eq!(carts.len(), 2, "{offered:?}");
    assert!(carts[0].starts_with("Desk Lamp"), "{carts:?}");
    assert!(carts[1].starts_with("Trail Runner"), "{carts:?}");
    // A control's own shadow inside is not offered again; a nested
    // component is named by what its shadow root shows.
    assert_eq!(
        offered.iter().filter(|label| label.contains("★")).count(),
        0,
        "{offered:?}"
    );
    assert!(
        offered.contains(&"click Favourite".to_string()),
        "{offered:?}"
    );
    assert!(offered.contains(&"fill Email".to_string()), "{offered:?}");
    assert!(
        offered.contains(&"click Sign up".to_string()),
        "{offered:?}"
    );
    assert!(
        !offered.iter().any(|label| label.contains("Closed action")),
        "{offered:?}"
    );
    // Shadow text is page text, where it is drawn.
    let text = observation["text"].as_str().unwrap();
    assert!(text.contains("Desk Lamp") && text.contains("$90"), "{text}");
    assert!(!text.contains("Closed action"), "{text}");

    let lamp = observation["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|action| {
            action["label"] == "Add to cart"
                && action["context"]
                    .as_str()
                    .is_some_and(|context| context.starts_with("Desk Lamp"))
        })
        .unwrap()
        .clone();
    let outcome = page
        .act(&lamp, &observation, None, Duration::from_millis(100))
        .await
        .unwrap();
    assert_eq!(outcome, JevActOutcome::done());
    let after = page.observe().await.unwrap();
    assert!(
        after["text"].as_str().unwrap().contains("Cart: Desk Lamp"),
        "{after:#}"
    );
    let (_, favourited) = act_on(&mut page, &after, "click", "Favourite", None)
        .await
        .unwrap();
    assert!(
        favourited["text"].as_str().unwrap().contains("Favourited."),
        "{favourited:#}"
    );
    page.close().await.ok();
}

#[tokio::test]
async fn a_form_two_shadow_roots_deep_is_filled_and_sent() {
    let harness = harness_or_skip!();
    let decider = Arc::new(PlanDecider::new(vec![
        pick("fill", "Email"),
        pick("click", "Sign up"),
    ]));
    let text = Arc::new(FieldValues::new(&[("Email", "ada@example.com")]));
    let result = harness
        .run("shadow.html", decider, Some(text), Duration::from_secs(30))
        .await;
    assert_eq!(result.status, JevStatus::Done, "{result:#?}");
    assert!(
        result.visible_text.contains("Signed up: ada@example.com"),
        "{}",
        result.visible_text
    );
    assert_eq!(result.actions[0].refused, None);
}

#[tokio::test]
async fn same_origin_frames_are_offered_in_place_and_cross_origin_ones_are_not() {
    let harness = harness_or_skip!();
    let mut page = harness.open("frames.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let offered = labels(&observation);
    for expected in [
        "fill Name",
        "fill Email",
        "fill Start date",
        "click Subscribe",
        "click Accept terms",
    ] {
        assert!(
            offered.contains(&expected.to_string()),
            "{expected}: {offered:?}"
        );
    }
    assert!(
        !offered.iter().any(|label| label.contains("Remote button")),
        "{offered:?}"
    );
    let text = observation["text"].as_str().unwrap();
    assert!(text.contains("Join our newsletter"), "{text}");
    // Placed in the window: inside the frame's border and padding (8 px
    // margin, 4 px border, 6 px padding), and halved by the scaled frame.
    let subscribe = find(&observation, "click", "Subscribe").unwrap();
    assert!(
        subscribe["rect"]["x"].as_f64().unwrap() >= 18.0,
        "{subscribe}"
    );
    let terms = find(&observation, "click", "Accept terms").unwrap().clone();
    let form = find(&observation, "fill", "Name").unwrap();
    assert!(
        terms["rect"]["y"].as_f64().unwrap() > form["rect"]["y"].as_f64().unwrap() + 200.0,
        "{terms} {form}"
    );
    assert!(terms["rect"]["h"].as_f64().unwrap() < 40.0, "{terms}");

    // The scaled frame's button is hit where it is drawn.
    let outcome = page
        .act(&terms, &observation, None, Duration::from_millis(100))
        .await
        .unwrap();
    assert_eq!(outcome, JevActOutcome::done());
    let after = page.observe().await.unwrap();
    assert!(
        after["text"]
            .as_str()
            .unwrap()
            .contains("Accept terms clicked."),
        "{after:#}"
    );
    page.close().await.ok();
}

#[tokio::test]
async fn a_form_in_a_frame_is_filled_and_submitted() {
    let harness = harness_or_skip!();
    let decider = Arc::new(PlanDecider::new(vec![
        pick("fill", "Name"),
        pick("fill", "Email"),
        pick("fill", "Start date"),
        pick("click", "Subscribe"),
    ]));
    let text = Arc::new(FieldValues::new(&[
        ("Name", "Ada Lovelace"),
        ("Email", "ada@example.com"),
        ("Start date", "2026-10-12"),
    ]));
    let result = harness
        .run("frames.html", decider, Some(text), Duration::from_secs(30))
        .await;
    assert_eq!(result.status, JevStatus::Done, "{result:#?}");
    assert!(
        result.actions.iter().all(|action| action.refused.is_none()),
        "{result:#?}"
    );
    assert!(
        result
            .visible_text
            .contains("Subscribed: Ada Lovelace <ada@example.com>"),
        "{}",
        result.visible_text
    );
}
