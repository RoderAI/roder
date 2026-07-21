//! Same-stream tool executor hardening tests (roadmap phase 78, Task 4).
//!
//! These tests run the actual SDK MCP tool handlers registered by
//! `build_options` against a fake Roder `TurnToolExecutor`, proving mapped
//! Claude tools (canonical names plus `Read`/`Bash`-style aliases) execute
//! through Roder's executor with repaired arguments, that executor errors
//! and denials surface as tool errors back to the SDK loop, and that
//! unmanaged tools stay denied by the `can_use_tool` callback — all without
//! spawning a real `claude` CLI.

use std::sync::{Arc, Mutex};

use claude_code_sdk_rust::{PermissionResult, types::ToolPermissionContext};
use roder_api::inference::{
    AgentInferenceRequest, HostedWebSearchConfig, InstructionBundle, ModelSelection, OutputConfig,
    ReasoningConfig, RuntimeHints, ToolCallCompleted, TurnToolExecutor, TurnToolOutcome,
};
use roder_api::tools::{ToolChoice, ToolSpec};
use roder_ext_claude_code::{ClaudeCodeConfig, build_options};
use serde_json::json;

#[derive(Default)]
struct RecordingExecutor {
    calls: Mutex<Vec<ToolCallCompleted>>,
    /// Tool names whose execution should fail like a Roder permission denial.
    deny: Vec<String>,
}

#[async_trait::async_trait]
impl TurnToolExecutor for RecordingExecutor {
    async fn execute(&self, call: ToolCallCompleted) -> anyhow::Result<TurnToolOutcome> {
        self.calls.lock().unwrap().push(call.clone());
        if self.deny.iter().any(|name| name == &call.name) {
            return Ok(TurnToolOutcome {
                result: format!("permission denied for {}", call.name),
                is_error: true,
            });
        }
        if call.name == "shell" {
            return Err(anyhow::anyhow!("executor rejected shell during shutdown"));
        }
        if call.name == "request_user_input" {
            // Mirror `Runtime::resolve_user_input_request`: the runtime executor
            // blocks the survey tool until the client answers, then returns the
            // resolved answers to the model. Here we stand in for that resolved
            // outcome so the test exercises the claude-code SDK -> executor path.
            return Ok(TurnToolOutcome {
                result: "User input received:\n{\"release\":\"fix-first\"}".to_string(),
                is_error: false,
            });
        }
        Ok(TurnToolOutcome {
            result: format!("executed {} with {}", call.name, call.arguments),
            is_error: false,
        })
    }
}

fn request_with_tools(tools: Vec<ToolSpec>) -> AgentInferenceRequest {
    AgentInferenceRequest {
        model: ModelSelection {
            provider: "claude-code".to_string(),
            model: "sonnet".to_string(),
        },
        instructions: InstructionBundle::default(),
        transcript: Vec::new(),
        tools,
        tool_choice: ToolChoice::Auto,
        reasoning: ReasoningConfig::default(),
        output: OutputConfig::default(),
        runtime: RuntimeHints {
            hosted_web_search: HostedWebSearchConfig::disabled(),
            ..RuntimeHints::default()
        },
        metadata: json!({}),
    }
}

fn read_file_spec() -> ToolSpec {
    ToolSpec {
        name: "read_file".to_string(),
        description: "Read a file".to_string(),
        parameters: json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"],
            "additionalProperties": false
        }),
    }
}

fn request_user_input_spec() -> ToolSpec {
    ToolSpec {
        name: "request_user_input".to_string(),
        description:
            "Request user input for one to three short questions and wait for the response."
                .to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "questions": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 3,
                    "items": {
                        "type": "object",
                        "properties": {
                            "header": { "type": "string" },
                            "id": { "type": "string" },
                            "question": { "type": "string" },
                            "options": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "label": { "type": "string" },
                                        "description": { "type": "string" }
                                    },
                                    "required": ["label", "description"],
                                    "additionalProperties": false
                                }
                            }
                        },
                        "required": ["header", "id", "question", "options"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["questions"],
            "additionalProperties": false
        }),
    }
}

