//! Real-DOM assertions against the page layer and the loop.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use super::scripted::{FieldValues, PlanDecider, find, pick};
use super::{timed_step, write_jsonl};
use crate::engine::{Covered, JevStatus, StaleObservation};

fn labels(observation: &Value) -> Vec<(String, String, String)> {
    observation["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|action| {
            (
                action["id"].as_str().unwrap_or_default().to_string(),
                action["kind"].as_str().unwrap_or_default().to_string(),
                action["label"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

fn stale_message(error: &anyhow::Error) -> String {
    assert!(
        error.is::<StaleObservation>(),
        "expected a stale page: {error:#}"
    );
    error.to_string()
}

#[tokio::test]
async fn observes_actionable_controls_with_stable_ids() {
    let harness = harness_or_skip!();
    let mut page = harness.open("basic.html").await.unwrap();
    let observation = page.observe().await.unwrap();

    let owned = labels(&observation);
    let seen = owned
        .iter()
        .map(|(id, kind, label)| (id.as_str(), kind.as_str(), label.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        seen,
        vec![
            ("e1", "click", "Read the docs"),
            ("e2", "click", "Save draft"),
            ("e3", "fill", "Full name"),
            ("e4", "click", "Open Full name"),
            ("e5", "click", "Newsletter"),
            ("e6", "fill", "Secret"),
            ("e7", "click", "Open Secret"),
            ("wait", "wait", "Wait for the page to update"),
        ],
        "{observation:#}"
    );
    // A password field is offered, saying only that it is one and empty.
    let secret = &observation["actions"][5];
    assert_eq!(secret["input_type"], json!("password"));
    assert_eq!(secret["filled"], json!(false));
    // Disabled, hidden, transparent and aria-hidden controls are never
    // offered.
    for skipped in ["Hidden action", "Transparent action"] {
        assert!(
            !seen.iter().any(|(_, _, label)| label.contains(skipped)),
            "{skipped}"
        );
    }
    assert!(
        !seen
            .iter()
            .any(|(_, _, label)| label.contains("Aria hidden"))
    );
    assert!(
        !observation["text"]
            .as_str()
            .unwrap()
            .contains("Aria hidden")
    );
    assert_eq!(observation["title"], json!("Basic controls"));
    assert_eq!(observation["fingerprint"].as_str().unwrap().len(), 64);
    assert!(page.fresh(&observation, None).await.unwrap());

    // Ids are positional and node identity is kept across observations.
    let again = page.observe().await.unwrap();
    assert_eq!(again["actions"], observation["actions"]);
    assert_eq!(again["fingerprint"], observation["fingerprint"]);
}

#[tokio::test]
async fn a_covered_button_is_refused_without_a_click() {
    let harness = harness_or_skip!();
    let mut page = harness.open("covered.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    // Snapshot does not hit-test, so the covered button is still offered.
    let button = find(&observation, "click", "Buy now")
        .expect("covered button observed")
        .clone();

    let error = page
        .act(&button, &observation, None, Duration::from_millis(100))
        .await
        .unwrap_err();
    // Covered, not stale: the loop records it as a step that changed nothing.
    assert!(error.is::<Covered>(), "{error:#}");
    assert_eq!(
        error.to_string(),
        "Another element covers the target; nothing was clicked."
    );
    assert_eq!(page.evaluate("window.clicks ?? 0").await.unwrap(), json!(0));
}

#[tokio::test]
async fn selecting_an_option_fires_change() {
    let harness = harness_or_skip!();
    let mut page = harness.open("select.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let options = labels(&observation)
        .into_iter()
        .filter(|(_, kind, _)| kind == "select")
        .map(|(_, _, label)| label)
        .collect::<Vec<_>>();
    // The current option is offered too, so a goal asking for it can be
    // met; the disabled one is not.
    assert_eq!(
        options,
        vec!["Colour → Red", "Colour → Green", "Colour → Blue"]
    );

    let blue = find(&observation, "select", "Colour → Blue")
        .unwrap()
        .clone();
    let (next, timing) = timed_step(&mut page, &observation, &blue, None)
        .await
        .unwrap();
    assert_eq!(
        page.evaluate("window.changes").await.unwrap(),
        json!(["blue"])
    );
    assert!(
        next["text"].as_str().unwrap().contains("Changed to blue"),
        "{next:#}"
    );
    assert_ne!(next["fingerprint"], observation["fingerprint"]);
    // The select now shows Blue as its current value, among the same options.
    let blue = find(&next, "select", "Colour → Blue").unwrap();
    assert_eq!(blue["current_value"], "Blue");
    assert!(find(&next, "select", "Colour → Red").is_some());

    assert!(timing.settle_ms >= 0.0 && timing.act_ms > 0.0, "{timing:?}");
    write_jsonl("select_timing", &[timing]).unwrap();
}

#[tokio::test]
async fn filled_text_reaches_the_form_post() {
    let harness = harness_or_skip!();
    let decider = Arc::new(PlanDecider::new(vec![
        pick("fill", "Name"),
        pick("fill", "Email"),
        pick("fill", "Message"),
        pick("click", "Send message"),
    ]));
    let text = Arc::new(FieldValues::new(&[
        ("Name", "Ada Lovelace"),
        ("Email", "ada@example.com"),
        ("Message", "Hello & welcome"),
    ]));
    let result = harness
        .run(
            "contact.html",
            decider.clone(),
            Some(text),
            Duration::from_secs(30),
        )
        .await;

    assert_eq!(result.stopped_because, None, "{result:#?}");
    assert_eq!(result.status, JevStatus::Done, "{result:#?}");
    assert_eq!(result.text_calls, 3);
    // One decision per step plus DONE: the submit's navigation costs no
    // extra model call.
    assert_eq!(
        result.model_calls,
        5,
        "{:?}",
        decider.chosen.lock().unwrap()
    );
    let kinds = result
        .actions
        .iter()
        .map(|action| (action.kind.as_str(), action.text.as_deref()))
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            ("fill", Some("Ada Lovelace")),
            ("fill", Some("ada@example.com")),
            ("fill", Some("Hello & welcome")),
            ("click", None),
        ]
    );
    assert!(result.url.ends_with("/submit/contact"), "{}", result.url);
    assert_eq!(result.title, "Thanks");

    let posts = harness.site.wait_for_posts(1, Duration::from_secs(5)).await;
    assert_eq!(posts.len(), 1, "{posts:?}");
    let post = &posts[0];
    assert_eq!(post.path, "/submit/contact");
    assert_eq!(post.field("name").as_deref(), Some("Ada Lovelace"));
    assert_eq!(post.field("email").as_deref(), Some("ada@example.com"));
    assert_eq!(post.field("message").as_deref(), Some("Hello & welcome"));

    let rows = result
        .actions
        .iter()
        .map(|action| {
            json!({
                "step": action.step, "kind": action.kind, "action": action.action,
                "executed_ms": action.executed_ms, "elapsed_ms": action.elapsed_ms,
                "page_changed": action.page_changed,
            })
        })
        .collect::<Vec<_>>();
    write_jsonl("contact_run", &rows).unwrap();
}

#[tokio::test]
async fn a_navigation_between_decision_and_act_is_stale() {
    let harness = harness_or_skip!();
    let mut page = harness
        .open("navigate.html?observed_after=300")
        .await
        .unwrap();
    let observation = page.observe().await.unwrap();
    let button = find(&observation, "click", "Stay here")
        .expect("observed before the page navigates")
        .clone();
    assert!(
        observation["url"]
            .as_str()
            .unwrap()
            .contains("navigate.html")
    );

    // The page leaves on its own timer, after the observation.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(path) = page.evaluate("location.pathname").await
            && path == json!("/pages/landing.html")
            && page.evaluate("document.readyState").await.ok() == Some(json!("complete"))
        {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "page never navigated"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    assert!(!page.fresh(&observation, None).await.unwrap());
    assert!(!page.fresh(&observation, Some(&button)).await.unwrap());
    let error = page
        .act(&button, &observation, None, Duration::from_millis(100))
        .await
        .unwrap_err();
    assert_eq!(
        stale_message(&error),
        "Page changed since this decision. Observe again."
    );
    let landed = page.observe().await.unwrap();
    assert!(landed["text"].as_str().unwrap().contains("You arrived."));
}
