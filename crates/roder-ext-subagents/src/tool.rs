use std::sync::Arc;

use anyhow::bail;
use roder_api::extension::ToolProviderId;
use roder_api::policy_mode::PolicyMode;
use roder_api::subagents::{
    SubagentDispatcher, SubagentExitReason, SubagentLane, SubagentRequest, SubagentResult,
};
use roder_api::tools::{
    ToolCall, ToolContributor, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult,
    ToolSpec,
};
use serde::Deserialize;
use serde_json::{Value, json};

const CANONICAL_TASK_TOOL: &str = "task";

#[derive(Debug, Clone)]
pub struct TaskToolConfig {
    pub provider_id: ToolProviderId,
    pub expose_per_type: bool,
}

impl Default for TaskToolConfig {
    fn default() -> Self {
        Self {
            provider_id: "roder-subagents-task".to_string(),
            expose_per_type: false,
        }
    }
}

pub struct TaskToolContributor {
    config: TaskToolConfig,
    dispatcher: Arc<dyn SubagentDispatcher>,
}

impl TaskToolContributor {
    pub fn new(config: TaskToolConfig, dispatcher: Arc<dyn SubagentDispatcher>) -> Self {
        Self { config, dispatcher }
    }
}

impl ToolContributor for TaskToolContributor {
    fn id(&self) -> ToolProviderId {
        self.config.provider_id.clone()
    }

    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        registry.register(Arc::new(TaskTool::canonical(self.dispatcher.clone())))?;

        if self.config.expose_per_type {
            for definition in self.dispatcher.definitions() {
                registry.register(Arc::new(TaskTool::for_agent_type(
                    self.dispatcher.clone(),
                    definition.agent_type,
                )))?;
            }
        }

        Ok(())
    }
}

pub struct TaskTool {
    name: String,
    fixed_subagent_type: Option<String>,
    dispatcher: Arc<dyn SubagentDispatcher>,
}

impl TaskTool {
    pub fn canonical(dispatcher: Arc<dyn SubagentDispatcher>) -> Self {
        Self {
            name: CANONICAL_TASK_TOOL.to_string(),
            fixed_subagent_type: None,
            dispatcher,
        }
    }

    pub fn for_agent_type(
        dispatcher: Arc<dyn SubagentDispatcher>,
        agent_type: impl Into<String>,
    ) -> Self {
        let agent_type = agent_type.into();
        Self {
            name: namespaced_task_tool_name(&agent_type),
            fixed_subagent_type: Some(agent_type),
            dispatcher,
        }
    }
}

#[async_trait::async_trait]
impl ToolExecutor for TaskTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name.clone(),
            description: match &self.fixed_subagent_type {
                Some(agent_type) => {
                    format!("Dispatch the {agent_type} subagent and return its final message.")
                }
                None => "Dispatch a configured subagent and return its final message.".to_string(),
            },
            parameters: json!({
                "type": "object",
                "properties": {
                    "description": {
                        "type": "string",
                        "description": "Short label for the child task."
                    },
                    "prompt": {
                        "type": "string",
                        "description": "Full instructions for the child agent."
                    },
                    "subagent_type": {
                        "type": "string",
                        "description": "Optional configured subagent type."
                    },
                    "model": {
                        "type": "string",
                        "description": "Optional model override."
                    },
                    "tools": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional additional restriction over the subagent tool whitelist."
                    },
                    "lane": {
                        "type": "string",
                        "enum": ["scout", "editor", "reviewer", "runner"],
                        "description": "Optional execution lane preset for policy, concurrency, and summary expectations."
                    },
                    "max_concurrent": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Optional lane-local concurrency cap for this request."
                    },
                    "allowed_tools": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional explicit allowed-tool cap in addition to the subagent definition."
                    },
                    "parent_deadline_seconds": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Optional remaining parent deadline budget in seconds."
                    },
                    "inputs": {
                        "type": "object",
                        "description": "Optional freeform structured context for the child task."
                    },
                    "timeout_seconds": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Optional timeout for the child run."
                    }
                },
                "required": ["description", "prompt"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let request = match parse_task_request(call.arguments, self.fixed_subagent_type.as_deref())
        {
            Ok(request) => request,
            Err(err) => {
                return Ok(error_result(
                    call.id,
                    call.name,
                    "invalid_arguments",
                    err.to_string(),
                ));
            }
        };
        if let Err(err) = validate_policy_mode(ctx.effective_mode, &request) {
            return Ok(error_result(
                call.id,
                call.name,
                "policy_mode",
                err.to_string(),
            ));
        }

        match self
            .dispatcher
            .dispatch_traced(
                ctx.thread_id,
                ctx.turn_id,
                request.clone(),
                ctx.handles.subagent_trace_sink.clone(),
            )
            .await
        {
            Ok(result) => Ok(result_to_tool_result(call.id, call.name, request, result)),
            Err(err) => {
                let message = err.to_string();
                let kind = classify_dispatch_error(&message);
                Ok(error_result(call.id, call.name, kind, message))
            }
        }
    }
}

pub fn namespaced_task_tool_name(agent_type: &str) -> String {
    format!("task_{agent_type}")
}

