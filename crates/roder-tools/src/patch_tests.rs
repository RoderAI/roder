use super::*;
use roder_api::policy_mode::PolicyMode;
use serde_json::{Value, json};

#[test]
fn codex_patch_produces_hunk_records() {
    let ctx = ToolExecutionContext::new("thread-1", "turn-1", PolicyMode::Default);
    let call = ToolCall {
        id: "patch-1".to_string(),
        name: "apply_patch".to_string(),
        arguments: json!({}),
        raw_arguments: "{}".to_string(),
        thread_id: "thread-1".to_string(),
        turn_id: "turn-1".to_string(),
    };
    let records = hunk_records_from_patch(
        &ctx,
        &call,
        "*** Begin Patch\n*** Update File: src/lib.rs\n@@\n-old\n+new\n*** End Patch\n",
    )
    .unwrap();

    assert_eq!(records.len(), 1);
    assert_eq!(records[0].path, "src/lib.rs");
    assert_eq!(records[0].tool_name, "apply_patch");
    assert_eq!(records[0].diff.len(), 2);
}

fn call_with_arguments(arguments: Value) -> ToolCall {
    ToolCall {
        id: "patch-1".to_string(),
        name: "apply_patch".to_string(),
        arguments,
        raw_arguments: "{}".to_string(),
        thread_id: "thread-1".to_string(),
        turn_id: "turn-1".to_string(),
    }
}

#[test]
fn patch_text_accepts_json_function_arguments() {
    let call = call_with_arguments(json!({ "patch": "PATCH" }));
    assert_eq!(patch_text_from_call(&call).unwrap(), "PATCH");
}

#[test]
fn patch_text_requires_the_canonical_field() {
    for arguments in [
        json!({"input": "PATCH"}),
        json!({"raw": "PATCH"}),
        json!("PATCH"),
    ] {
        assert!(patch_text_from_call(&call_with_arguments(arguments)).is_err());
    }
}
