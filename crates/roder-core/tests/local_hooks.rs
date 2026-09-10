//! End-to-end coverage for `.roder/hooks.json` `PreToolUse` dispatch.
//!
//! These drive real hook processes rather than the JSON parser alone, so they
//! pin the behaviour a hook author actually observes: a chain sees each
//! rewrite, a deny blocks whether or not it explains itself, and an unhooked
//! call is passed through untouched.

use std::path::Path;
use std::sync::Arc;

use roder_api::events::{ThreadId, TurnId};
use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::tools::ToolCall;
use roder_core::fake_provider::FakeInferenceEngine;
use roder_core::hooks::{PreToolUseResult, run_pre_tool_use};
use roder_core::{Runtime, RuntimeConfig};
use serde_json::{Value, json};

fn runtime() -> Arc<Runtime> {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FakeInferenceEngine));
    Arc::new(Runtime::new(builder.build().unwrap(), RuntimeConfig::default()).unwrap())
}

fn write_hooks(workspace: &Path, hooks: Value) {
    std::fs::create_dir_all(workspace.join(".roder")).unwrap();
    std::fs::write(
        workspace.join(".roder/hooks.json"),
        serde_json::to_vec(&hooks).unwrap(),
    )
    .unwrap();
}

/// A hook that reads the payload on stdin and answers with `python3`, so the
/// assertions can depend on what the handler was actually handed.
fn python_hook(body: &str) -> Value {
    json!({
        "type": "command",
        "command": format!("python3 -c {}", shell_quote(body)),
        "timeout": 30,
    })
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

fn tool_call(arguments: Value) -> ToolCall {
    ToolCall {
        id: "call-1".to_string(),
        name: "shell".to_string(),
        raw_arguments: arguments.to_string(),
        arguments,
        thread_id: ThreadId::from("thread-hooks"),
        turn_id: TurnId::from("turn-hooks"),
    }
}

#[tokio::test]
async fn chained_hooks_each_observe_the_previous_rewrite() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path();

    // First hook rewrites the input; second echoes back the input it was given.
    // If the payload were rebuilt only once, the second hook would report the
    // model's original "start" rather than the first hook's "one".
    write_hooks(
        workspace,
        json!({"hooks": {"PreToolUse": [{"matcher": "shell", "hooks": [
            python_hook(
                "import json,sys; json.load(sys.stdin); \
                 print(json.dumps({'hookSpecificOutput': {'permissionDecision': 'allow', \
                 'updatedInput': {'step': 'one'}}}))",
            ),
            python_hook(
                "import json,sys; p=json.load(sys.stdin); \
                 print(json.dumps({'hookSpecificOutput': {'permissionDecision': 'allow', \
                 'updatedInput': {'seen': p['toolInput'].get('step')}}}))",
            ),
        ]}]}}),
    );

    let runtime = runtime();
    let call = tool_call(json!({"step": "start"}));
    let result = run_pre_tool_use(
        &runtime,
        &call.thread_id.clone(),
        &call.turn_id.clone(),
        Some(&workspace.display().to_string()),
        &call,
    )
    .await;

    match result {
        PreToolUseResult::Continue(Some(input)) => {
            assert_eq!(
                input.get("seen").and_then(Value::as_str),
                Some("one"),
                "second hook must see the first hook's rewrite, got {input}"
            );
        }
        other => panic!("expected a rewritten Continue, got {other:?}"),
    }
}

#[tokio::test]
async fn a_deny_without_a_reason_still_blocks_the_call() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path();
    write_hooks(
        workspace,
        json!({"hooks": {"PreToolUse": [{"matcher": "shell", "hooks": [
            python_hook(
                "import json,sys; json.load(sys.stdin); \
                 print(json.dumps({'hookSpecificOutput': {'permissionDecision': 'deny'}}))",
            ),
        ]}]}}),
    );

    let runtime = runtime();
    let call = tool_call(json!({"command": "rm -rf /"}));
    let result = run_pre_tool_use(
        &runtime,
        &call.thread_id.clone(),
        &call.turn_id.clone(),
        Some(&workspace.display().to_string()),
        &call,
    )
    .await;

    match result {
        PreToolUseResult::Denied(reason) => {
            assert!(!reason.trim().is_empty(), "denial must carry a reason");
        }
        other => panic!("a bare deny must block the call, got {other:?}"),
    }
}

#[tokio::test]
async fn an_unhooked_call_is_passed_through_untouched() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = runtime();
    let call = tool_call(json!({"command": "echo hi"}));

    let result = run_pre_tool_use(
        &runtime,
        &call.thread_id.clone(),
        &call.turn_id.clone(),
        Some(&temp.path().display().to_string()),
        &call,
    )
    .await;

    // `None` is what keeps the model's original `raw_arguments` string intact
    // at the call site instead of re-serializing it on every tool call.
    assert!(
        matches!(result, PreToolUseResult::Continue(None)),
        "expected an untouched pass-through, got {result:?}"
    );
}
