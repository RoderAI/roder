//! A handoff is an outcome, not a failed tool call. A run that ends
//! `needs_input`, `needs_confirmation` or `access_denied` is Jev telling the
//! caller what only the caller can do; Roder counts every `is_error` tool
//! result toward its five-in-a-row stop, so these results must not be
//! errors, until the same handoff comes back unchanged. Driven through the
//! real session layer on fixture pages, and read as the tool returns them.

use std::sync::Arc;

use roder_api::tools::{ToolCall, ToolResult};
use serde_json::{Value, json};

use super::Harness;
use super::fallback_tests::{ceilings, fall_back, jev};
use super::sessions::{call_under, test_sessions};
use crate::engine::JevDecisionClient;
use crate::fallback::FallbackMode;
use crate::runner::Ceilings;
use crate::session::JevSessions;
use crate::tools::finished;

const THREAD: &str = "handoff-thread";

fn tool_call() -> ToolCall {
    let arguments = json!({"goal": "scripted"});
    ToolCall {
        id: "call".into(),
        name: "jev_browse".into(),
        raw_arguments: arguments.to_string(),
        arguments,
        thread_id: THREAD.into(),
        turn_id: "turn".into(),
    }
}

/// The result the tool returns for `data`.
fn result_of(data: Value) -> ToolResult {
    finished(tool_call(), THREAD, data)
}

/// The gate on, the fallback off: a handoff status stays what Jev said.
fn operator() -> Ceilings {
    let mut operator = ceilings(FallbackMode::Off);
    operator.confirm_irreversible = true;
    operator
}

/// One call on the shared thread, read as the tool returns it.
async fn browse(harness: &Harness, sessions: &JevSessions, page: &str, plan: Value) -> ToolResult {
    let decision: Arc<dyn JevDecisionClient> = jev(plan);
    let data = call_under(
        harness,
        sessions,
        THREAD,
        json!({"goal": "scripted", "url": harness.site.url(page)}),
        decision,
        None,
        &operator(),
    )
    .await
    .unwrap();
    result_of(data)
}

fn status(result: &ToolResult) -> &str {
    result.data["status"].as_str().unwrap_or_default()
}

/// The "Next:" sentence of the text.
fn next_line(result: &ToolResult) -> Option<&str> {
    result.text.lines().find(|line| line.starts_with("Next: "))
}

fn class(result: &ToolResult) -> &str {
    result.data["outcome_class"].as_str().unwrap_or_default()
}

