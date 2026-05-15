use std::sync::Arc;

use anyhow::bail;
use roder_api::extension::ToolProviderId;
use roder_api::policy_mode::PolicyMode;
use roder_api::tools::{
    ToolCall, ToolContributor, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult,
    ToolSpec,
};
use serde::Deserialize;
use serde_json::{Value, json};

pub const EXIT_PLAN_MODE_TOOL: &str = "exit_plan_mode";

pub struct ExitPlanModeToolContributor;

impl ToolContributor for ExitPlanModeToolContributor {
    fn id(&self) -> ToolProviderId {
        "plan-mode-tools".to_string()
    }

    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        registry.register(Arc::new(ExitPlanModeTool))
    }
}

pub struct ExitPlanModeTool;

#[async_trait::async_trait]
impl ToolExecutor for ExitPlanModeTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: EXIT_PLAN_MODE_TOOL.to_string(),
            description: "Request user approval to leave plan mode.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "summary": {
                        "type": "string",
                        "description": "Concise plan summary for user approval."
                    },
                    "next_steps": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional concrete implementation steps."
                    },
                    "target_mode": {
                        "type": "string",
                        "enum": ["default", "accept_edits"],
                        "description": "Mode to enter after approval."
                    }
                },
                "required": ["summary"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args = match parse_arguments(call.arguments) {
            Ok(args) => args,
            Err(err) => {
                return Ok(error_result(
                    call.id,
                    call.name,
                    "invalid_arguments",
                    err.to_string(),
                ));
            }
        };
        if ctx.effective_mode != PolicyMode::Plan {
            return Ok(error_result(
                call.id,
                call.name,
                "not_in_plan_mode",
                "exit_plan_mode can only be used while policy mode is plan".to_string(),
            ));
        }

        let request_id = uuid::Uuid::new_v4().to_string();
        let target_mode = args.target_mode.unwrap_or(PolicyMode::Default);
        let text = format!("Plan exit requested: {}", args.summary);
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text,
            data: json!({
                "policy_exit_plan_request": {
                    "request_id": request_id,
                    "target_mode": target_mode,
                    "summary": args.summary,
                    "next_steps": args.next_steps,
                    "pending": true
                }
            }),
            is_error: false,
        })
    }
}

fn parse_arguments(arguments: Value) -> anyhow::Result<ExitPlanArguments> {
    let args: ExitPlanArguments = serde_json::from_value(arguments)?;
    if args.summary.trim().is_empty() {
        bail!("summary must not be empty");
    }
    Ok(args)
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExitPlanArguments {
    summary: String,
    #[serde(default)]
    next_steps: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_target_mode")]
    target_mode: Option<PolicyMode>,
}

fn deserialize_target_mode<'de, D>(deserializer: D) -> Result<Option<PolicyMode>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    value
        .map(|value| match value.as_str() {
            "default" => Ok(PolicyMode::Default),
            "accept_edits" => Ok(PolicyMode::AcceptEdits),
            other => Err(serde::de::Error::custom(format!(
                "unsupported target_mode {other:?}"
            ))),
        })
        .transpose()
}
