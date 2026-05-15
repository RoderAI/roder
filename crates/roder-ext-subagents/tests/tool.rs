use std::sync::{Arc, Mutex};

use anyhow::anyhow;
use roder_api::events::{ThreadId, TurnId};
use roder_api::inference::TokenUsage;
use roder_api::subagents::{
    SubagentDefinition, SubagentDispatcher, SubagentExitReason, SubagentPermissionMode,
    SubagentRequest, SubagentResult,
};
use roder_api::tools::{
    ToolCall, ToolContributor, ToolExecutionContext, ToolExecutor, ToolRegistry,
};
use roder_ext_subagents::{TaskTool, TaskToolConfig, TaskToolContributor};
use serde_json::json;

#[tokio::test]
async fn task_tool_validates_required_arguments() {
    let tool = TaskTool::canonical(Arc::new(FakeDispatcher::default()));

    let result = tool
        .execute(
            context(),
            call(json!({
                "description": "Inspect files"
            })),
        )
        .await
        .unwrap();

    assert!(result.is_error);
    assert_eq!(result.data["error"]["kind"], "invalid_arguments");
}

#[tokio::test]
async fn task_tool_leaves_missing_subagent_type_for_dispatcher_fallback() {
    let dispatcher = Arc::new(FakeDispatcher::default());
    let tool = TaskTool::canonical(dispatcher.clone());

    let result = tool
        .execute(
            context(),
            call(json!({
                "description": "Inspect files",
                "prompt": "Find the entry point",
                "model": "override-model",
                "tools": ["Read"],
                "inputs": { "path": "crates" },
                "timeout_seconds": 3
            })),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert_eq!(result.text, "explore: Inspect files\n\nchild final");
    assert_eq!(result.data["thread_id"], "child-thread");
    assert_eq!(result.data["turn_id"], "child-turn");
    assert_eq!(result.data["model"], "override-model");
    assert_eq!(result.data["usage"]["total_tokens"], 3);
    assert_eq!(result.data["exit_reason"], "completed");
    assert_eq!(result.data["transcript"]["items"][0]["role"], "assistant");

    let requests = dispatcher.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].subagent_type, None);
    assert_eq!(requests[0].model.as_deref(), Some("override-model"));
    assert_eq!(requests[0].tools.as_ref().unwrap(), &["Read"]);
    assert_eq!(requests[0].inputs.as_ref().unwrap()["path"], "crates");
}

#[tokio::test]
async fn task_tool_reports_unknown_subagent_type_as_stable_tool_error() {
    let tool = TaskTool::canonical(Arc::new(FakeDispatcher::default()));

    let result = tool
        .execute(
            context(),
            call(json!({
                "description": "Inspect files",
                "prompt": "Find the entry point",
                "subagent_type": "missing"
            })),
        )
        .await
        .unwrap();

    assert!(result.is_error);
    assert_eq!(result.data["error"]["kind"], "unknown_subagent_type");
}

#[tokio::test]
async fn task_tool_reports_tool_whitelist_errors_as_stable_tool_errors() {
    let tool = TaskTool::canonical(Arc::new(FakeDispatcher::default()));

    let result = tool
        .execute(
            context(),
            call(json!({
                "description": "Inspect files",
                "prompt": "Find the entry point",
                "tools": ["Shell"]
            })),
        )
        .await
        .unwrap();

    assert!(result.is_error);
    assert_eq!(result.data["error"]["kind"], "tool_whitelist");
}

#[tokio::test]
async fn task_tool_reports_timeout_results_as_stable_tool_errors() {
    let tool = TaskTool::canonical(Arc::new(FakeDispatcher::with_timeout()));

    let result = tool
        .execute(
            context(),
            call(json!({
                "description": "Inspect files",
                "prompt": "Find the entry point"
            })),
        )
        .await
        .unwrap();

    assert!(result.is_error);
    assert_eq!(result.data["error"]["kind"], "timeout");
    assert_eq!(result.data["exit_reason"], "timeout");
}