/// What Roder's reliability accounting makes of `results`: the longest run
/// of consecutive `is_error` results (`crates/roder-core/src/reliability.rs`
/// copies `ToolResult.is_error` into the record it counts).
fn longest_failure_run(results: &[ToolResult]) -> usize {
    let (mut longest, mut run) = (0, 0);
    for result in results {
        run = if result.is_error { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    longest
}

/// Five different handoffs in a row, one of each status and two that share a
/// page: none counts as a failed tool call, so the five-in-a-row stop never
/// comes near, and the caller's text never mentions the field.
#[tokio::test]
async fn five_different_handoffs_in_a_row_are_not_failed_tool_calls() {
    let harness = harness_or_skip!();
    let sessions = test_sessions();
    let mut results = Vec::new();
    for (page, plan, expected) in [
        ("access-denied.html?status=403", json!([]), "access_denied"),
        ("contact.html", json!([{"fill": "Name"}]), "needs_input"),
        // The same page, another field: progress, not a repeat.
        ("contact.html", json!([{"fill": "Email"}]), "needs_input"),
        (
            "pay.html",
            json!([{"click": "Pay now", "irreversible": 0.95}]),
            "needs_confirmation",
        ),
        ("access-denied.html?status=429", json!([]), "access_denied"),
    ] {
        let result = browse(&harness, &sessions, page, plan).await;
        assert_eq!(status(&result), expected, "{:#}", result.data);
        assert!(!result.is_error, "{expected} on {page}: {}", result.text);
        assert_eq!(class(&result), "handoff", "{:#}", result.data);
        results.push(result);
    }
    assert_eq!(results.len(), 5);
    assert_eq!(longest_failure_run(&results), 0);

    // The field is for programs: the caller's text never mentions it.
    for result in &results {
        assert!(!result.text.contains("outcome_class"), "{}", result.text);
        assert!(!result.text.contains("repeated"), "{}", result.text);
        assert!(result.text.contains("\nNext: "), "{}", result.text);
    }
}

/// The same handoff on the same page again is the caller not acting on it:
/// an error, and it stays one for as long as it repeats. A different page or
/// reason, or any other kind of call in between, starts it over.
#[tokio::test]
async fn an_identical_handoff_on_the_same_page_is_a_failed_tool_call() {
    let harness = harness_or_skip!();
    let sessions = test_sessions();
    let refused = "access-denied.html?status=403";
    let mut results = Vec::new();
    for _ in 0..3 {
        results.push(browse(&harness, &sessions, refused, json!([])).await);
    }
    assert_eq!(
        results.iter().map(|r| r.is_error).collect::<Vec<_>>(),
        [false, true, true]
    );
    assert_eq!(
        results.iter().map(class).collect::<Vec<_>>(),
        ["handoff", "repeated_handoff", "repeated_handoff"]
    );
    assert!(results.iter().all(|r| status(r) == "access_denied"));
    // The repeat tells the caller what the first did.
    assert_eq!(next_line(&results[1]), next_line(&results[0]));
    assert!(next_line(&results[0]).is_some());

    // Another refusal is a new handoff.
    let other = browse(
        &harness,
        &sessions,
        "access-denied.html?status=429",
        json!([]),
    )
    .await;
    assert!(!other.is_error, "{}", other.text);
    assert_eq!(class(&other), "handoff");

    // So is the first one again after a call that got somewhere.
    let done = browse(&harness, &sessions, "basic.html", json!([])).await;
    assert_eq!(status(&done), "done");
    assert!(!done.is_error);
    assert!(done.data.get("outcome_class").is_none(), "{:#}", done.data);
    let again = browse(&harness, &sessions, refused, json!([])).await;
    assert!(!again.is_error, "{}", again.text);
    assert_eq!(class(&again), "handoff");

    // Six identical needs_input results: the second to the sixth are errors.
    let mut asked = Vec::new();
    for _ in 0..6 {
        asked.push(
            browse(
                &harness,
                &sessions,
                "contact.html",
                json!([{"fill": "Name"}]),
            )
            .await,
        );
    }
    assert!(!asked[0].is_error);
    assert!(asked[1..].iter().all(|r| r.is_error));
    assert_eq!(longest_failure_run(&asked), 5);
}

/// A handoff the automatic fallback ends in is the call's outcome, so it is
/// marked like Jev's own: the status the tool reports is the fallback's.
#[tokio::test]
async fn a_handoff_the_fallback_ends_in_is_marked_too() {
    let harness = harness_or_skip!();
    let mut gated = ceilings(FallbackMode::Auto);
    gated.confirm_irreversible = true;
    let data = fall_back(
        &harness,
        "pay.html",
        json!([{"blocked": true}]),
        json!([{"tool": "click", "args": {"ref_of": "Pay now"}}, {"say": "DONE: paid"}]),
        &gated,
    )
    .await;
    assert_eq!(data["jev_status"], "blocked", "{data:#}");
    let result = result_of(data);
    assert_eq!(status(&result), "needs_confirmation", "{:#}", result.data);
    assert!(!result.is_error, "{}", result.text);
    assert_eq!(class(&result), "handoff");
}

/// Another thread has its own session, and so its own last handoff.
#[tokio::test]
async fn another_threads_handoff_is_not_a_repeat() {
    let harness = harness_or_skip!();
    let sessions = test_sessions();
    let refused = "access-denied.html?status=403";
    let first = browse(&harness, &sessions, refused, json!([])).await;
    assert!(!first.is_error);
    let decision: Arc<dyn JevDecisionClient> = jev(json!([]));
    let elsewhere = call_under(
        &harness,
        &sessions,
        "another-thread",
        json!({"goal": "scripted", "url": harness.site.url(refused)}),
        decision,
        None,
        &operator(),
    )
    .await
    .unwrap();
    let elsewhere = result_of(elsewhere);
    assert!(!elsewhere.is_error, "{}", elsewhere.text);
    assert_eq!(class(&elsewhere), "handoff");
}

/// Every other status that is not a success stays a failed tool call, and
/// none of them gains the field.
#[tokio::test]
async fn blocked_and_the_rest_stay_failed_tool_calls() {
    let harness = harness_or_skip!();
    let sessions = test_sessions();
    let blocked = browse(
        &harness,
        &sessions,
        "hover-menu.html",
        json!([{"blocked": true}]),
    )
    .await;
    assert_eq!(status(&blocked), "blocked", "{:#}", blocked.data);
    assert!(blocked.is_error);
    assert!(blocked.data.get("outcome_class").is_none());

    for status in [
        "blocked",
        "budget_exceeded",
        "timed_out",
        "unavailable",
        "error",
        "busy",
        "ready",
    ] {
        let result = result_of(json!({"status": status, "url": "https://a.test/"}));
        assert!(result.is_error, "{status}");
        assert!(result.data.get("outcome_class").is_none(), "{status}");
    }
    for status in ["done", "closed"] {
        let result = result_of(json!({"status": status, "url": "https://a.test/"}));
        assert!(!result.is_error, "{status}");
        assert!(result.data.get("outcome_class").is_none(), "{status}");
    }
    // A handoff status the session did not classify is not trusted: it is
    // an error, as every non-done status was.
    for status in ["needs_input", "needs_confirmation", "access_denied"] {
        let result = result_of(json!({"status": status, "url": "https://a.test/"}));
        assert!(result.is_error, "{status}");
    }
}
