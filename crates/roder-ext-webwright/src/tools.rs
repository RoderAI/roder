mod support;

use std::fs;
use std::sync::Arc;

use roder_api::extension::ToolProviderId;
use roder_api::tools::{
    ToolCall, ToolContributor, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult,
    ToolSpec,
};
use serde::Deserialize;
use serde_json::json;

use crate::verify::verify_workspace;
use crate::workspace::{
    FINAL_LOG_FILE, FINAL_SCRIPT_FILE, WebwrightManifest, WebwrightMode, WebwrightWorkspace,
    sanitize_task_id,
};
use support::{
    allocate_next_run, default_python_for_workspace, error_result, lint_script_text,
    read_tail_lines, resolve_workspace_arg, run_final_script, workspace_path, workspace_tool_spec,
};

pub const WEBWRIGHT_PREPARE_WORKSPACE_TOOL: &str = "webwright.prepare_workspace";
pub const WEBWRIGHT_ALLOCATE_RUN_TOOL: &str = "webwright.allocate_run";
pub const WEBWRIGHT_LINT_SCRIPT_TOOL: &str = "webwright.lint_script";
pub const WEBWRIGHT_RUN_SCRIPT_TOOL: &str = "webwright.run_script";
pub const WEBWRIGHT_LIST_ARTIFACTS_TOOL: &str = "webwright.list_artifacts";
pub const WEBWRIGHT_READ_LOG_TAIL_TOOL: &str = "webwright.read_log_tail";
pub const WEBWRIGHT_VERIFY_RUN_TOOL: &str = "webwright.verify_run";
pub const WEBWRIGHT_SUMMARIZE_VERIFICATION_TOOL: &str = "webwright.summarize_verification";

#[derive(Debug, Default)]
pub struct WebwrightToolContributor;

impl ToolContributor for WebwrightToolContributor {
    fn id(&self) -> ToolProviderId {
        "webwright".to_string()
    }

    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        registry.register(Arc::new(PrepareWorkspaceTool))?;
        registry.register(Arc::new(AllocateRunTool))?;
        registry.register(Arc::new(LintScriptTool))?;
        registry.register(Arc::new(RunScriptTool))?;
        registry.register(Arc::new(ListArtifactsTool))?;
        registry.register(Arc::new(ReadLogTailTool))?;
        registry.register(Arc::new(VerifyRunTool))?;
        registry.register(Arc::new(SummarizeVerificationTool))
    }
}

#[derive(Debug)]
struct PrepareWorkspaceTool;

#[derive(Debug)]
struct AllocateRunTool;

#[derive(Debug)]
struct LintScriptTool;

#[derive(Debug)]
struct RunScriptTool;

#[derive(Debug)]
struct ListArtifactsTool;

#[derive(Debug)]
struct ReadLogTailTool;

#[derive(Debug)]
struct VerifyRunTool;

