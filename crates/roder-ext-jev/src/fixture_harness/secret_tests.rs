//! Password and one-time-code fields on real pages: offered to type into,
//! checked inside the page, and never read back, even when the page later
//! reveals the field or echoes what was typed.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use super::act_on;
use super::scripted::{FieldValues, PlanDecider, find, pick};
use crate::engine::{JevActOutcome, JevStatus};

const PASSWORD: &str = "correct-horse-7431";

/// Every string the observation carries that leaves the page layer.
fn shown(observation: &Value) -> String {
    let mut observation = observation.clone();
    // The marker and guards are compared with the live page, never reported.
    for key in ["marker", "guards", "page_key", "fingerprint"] {
        observation.as_object_mut().unwrap().remove(key);
    }
    observation.to_string()
}

#[tokio::test]
async fn a_typed_password_lands_and_only_whether_it_did_comes_back() {
    let harness = harness_or_skip!();
    let mut page = harness.open("login.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let field = find(&observation, "fill", "Password").expect("password offered");
    assert_eq!(field["input_type"], json!("password"));
    assert_eq!(field["filled"], json!(false));
    assert_eq!(field["role"], json!("textbox"));

    let (outcome, next) = act_on(&mut page, &observation, "fill", "Password", Some(PASSWORD))
        .await
        .unwrap();
    assert_eq!(outcome, JevActOutcome::done());
    assert_eq!(
        page.evaluate("document.querySelector('[name=password]').value")
            .await
            .unwrap(),
        json!(PASSWORD)
    );
    // Filled, and Enter is offered in it, but nothing it holds is read.
    let field = find(&next, "fill", "Password").unwrap();
    assert_eq!(field["filled"], json!(true));
    assert!(find(&next, "enter", "Password").is_some(), "{next:#}");
    assert!(!next.to_string().contains(PASSWORD), "{next:#}");
}

#[tokio::test]
async fn a_password_the_field_cuts_short_is_refused_without_quoting_it() {
    let harness = harness_or_skip!();
    let mut page = harness.open("secrets.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let (outcome, next) = act_on(&mut page, &observation, "fill", "Short PIN", Some("98765"))
        .await
        .unwrap();
    let refused = outcome.refused.expect("a cut-short secret is refused");
    assert!(!refused.contains("9876"), "{refused}");
    assert!(refused.contains("secret field"), "{refused}");
    assert!(!next.to_string().contains("9876"), "{next:#}");
}

#[tokio::test]
async fn a_revealed_secret_stays_a_secret() {
    let harness = harness_or_skip!();
    let mut page = harness.open("secrets.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let (outcome, next) = act_on(
        &mut page,
        &observation,
        "fill",
        "New password",
        Some(PASSWORD),
    )
    .await
    .unwrap();
    assert_eq!(outcome, JevActOutcome::done());
    let (_, revealed) = act_on(&mut page, &next, "click", "Show password", None)
        .await
        .unwrap();
    assert_eq!(
        page.evaluate("document.getElementById('reveal').type")
            .await
            .unwrap(),
        json!("text")
    );
    let field = find(&revealed, "fill", "New password").unwrap();
    assert_eq!(field["input_type"], json!("password"));
    assert_eq!(field["value"], json!(""));
    // The field's value is never read; the page's own echo is page text,
    // which the loop scrubs (see `agent_secrets`), not this layer.
    let fields = revealed["actions"].to_string();
    assert!(!fields.contains(PASSWORD), "{fields}");
    assert!(shown(&revealed).contains("You typed"), "{revealed:#}");
}

/// Through the real loop: the typed password is recorded as "[secret]",
/// scrubbed from the page's echo, and appears nowhere in the result.
#[tokio::test]
async fn the_loop_records_a_typed_password_as_secret() {
    let harness = harness_or_skip!();
    let decider = Arc::new(PlanDecider::new(vec![pick("fill", "New password")]));
    let values = Arc::new(FieldValues::new(&[("New password", PASSWORD)]));
    let result = harness
        .run(
            "secrets.html",
            decider,
            Some(values),
            Duration::from_secs(20),
        )
        .await;
    assert_eq!(
        result.status,
        JevStatus::Done,
        "{:?}",
        result.stopped_because
    );
    assert_eq!(result.actions[0].text.as_deref(), Some("[secret]"));
    assert!(
        result.visible_text.contains("You typed [secret]"),
        "{}",
        result.visible_text
    );
    let everything = format!("{result:?} {}", serde_json::to_string(&result).unwrap());
    assert!(!everything.contains(PASSWORD), "{everything}");
}