fn shell_spec() -> ToolSpec {
    ToolSpec {
        name: "shell".to_string(),
        description: "Run a command".to_string(),
        parameters: json!({
            "type": "object",
            "properties": { "command": { "type": "string" } },
            "required": ["command"],
            "additionalProperties": false
        }),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn claude_read_alias_executes_through_roder_executor_with_repaired_args() {
    let executor = Arc::new(RecordingExecutor::default());
    let options = build_options(
        &ClaudeCodeConfig::default(),
        &request_with_tools(vec![read_file_spec()]),
        Some(executor.clone()),
        None,
        None,
    )
    .unwrap();
    let server = options.sdk_mcp_servers.get("roder").unwrap();

    // Claude calls its native alias `Read` with `file_path`; the handler must
    // repair the argument name and execute canonical `read_file`.
    let content = server
        .call_tool("Read", json!({ "file_path": "README.md" }))
        .expect("Read alias executes");
    let text = match &content[0] {
        claude_code_sdk_rust::mcp::MCPContent::Text { text } => text.clone(),
        other => panic!("expected text content, got {other:?}"),
    };
    assert!(text.contains("executed read_file"), "{text}");
    assert!(text.contains("\"path\":\"README.md\""), "{text}");

    let calls = executor.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "read_file");
    assert!(
        calls[0].id.starts_with("claude-code-Read-"),
        "id should be name-prefixed and unique: {}",
        calls[0].id
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn repeated_calls_of_same_tool_get_unique_ids() {
    let executor = Arc::new(RecordingExecutor::default());
    let options = build_options(
        &ClaudeCodeConfig::default(),
        &request_with_tools(vec![read_file_spec()]),
        Some(executor.clone()),
        None,
        None,
    )
    .unwrap();
    let server = options.sdk_mcp_servers.get("roder").unwrap();

    server
        .call_tool("Read", json!({ "file_path": "README.md" }))
        .expect("first Read alias executes");
    server
        .call_tool("Read", json!({ "file_path": "Cargo.toml" }))
        .expect("second Read alias executes");

    let calls = executor.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    // Distinct ids are required: the TUI/runtime key tool-call rows by id, so a
    // reused id would collapse the second call into the first row.
    assert_ne!(
        calls[0].id, calls[1].id,
        "repeated tool calls must have unique ids"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn denied_roder_permission_surfaces_as_tool_error_to_the_sdk_loop() {
    let executor = Arc::new(RecordingExecutor {
        calls: Mutex::new(Vec::new()),
        deny: vec!["read_file".to_string()],
    });
    let options = build_options(
        &ClaudeCodeConfig::default(),
        &request_with_tools(vec![read_file_spec()]),
        Some(executor.clone()),
        None,
        None,
    )
    .unwrap();
    let server = options.sdk_mcp_servers.get("roder").unwrap();

    let content = server
        .call_tool("read_file", json!({ "path": "/etc/shadow" }))
        .expect("denial is reported as tool content, not a transport error");
    let text = match &content[0] {
        claude_code_sdk_rust::mcp::MCPContent::Text { text } => text.clone(),
        other => panic!("expected text content, got {other:?}"),
    };
    assert!(text.contains("Tool returned an error"), "{text}");
    assert!(text.contains("permission denied for read_file"), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn executor_failure_propagates_as_mcp_error() {
    let executor = Arc::new(RecordingExecutor::default());
    let options = build_options(
        &ClaudeCodeConfig::default(),
        &request_with_tools(vec![shell_spec()]),
        Some(executor.clone()),
        None,
        None,
    )
    .unwrap();
    let server = options.sdk_mcp_servers.get("roder").unwrap();

    // Bash alias arrives as a raw argv array; the input repair joins it into
    // the canonical `command` string before the executor rejects it.
    let error = server
        .call_tool("Bash", json!(["ls", "-la"]))
        .expect_err("executor failure must propagate");
    assert!(error.contains("executor rejected shell"), "{error}");

    let calls = executor.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
    let arguments: serde_json::Value = serde_json::from_str(&calls[0].arguments).unwrap();
    assert_eq!(arguments["command"], "ls -la");
}

#[tokio::test(flavor = "multi_thread")]
async fn unmanaged_and_unadvertised_tools_stay_denied_by_can_use_tool() {
    let options = build_options(
        &ClaudeCodeConfig::default(),
        &request_with_tools(vec![read_file_spec()]),
        Some(Arc::new(RecordingExecutor::default())),
        None,
        None,
    )
    .unwrap();
    let callback = options.can_use_tool.expect("can_use_tool registered");

    // A bare built-in Claude tool is not managed by Roder.
    let result = callback
        .call(
            "WebFetch".to_string(),
            serde_json::Map::new(),
            ToolPermissionContext::default(),
        )
        .await
        .unwrap();
    assert!(
        matches!(result, PermissionResult::Deny { ref message, .. } if message.contains("not managed by Roder")),
        "{result:?}"
    );

    // A Roder-prefixed tool that was not advertised this turn is denied too.
    let result = callback
        .call(
            "mcp__roder__shell".to_string(),
            serde_json::Map::new(),
            ToolPermissionContext::default(),
        )
        .await
        .unwrap();
    assert!(
        matches!(result, PermissionResult::Deny { ref message, .. } if message.contains("not advertised")),
        "{result:?}"
    );

    // The advertised mapped tool is allowed.
    let result = callback
        .call(
            "mcp__roder__read_file".to_string(),
            serde_json::Map::new(),
            ToolPermissionContext::default(),
        )
        .await
        .unwrap();
    assert!(
        matches!(result, PermissionResult::Allow { .. }),
        "{result:?}"
    );
}

/// The interactive survey tool must be available on the claude-code path: it is
/// advertised as `mcp__roder__request_user_input`, the `can_use_tool` callback
/// pre-authorizes it, and calling it routes through Roder's executor with the
/// nested `questions` payload intact. The runtime executor blocks the call
/// until the client answers and returns the resolved answers to the model;
/// here a fake executor stands in for that resolved outcome.
#[tokio::test(flavor = "multi_thread")]
async fn request_user_input_is_advertised_and_routes_through_executor() {
    let executor = Arc::new(RecordingExecutor::default());
    let options = build_options(
        &ClaudeCodeConfig::default(),
        &request_with_tools(vec![request_user_input_spec()]),
        Some(executor.clone()),
        None,
        None,
    )
    .unwrap();

    // Advertised to the CLI under the canonical Roder MCP name (no alias).
    assert!(
        options
            .allowed_tools
            .iter()
            .any(|name| name == "mcp__roder__request_user_input"),
        "request_user_input must be advertised: {:?}",
        options.allowed_tools
    );

    // The permission callback pre-authorizes the advertised survey tool.
    let callback = options
        .can_use_tool
        .clone()
        .expect("can_use_tool registered");
    let permission = callback
        .call(
            "mcp__roder__request_user_input".to_string(),
            serde_json::Map::new(),
            ToolPermissionContext::default(),
        )
        .await
        .unwrap();
    assert!(
        matches!(permission, PermissionResult::Allow { .. }),
        "{permission:?}"
    );

    // Calling it routes through Roder's executor with the questions preserved.
    let server = options.sdk_mcp_servers.get("roder").unwrap();
    let questions = json!({
        "questions": [{
            "header": "Failing e2e test before release",
            "id": "release",
            "question": "How do you want to handle it before releasing?",
            "options": [
                { "label": "fix-first", "description": "Fix the test before releasing." },
                { "label": "ship-anyway", "description": "Release and follow up." }
            ]
        }]
    });
    let content = server
        .call_tool("request_user_input", questions.clone())
        .expect("request_user_input executes through the roder executor");
    let text = match &content[0] {
        claude_code_sdk_rust::mcp::MCPContent::Text { text } => text.clone(),
        other => panic!("expected text content, got {other:?}"),
    };
    assert!(text.contains("User input received"), "{text}");

    let calls = executor.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "request_user_input");
    // The nested questions array must survive `retain_schema_properties` so the
    // survey reaches the runtime tool intact rather than being flattened away.
    let arguments: serde_json::Value = serde_json::from_str(&calls[0].arguments).unwrap();
    assert_eq!(
        arguments["questions"][0]["id"], "release",
        "questions payload must be preserved: {arguments}"
    );
    assert_eq!(
        arguments["questions"][0]["options"][0]["label"],
        "fix-first"
    );
}