#[tokio::test]
async fn namespaced_task_tool_pins_the_subagent_type() {
    let dispatcher = Arc::new(FakeDispatcher::default());
    let tool = TaskTool::for_agent_type(dispatcher.clone(), "review");

    let result = tool
        .execute(
            context(),
            call(json!({
                "description": "Review files",
                "prompt": "Check the patch",
                "subagent_type": "explore"
            })),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert_eq!(result.data["agent_type"], "review");
    assert_eq!(
        dispatcher.requests.lock().unwrap()[0]
            .subagent_type
            .as_deref(),
        Some("review")
    );
}

#[test]
fn task_tool_contributor_installs_only_canonical_tool_by_default() {
    let contributor = TaskToolContributor::new(
        TaskToolConfig::default(),
        Arc::new(FakeDispatcher::default()),
    );
    let mut registry = ToolRegistry::default();

    contributor.contribute(&mut registry).unwrap();

    assert_eq!(tool_names(&registry), ["task"]);
}

#[test]
fn task_tool_contributor_installs_namespaced_tools_only_when_enabled() {
    let contributor = TaskToolContributor::new(
        TaskToolConfig {
            expose_per_type: true,
            ..TaskToolConfig::default()
        },
        Arc::new(FakeDispatcher::default()),
    );
    let mut registry = ToolRegistry::default();

    contributor.contribute(&mut registry).unwrap();

    assert_eq!(
        tool_names(&registry),
        ["task", "task_explore", "task_review"]
    );
}

fn context() -> ToolExecutionContext {
    ToolExecutionContext {
        thread_id: "parent-thread".to_string(),
        turn_id: "parent-turn".to_string(),
    }
}

fn call(arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        id: "tool-call".to_string(),
        name: "task".to_string(),
        raw_arguments: arguments.to_string(),
        arguments,
        thread_id: "parent-thread".to_string(),
        turn_id: "parent-turn".to_string(),
    }
}

fn tool_names(registry: &ToolRegistry) -> Vec<String> {
    registry.specs().into_iter().map(|spec| spec.name).collect()
}

#[derive(Default)]
struct FakeDispatcher {
    requests: Mutex<Vec<SubagentRequest>>,
    timeout: bool,
}

impl FakeDispatcher {
    fn with_timeout() -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            timeout: true,
        }
    }
}

#[async_trait::async_trait]
impl SubagentDispatcher for FakeDispatcher {
    fn id(&self) -> String {
        "fake".to_string()
    }

    fn definitions(&self) -> Vec<SubagentDefinition> {
        vec![definition("explore"), definition("review")]
    }

    async fn dispatch(
        &self,
        _parent_thread_id: ThreadId,
        _parent_turn_id: TurnId,
        request: SubagentRequest,
    ) -> anyhow::Result<SubagentResult> {
        self.requests.lock().unwrap().push(request.clone());
        if request.subagent_type.as_deref() == Some("missing") {
            return Err(anyhow!("unknown subagent type \"missing\""));
        }
        if request
            .tools
            .as_ref()
            .is_some_and(|tools| tools.iter().any(|tool| tool == "Shell"))
        {
            return Err(anyhow!(
                "requested tool \"Shell\" is not allowed by subagent \"explore\""
            ));
        }

        let exit_reason = if self.timeout {
            SubagentExitReason::Timeout
        } else {
            SubagentExitReason::Completed
        };
        Ok(SubagentResult {
            thread_id: "child-thread".to_string(),
            turn_id: "child-turn".to_string(),
            agent_type: request
                .subagent_type
                .unwrap_or_else(|| "explore".to_string()),
            model: request.model.or_else(|| Some("mock".to_string())),
            final_message: if self.timeout {
                "subagent timed out".to_string()
            } else {
                "child final".to_string()
            },
            usage: Some(TokenUsage {
                prompt_tokens: 1,
                completion_tokens: 2,
                total_tokens: 3,
            }),
            exit_reason,
            transcript: Some(json!({
                "items": [{ "role": "assistant", "text": "child final" }],
                "truncated": false
            })),
            metadata: if self.timeout {
                json!({ "error": { "kind": "timeout" } })
            } else {
                json!({})
            },
        })
    }
}

fn definition(agent_type: &str) -> SubagentDefinition {
    SubagentDefinition {
        agent_type: agent_type.to_string(),
        description: format!("{agent_type} agent"),
        tools: vec!["Read".to_string()],
        model: Some("mock".to_string()),
        system_prompt: Some("system".to_string()),
        permission_mode: SubagentPermissionMode::ReadOnly,
        max_turns: Some(4),
        max_result_chars: Some(4000),
    }
}
