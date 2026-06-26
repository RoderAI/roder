use std::sync::{Arc, Mutex};

use anyhow::anyhow;
use roder_api::ToolSchemaPolicy;
use roder_api::events::{ThreadId, TurnId};
use roder_api::inference::TokenUsage;
use roder_api::policy_mode::PolicyMode;
use roder_api::subagents::{
    AgentSwarmConfig, SubagentDefinition, SubagentDispatcher, SubagentExitReason, SubagentLane,
    SubagentPermissionMode, SubagentRequest, SubagentResult,
};
use roder_api::tools::{
    ToolCall, ToolContributor, ToolExecutionContext, ToolExecutor, ToolRegistry,
};
use roder_ext_subagents::{AgentSwarmTool, TaskTool, TaskToolConfig, TaskToolContributor};
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
                "lane": "scout",
                "max_concurrent": 2,
                "allowed_tools": ["Read"],
                "parent_deadline_seconds": 30,
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
    assert_eq!(requests[0].lane, Some(SubagentLane::Scout));
    assert_eq!(requests[0].max_concurrent, Some(2));
    assert_eq!(requests[0].allowed_tools.as_ref().unwrap(), &["Read"]);
    assert_eq!(requests[0].parent_deadline_seconds, Some(30));
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
async fn speed_policy_blocks_editor_lane_in_plan_mode() {
    let tool = TaskTool::canonical(Arc::new(FakeDispatcher::default()));

    let result = tool
        .execute(
            ToolExecutionContext::new("parent-thread", "parent-turn", PolicyMode::Plan),
            call(json!({
                "description": "Edit files",
                "prompt": "Patch the code",
                "lane": "editor"
            })),
        )
        .await
        .unwrap();

    assert!(result.is_error);
    assert_eq!(result.data["error"]["kind"], "policy_mode");
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

    assert_eq!(tool_names(&registry), ["agent_swarm", "task"]);
}

#[test]
fn task_tool_contributor_can_disable_agent_swarm_tool() {
    let contributor = TaskToolContributor::new(
        TaskToolConfig {
            expose_agent_swarm: false,
            ..TaskToolConfig::default()
        },
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
        ["agent_swarm", "task", "task_explore", "task_review"]
    );
}

#[test]
fn schema_snapshot_covers_model_facing_task_tool() {
    let spec = TaskTool::canonical(Arc::new(FakeDispatcher::default()))
        .spec()
        .normalized_for_model(ToolSchemaPolicy::strict());
    let schema = serde_json::to_string(&spec.parameters).unwrap();

    assert!(
        schema.starts_with(r#"{"type":"object","required":["description","prompt"],"properties":"#)
    );
    assert!(schema.contains(
        r#""inputs":{"type":"object","description":"Optional freeform structured context for the child task."}"#
    ));
    assert!(schema.contains(r#""lane":{"type":"string""#));
    assert!(schema.contains(r#""enum":["scout","editor","reviewer","runner"]"#));
    assert!(schema.contains(r#""max_concurrent":{"type":"integer""#));
    assert!(schema.contains(r#""minimum":1"#));
    assert!(schema.contains(r#""additionalProperties":false"#));
}

#[tokio::test]
async fn agent_swarm_tool_dispatches_children_in_order() {
    let dispatcher = Arc::new(FakeDispatcher::default());
    let tool = AgentSwarmTool::new(dispatcher.clone(), AgentSwarmConfig::default());

    let result = tool
        .execute(
            context(),
            agent_swarm_call(json!({
                "description": "inspect fixtures",
                "subagent_type": "explore",
                "prompt_template": "Read {{item}} and report.",
                "items": ["a.rs", "b.rs"]
            })),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.text.contains("<agent_swarm_result>"));
    assert!(result.text.contains("<summary>completed: 2</summary>"));
    assert_eq!(result.data["agent_swarm"]["completed"], 2);
    assert_eq!(result.data["agent_swarm"]["children"][0]["item"], "a.rs");
    assert_eq!(result.data["agent_swarm"]["children"][1]["item"], "b.rs");

    let requests = dispatcher.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].subagent_type.as_deref(), Some("explore"));
    assert_eq!(requests[0].prompt, "Read a.rs and report.");
    assert_eq!(requests[1].prompt, "Read b.rs and report.");
}

#[tokio::test]
async fn agent_swarm_tool_rejects_single_item_without_resume() {
    let tool = AgentSwarmTool::new(Arc::new(FakeDispatcher::default()), AgentSwarmConfig::default());

    let result = tool
        .execute(
            context(),
            agent_swarm_call(json!({
                "description": "inspect fixtures",
                "prompt_template": "Read {{item}}.",
                "items": ["only.rs"]
            })),
        )
        .await
        .unwrap();

    assert!(result.is_error);
    assert_eq!(result.data["error"]["kind"], "invalid_arguments");
}

fn context() -> ToolExecutionContext {
    ToolExecutionContext::new("parent-thread", "parent-turn", PolicyMode::Default)
}

fn agent_swarm_call(arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        id: "tool-call".to_string(),
        name: "agent_swarm".to_string(),
        raw_arguments: arguments.to_string(),
        arguments,
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
            usage: Some(TokenUsage::new(1, 2, 3)),
            exit_reason,
            transcript: Some(json!({
                "items": [{ "role": "assistant", "text": "child final" }],
                "truncated": false
            })),
            metadata: if self.timeout {
                json!({ "error": { "kind": "timeout" } })
            } else {
                json!({ "lane": request.lane.map(|lane| lane.as_str()) })
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
