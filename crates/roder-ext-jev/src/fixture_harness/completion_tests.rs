//! Independent browser postconditions reject a model that claims DONE early.
use super::scripted::{PlanDecider, pick};
use super::sessions::{call, test_sessions};
use serde_json::json;
use std::sync::Arc;

#[tokio::test]
async fn success_condition_rejects_false_done_and_allows_verified_progress() {
    let harness = harness_or_skip!();
    let sessions = test_sessions();
    let args = json!({"goal":"Add one", "url":harness.site.url("counter.html"),
        "success_condition":{"url_contains":"counter.html", "text_contains":"Count:\n1"}});
    let premature = call(
        &harness,
        &sessions,
        "completion",
        args,
        Arc::new(PlanDecider::new(vec![])),
    )
    .await
    .unwrap();
    assert_eq!(premature["status"], "blocked", "{premature:#}");
    assert_eq!(premature["completion_verification"]["status"], "failed");
    assert_eq!(premature["fallback"]["trigger"]["kind"], "outcome_mismatch");
    let correct = call(
        &harness,
        &sessions,
        "completion",
        json!({"goal":"Add one", "success_condition":{"text_contains":"Count:\n1"}}),
        Arc::new(PlanDecider::new(vec![pick("click", "Add one")])),
    )
    .await
    .unwrap();
    assert_eq!(correct["status"], "done", "{correct:#}");
    assert_eq!(correct["completion_verification"]["status"], "passed");
    assert!(
        correct["visible_text"]
            .as_str()
            .unwrap()
            .contains("Count:\n1")
    );
}
#[tokio::test]
async fn search_false_done_fails_a_result_url_condition() {
    let harness = harness_or_skip!();
    let result = call(
        &harness,
        &test_sessions(),
        "search",
        json!({"goal":"Search", "url":harness.site.url("keys.html"),
        "success_condition":{"url_contains":"/keys/search"}}),
        Arc::new(PlanDecider::new(vec![])),
    )
    .await
    .unwrap();
    assert_eq!(result["status"], "blocked", "{result:#}");
    assert_eq!(result["completion_verification"]["status"], "failed");
}

#[tokio::test]
async fn failed_jev_done_runs_fallback_and_rejects_its_false_done_too() {
    use super::fallback_script::ScriptedFallback;
    use super::fallback_tests::ceilings;
    use super::sessions::call_falling_back;
    use crate::fallback::FallbackMode;
    let harness = harness_or_skip!();
    for (name, plan, expected) in [
        (
            "recover",
            json!([{"tool":"click","args":{"ref_of":"Add one"}},{"say":"DONE: added one"}]),
            "done",
        ),
        (
            "false-fallback",
            json!([{"say":"DONE: already complete"}]),
            "blocked",
        ),
    ] {
        let result=call_falling_back(&harness,&test_sessions(),name,json!({"goal":"Add one", "url":harness.site.url("counter.html"),"success_condition":{"text_contains":"Count:\n1"}}),
            Arc::new(PlanDecider::new(vec![])),Arc::new(ScriptedFallback::from_json(plan)),&ceilings(FallbackMode::Auto)).await.unwrap();
        assert_eq!(result["status"], expected, "{result:#}");
        assert_eq!(result["fallback"]["ran"], true);
        assert_eq!(result["jev_status"], "blocked");
        let mut text_data = result.clone();
        let text = crate::report::tool_text(&mut text_data);
        assert!(text.contains("Completion check:"), "{text}");
        assert_eq!(
            result["completion_verification"]["status"],
            if expected == "done" {
                "passed"
            } else {
                "failed"
            }
        );
    }
}
#[tokio::test]
async fn partial_fallback_preserves_its_actual_final_observation() {
    use super::fallback_script::ScriptedFallback;
    use super::fallback_tests::ceilings;
    use super::sessions::call_falling_back;
    use crate::fallback::FallbackMode;
    let harness = harness_or_skip!();
    let result=call_falling_back(&harness,&test_sessions(),"partial",json!({"goal":"Add two", "url":harness.site.url("counter.html"),"success_condition":{"text_contains":"Count:\n2"}}),
        Arc::new(PlanDecider::new(vec![])),Arc::new(ScriptedFallback::from_json(json!([{"tool":"click","args":{"ref_of":"Add one"}},{"say":"BLOCKED: cannot continue"}]))),&ceilings(FallbackMode::Auto)).await.unwrap();
    assert_eq!(result["status"], "blocked", "{result:#}");
    assert!(
        result["visible_text"]
            .as_str()
            .unwrap()
            .contains("Count:\n1"),
        "{result:#}"
    );
}

#[test]
fn success_conditions_reject_unknown_fields_and_unbounded_strings() {
    use crate::session::completion::Completion;
    assert!(Completion::parse(&json!({"success_condition":{"javascript":"true"}})).is_err());
    assert!(
        Completion::parse(&json!({"success_condition":{"text_contains":"x".repeat(4097)}}))
            .is_err()
    );
    assert!(
        Completion::parse(&json!({"success_condition":{}}))
            .unwrap()
            .is_none()
    );
}