#[derive(Debug)]
struct SummarizeVerificationTool;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrepareWorkspaceArgs {
    task: String,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    task_id: Option<String>,
    #[serde(default)]
    start_url: Option<String>,
    #[serde(default)]
    output_dir: Option<String>,
    #[serde(default)]
    browser: Option<String>,
    #[serde(default)]
    headless: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspacePathArgs {
    workspace: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScriptPathArgs {
    workspace: String,
    #[serde(default)]
    script_path: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunScriptArgs {
    workspace: String,
    #[serde(default)]
    python: Option<String>,
    #[serde(default)]
    timeout_seconds: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadLogTailArgs {
    workspace: String,
    #[serde(default)]
    max_lines: Option<usize>,
}

#[async_trait::async_trait]
impl ToolExecutor for PrepareWorkspaceTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: WEBWRIGHT_PREPARE_WORKSPACE_TOOL.to_string(),
            description:
                "Create a path-scoped Webwright workspace with starter plan and script files."
                    .to_string(),
            parameters: json!({
                "type": "object",
                "required": ["task"],
                "properties": {
                    "task": { "type": "string" },
                    "mode": { "type": "string", "enum": ["run", "craft"] },
                    "taskId": { "type": "string" },
                    "startUrl": { "type": "string" },
                    "outputDir": { "type": "string" },
                    "browser": { "type": "string" },
                    "headless": { "type": "boolean" }
                },
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args: PrepareWorkspaceArgs = serde_json::from_value(call.arguments.clone())?;
        if args.task.trim().is_empty() {
            return Ok(error_result(call, "webwright task must not be empty"));
        }
        let mode = WebwrightMode::parse(args.mode.as_deref().unwrap_or("run"))?;
        let task_id = args
            .task_id
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| sanitize_task_id(&args.task));
        let root = workspace_path(&ctx, args.output_dir.as_deref(), &task_id)?;
        let manifest = WebwrightManifest::new(
            task_id.clone(),
            args.task,
            mode,
            args.start_url,
            args.browser,
            args.headless.unwrap_or(true),
        );
        let workspace = WebwrightWorkspace::new(&root);
        workspace.create(&manifest)?;
        workspace.ensure_starter_files(&manifest)?;
        let summary = workspace.summary()?;
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text: format!("prepared Webwright workspace {}", root.display()),
            data: json!({ "webwright": { "taskId": task_id, "workspace": summary } }),
            is_error: false,
        })
    }
}

#[async_trait::async_trait]
impl ToolExecutor for AllocateRunTool {
    fn spec(&self) -> ToolSpec {
        workspace_tool_spec(
            WEBWRIGHT_ALLOCATE_RUN_TOOL,
            "Allocate the next Webwright final_runs/run_<n> directory and copy final_script.py.",
        )
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args: WorkspacePathArgs = serde_json::from_value(call.arguments.clone())?;
        let root = resolve_workspace_arg(&ctx, &args.workspace)?;
        let workspace = WebwrightWorkspace::new(&root);
        let run = allocate_next_run(&workspace)?;
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text: format!(
                "allocated Webwright run_{:03} at {}",
                run.run_id, run.run_dir
            ),
            data: json!({ "webwright": { "run": run, "workspace": workspace.summary()? } }),
            is_error: false,
        })
    }
}

#[async_trait::async_trait]
impl ToolExecutor for LintScriptTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: WEBWRIGHT_LINT_SCRIPT_TOOL.to_string(),
            description:
                "Lint a Webwright script for screenshot-policy and runnable-script issues."
                    .to_string(),
            parameters: json!({
                "type": "object",
                "required": ["workspace"],
                "properties": {
                    "workspace": { "type": "string" },
                    "scriptPath": { "type": "string" }
                },
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args: ScriptPathArgs = serde_json::from_value(call.arguments.clone())?;
        let root = resolve_workspace_arg(&ctx, &args.workspace)?;
        let workspace = WebwrightWorkspace::new(&root);
        let script_path = workspace.resolve_inside(
            args.script_path
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(FINAL_SCRIPT_FILE),
        )?;
        let text = fs::read_to_string(&script_path)?;
        let lint = lint_script_text(&text);
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text: if lint.passed {
                format!("webwright script lint passed: {}", script_path.display())
            } else {
                format!(
                    "webwright script lint failed: {}",
                    lint.checks
                        .iter()
                        .filter(|check| !check.passed)
                        .map(|check| check.message.as_str())
                        .collect::<Vec<_>>()
                        .join("; ")
                )
            },
            data: json!({ "webwright": { "scriptPath": script_path, "lint": lint } }),
            is_error: !lint.passed,
        })
    }
}

#[async_trait::async_trait]
impl ToolExecutor for RunScriptTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: WEBWRIGHT_RUN_SCRIPT_TOOL.to_string(),
            description:
                "Allocate a Webwright run directory and execute its copied final_script.py."
                    .to_string(),
            parameters: json!({
                "type": "object",
                "required": ["workspace"],
                "properties": {
                    "workspace": { "type": "string" },
                    "python": { "type": "string" },
                    "timeoutSeconds": { "type": "integer", "minimum": 1 }
                },
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        ctx.require_process_runner()?;
        let args: RunScriptArgs = serde_json::from_value(call.arguments.clone())?;
        let root = resolve_workspace_arg(&ctx, &args.workspace)?;
        let workspace = WebwrightWorkspace::new(&root);
        let run = allocate_next_run(&workspace)?;
        let interpreter = match args
            .python
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            Some(python) => python.to_string(),
            None => default_python_for_workspace(&ctx, &workspace)?,
        };
        let result =
            run_final_script(&run, &interpreter, args.timeout_seconds.unwrap_or(60)).await?;
        let is_error = result.timed_out || result.exit_code != Some(0);
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text: format!(
                "webwright run_{:03} exited with {:?}",
                result.run_id, result.exit_code
            ),
            data: json!({ "webwright": result }),
            is_error,
        })
    }
}

