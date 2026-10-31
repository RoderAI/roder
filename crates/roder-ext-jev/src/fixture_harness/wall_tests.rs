//! Sign-in walls on real pages, read as the tool returns them: a run that
//! reaches a sign-in it cannot make ends `needs_login` with a typed wall, is
//! a handoff rather than a failed tool call, and never falls back to a
//! driver that would try the sign-in anyway.

use std::sync::Arc;

use roder_api::tools::{ToolCall, ToolResult};
use serde_json::{Value, json};

use super::Harness;
use super::evals::TaskValues;
use super::fallback_tests::{ceilings, fall_back, jev};
use super::sessions::{call_under, test_sessions};
use crate::engine::JevTextValueResolver;
use crate::fallback::FallbackMode;
use crate::tools::finished;

const THREAD: &str = "wall-thread";

/// One call on a fresh session, read as the tool returns it.
async fn browse(
    harness: &Harness,
    page: &str,
    plan: Value,
    values: Option<Arc<dyn JevTextValueResolver>>,
) -> ToolResult {
    let data = call_under(
        harness,
        &test_sessions(),
        THREAD,
        json!({"goal": "scripted", "url": harness.site.url(page)}),
        jev(plan),
        values,
        &ceilings(FallbackMode::Off),
    )
    .await
    .unwrap();
    let arguments = json!({"goal": "scripted"});
    let call = ToolCall {
        id: "call".into(),
        name: "jev_browse".into(),
        raw_arguments: arguments.to_string(),
        arguments,
        thread_id: THREAD.into(),
        turn_id: "turn".into(),
    };
    finished(call, THREAD, data)
}

/// Values by field label; a field with none stops the run `needs_input`, as
/// the text helper does when the goal lacks it.
fn values(values: &[(&str, &str)]) -> Arc<TaskValues> {
    let values = values
        .iter()
        .map(|(label, value)| (label.to_string(), value.to_string()))
        .collect();
    Arc::new(TaskValues::new(&values))
}

fn origin(harness: &Harness) -> String {
    let url = reqwest::Url::parse(&harness.site.url("x.html")).unwrap();
    url.origin().ascii_serialization()
}

/// A sign-in whose password the goal does not give: the account name is
/// typed, then the run stops at the password wall, nothing typed into it.
#[tokio::test]
async fn a_password_the_goal_lacks_stops_at_a_password_wall() {
    let harness = harness_or_skip!();
    let values = values(&[("Username", "ada.lovelace")]);
    let result = browse(
        &harness,
        "login.html",
        json!([{"fill": "Username"}, {"fill": "Password"}, {"enter": "Password"}]),
        Some(values),
    )
    .await;
    let data = &result.data;
    assert_eq!(data["status"], "needs_login", "{data:#}");
    assert_eq!(
        data["wall"],
        json!({"kind": "password", "origin": origin(&harness), "fields": ["Password"]}),
        "{data:#}"
    );
    let reason = data["stopped_because"].as_str().unwrap();
    assert!(reason.starts_with("Sign-in required at "), "{reason}");
    assert!(reason.contains("\"Password\""), "{reason}");
    assert_eq!(data["actions"].as_array().unwrap().len(), 1, "{data:#}");
    // A handoff, and the caller is told who can get past it.
    assert!(!result.is_error, "{}", result.text);
    assert_eq!(data["outcome_class"], "handoff");
    assert!(
        result.text.contains("Jev: needs a sign-in."),
        "{}",
        result.text
    );
    assert!(
        result
            .text
            .contains("Result: the site wants a password at "),
        "{}",
        result.text
    );
    assert!(
        result.text.contains("Never guess credentials"),
        "{}",
        result.text
    );
}

/// A code page with nothing to type: a one-time-code wall, not a value the
/// caller forgot.
#[tokio::test]
async fn a_code_the_goal_lacks_stops_at_a_one_time_code_wall() {
    let harness = harness_or_skip!();
    let values = values(&[]);
    let result = browse(
        &harness,
        "one-time-code.html",
        json!([{"fill": "Code"}]),
        Some(values),
    )
    .await;
    assert_eq!(result.data["status"], "needs_login", "{:#}", result.data);
    assert_eq!(result.data["wall"]["kind"], "one_time_code");
    assert_eq!(result.data["wall"]["fields"], json!(["Code"]));
    assert!(result.text.contains("a one-time code"), "{}", result.text);
}

/// A field the goal lacks outside any sign-in is still `needs_input`.
#[tokio::test]
async fn a_missing_value_outside_a_sign_in_is_still_needs_input() {
    let harness = harness_or_skip!();
    let values = values(&[("Name", "Ada")]);
    let result = browse(
        &harness,
        "contact.html",
        json!([{"fill": "Name"}, {"fill": "Email"}]),
        Some(values),
    )
    .await;
    assert_eq!(result.data["status"], "needs_input", "{:#}", result.data);
    assert!(result.data.get("wall").is_none(), "{:#}", result.data);
}

/// A single sign-on page the model gives up on is a sign-in wall, and the
/// fallback, which would otherwise take over a BLOCKED, never runs.
#[tokio::test]
async fn a_single_sign_on_wall_ends_needs_login_and_does_not_fall_back() {
    let harness = harness_or_skip!();
    let result = fall_back(
        &harness,
        "sso.html",
        json!([{"blocked": true}]),
        json!([{"say": "DONE: signed in"}]),
        &ceilings(FallbackMode::Auto),
    )
    .await;
    assert_eq!(result["status"], "needs_login", "{result:#}");
    assert_eq!(
        result["wall"],
        json!({"kind": "sign_in", "origin": origin(&harness)}),
        "{result:#}"
    );
    assert_ne!(result["fallback"]["ran"], json!(true), "{result:#}");
}

/// An HTTP 401 with nothing to act on asks for a sign-in: `needs_login`
/// before any decision, where a 403 is `access_denied`.
#[tokio::test]
async fn a_401_with_nothing_to_act_on_is_a_sign_in_wall() {
    let harness = harness_or_skip!();
    let result = browse(&harness, "access-denied.html?status=401", json!([]), None).await;
    let data = &result.data;
    assert_eq!(data["status"], "needs_login", "{data:#}");
    assert_eq!(data["wall"]["kind"], "sign_in", "{data:#}");
    assert_eq!(data["model_calls"], 0, "{data:#}");
    assert!(
        data["stopped_because"]
            .as_str()
            .unwrap()
            .contains("HTTP 401"),
        "{data:#}"
    );
    let refused = browse(&harness, "access-denied.html?status=403", json!([]), None).await;
    assert_eq!(refused.data["status"], "access_denied");
    assert!(refused.data.get("wall").is_none());
}