fn parse_task_request(
    arguments: Value,
    fixed_subagent_type: Option<&str>,
) -> anyhow::Result<SubagentRequest> {
    let args: TaskToolArguments = serde_json::from_value(arguments)?;
    if args.description.trim().is_empty() {
        bail!("description must not be empty");
    }
    if args.prompt.trim().is_empty() {
        bail!("prompt must not be empty");
    }
    if matches!(args.timeout_seconds, Some(0)) {
        bail!("timeout_seconds must be at least 1");
    }
    if matches!(args.max_concurrent, Some(0)) {
        bail!("max_concurrent must be at least 1");
    }

    Ok(SubagentRequest {
        description: args.description,
        prompt: args.prompt,
        subagent_type: fixed_subagent_type
            .map(str::to_string)
            .or(args.subagent_type),
        model: args.model,
        tools: args.tools,
        lane: args.lane,
        max_concurrent: args.max_concurrent,
        allowed_tools: args.allowed_tools,
        parent_deadline_seconds: args.parent_deadline_seconds,
        inputs: args.inputs,
        timeout_seconds: args.timeout_seconds,
    })
}

fn validate_policy_mode(mode: PolicyMode, request: &SubagentRequest) -> anyhow::Result<()> {
    if mode != PolicyMode::Plan {
        return Ok(());
    }
    if matches!(
        request.lane,
        Some(SubagentLane::Editor | SubagentLane::Runner)
    ) {
        bail!("plan mode only allows scout or reviewer subagent lanes");
    }
    let tool_names = request
        .tools
        .iter()
        .flatten()
        .chain(request.allowed_tools.iter().flatten());
    if tool_names
        .into_iter()
        .any(|tool| is_state_changing_tool(tool))
    {
        bail!("plan mode blocks editor, runner, write, and process subagent tools");
    }
    Ok(())
}

fn is_state_changing_tool(tool: &str) -> bool {
    matches!(
        tool,
        "Shell"
            | "shell"
            | "exec_command"
            | "run_command"
            | "write_file"
            | "edit"
            | "multi_edit"
            | "apply_patch"
    )
}

fn result_to_tool_result(
    id: String,
    name: String,
    request: SubagentRequest,
    result: SubagentResult,
) -> ToolResult {
    let SubagentResult {
        thread_id,
        turn_id,
        agent_type,
        model,
        final_message,
        usage,
        exit_reason,
        transcript,
        metadata,
    } = result;
    let is_error = exit_reason != SubagentExitReason::Completed;
    let mut data = json!({
        "thread_id": thread_id,
        "turn_id": turn_id,
        "agent_type": agent_type,
        "model": model,
        "usage": usage,
        "exit_reason": exit_reason,
    });
    if let Some(transcript) = transcript {
        data["transcript"] = transcript;
    }
    if let Some(lane) = metadata.get("lane") {
        data["lane"] = lane.clone();
    }
    if let Some(summary_contract) = metadata.get("summary_contract") {
        data["summary_contract"] = summary_contract.clone();
    }
    if is_error {
        data["error"] = metadata
            .get("error")
            .cloned()
            .unwrap_or_else(|| json!({ "kind": exit_reason_error_kind(&data["exit_reason"]) }));
    }

    ToolResult {
        id,
        name,
        text: render_result_text(
            data["agent_type"].as_str().unwrap_or("unknown"),
            &request.description,
            &final_message,
        ),
        data,
        is_error,
    }
}

fn render_result_text(agent_type: &str, description: &str, final_message: &str) -> String {
    format!("{agent_type}: {description}\n\n{final_message}")
}

fn error_result(id: String, name: String, kind: &'static str, message: String) -> ToolResult {
    ToolResult {
        id,
        name,
        text: message.clone(),
        data: json!({ "error": { "kind": kind, "message": message } }),
        is_error: true,
    }
}

fn classify_dispatch_error(message: &str) -> &'static str {
    if message.contains("unknown subagent type") {
        "unknown_subagent_type"
    } else if message.contains("not allowed by subagent")
        || message.contains("subagent tool")
        || message.contains("unavailable tool")
    {
        "tool_whitelist"
    } else if message.contains("timed out") || message.contains("timeout") {
        "timeout"
    } else {
        "dispatch_failed"
    }
}

fn exit_reason_error_kind(exit_reason: &Value) -> &'static str {
    match exit_reason.as_str() {
        Some("timeout") => "timeout",
        Some("cancelled") => "cancelled",
        Some("max_turns") => "max_turns",
        Some("failed") => "dispatch_failed",
        _ => "dispatch_failed",
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskToolArguments {
    description: String,
    prompt: String,
    #[serde(default)]
    subagent_type: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    tools: Option<Vec<String>>,
    #[serde(default)]
    lane: Option<SubagentLane>,
    #[serde(default)]
    max_concurrent: Option<usize>,
    #[serde(default)]
    allowed_tools: Option<Vec<String>>,
    #[serde(default)]
    parent_deadline_seconds: Option<u64>,
    #[serde(default)]
    inputs: Option<Value>,
    #[serde(default)]
    timeout_seconds: Option<u64>,
}