#[async_trait::async_trait]
impl ToolExecutor for ListArtifactsTool {
    fn spec(&self) -> ToolSpec {
        workspace_tool_spec(
            WEBWRIGHT_LIST_ARTIFACTS_TOOL,
            "List structured Webwright artifacts for a prepared workspace.",
        )
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args: WorkspacePathArgs = serde_json::from_value(call.arguments.clone())?;
        let root = resolve_workspace_arg(&ctx, &args.workspace)?;
        let summary = WebwrightWorkspace::new(&root).summary()?;
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text: format!(
                "webwright workspace {}: {} run(s)",
                root.display(),
                summary.runs.len()
            ),
            data: json!({ "webwright": { "workspace": summary } }),
            is_error: false,
        })
    }
}

#[async_trait::async_trait]
impl ToolExecutor for ReadLogTailTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: WEBWRIGHT_READ_LOG_TAIL_TOOL.to_string(),
            description: "Read the latest Webwright final_script_log.txt tail.".to_string(),
            parameters: json!({
                "type": "object",
                "required": ["workspace"],
                "properties": {
                    "workspace": { "type": "string" },
                    "maxLines": { "type": "integer", "minimum": 1 }
                },
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args: ReadLogTailArgs = serde_json::from_value(call.arguments.clone())?;
        let root = resolve_workspace_arg(&ctx, &args.workspace)?;
        let workspace = WebwrightWorkspace::new(&root);
        let Some(run_id) = workspace.latest_run_id()? else {
            return Ok(error_result(call, "webwright workspace has no final run"));
        };
        let log_path = workspace.run_dir(run_id).join(FINAL_LOG_FILE);
        let lines = read_tail_lines(&log_path, args.max_lines.unwrap_or(20))?;
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text: lines.join("\n"),
            data: json!({
                "webwright": {
                    "runId": run_id,
                    "logPath": log_path,
                    "lines": lines
                }
            }),
            is_error: false,
        })
    }
}

#[async_trait::async_trait]
impl ToolExecutor for VerifyRunTool {
    fn spec(&self) -> ToolSpec {
        workspace_tool_spec(
            WEBWRIGHT_VERIFY_RUN_TOOL,
            "Verify the latest Webwright run against the required artifact contract.",
        )
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args: WorkspacePathArgs = serde_json::from_value(call.arguments.clone())?;
        let root = resolve_workspace_arg(&ctx, &args.workspace)?;
        let verification = verify_workspace(&root);
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text: if verification.passed {
                "webwright verification passed".to_string()
            } else {
                format!(
                    "webwright verification failed: {}",
                    verification
                        .checks
                        .iter()
                        .filter(|check| !check.passed)
                        .map(|check| check.message.as_str())
                        .collect::<Vec<_>>()
                        .join("; ")
                )
            },
            data: json!({ "webwright": { "verification": verification } }),
            is_error: !verification.passed,
        })
    }
}

#[async_trait::async_trait]
impl ToolExecutor for SummarizeVerificationTool {
    fn spec(&self) -> ToolSpec {
        workspace_tool_spec(
            WEBWRIGHT_SUMMARIZE_VERIFICATION_TOOL,
            "Summarize Webwright verification state without treating failure as tool failure.",
        )
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args: WorkspacePathArgs = serde_json::from_value(call.arguments.clone())?;
        let root = resolve_workspace_arg(&ctx, &args.workspace)?;
        let verification = verify_workspace(&root);
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text: if verification.passed {
                "webwright verification state: success".to_string()
            } else {
                format!(
                    "webwright verification state: failure ({} failing check(s))",
                    verification
                        .checks
                        .iter()
                        .filter(|check| !check.passed)
                        .count()
                )
            },
            data: json!({ "webwright": { "verification": verification } }),
            is_error: false,
        })
    }
}
