use std::path::Path;
use std::sync::Arc;

use roder_api::extension::ToolProviderId;
use roder_api::tools::{
    ToolCall, ToolContributor, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult,
    ToolSpec,
};
use serde_json::json;

use crate::command::ZeroCommandRunner;
use crate::patch::build_patch_text;
use crate::types::ZerolangConfig;

mod args;
mod support;
#[cfg(test)]
mod tests;

use args::{EditArgs, FixPlanArgs, GraphOutputArgs, InputArgs, SkillsGetArgs};
use support::{
    error_result, graph_output_spec, non_empty, operation_schema, push_target_emit,
    read_source_if_present, run_json_tool, run_text_tool, source_hunks, source_path,
    validate_graph_output_path, workspace_root,
};

pub const ZEROLANG_SKILLS_GET_TOOL: &str = "zerolang_skills_get";
pub const ZEROLANG_CHECK_TOOL: &str = "zerolang_check";
pub const ZEROLANG_GRAPH_DUMP_TOOL: &str = "zerolang_graph_dump";
pub const ZEROLANG_GRAPH_VIEW_TOOL: &str = "zerolang_graph_view";
pub const ZEROLANG_FIX_PLAN_TOOL: &str = "zerolang_fix_plan";
pub const ZEROLANG_EDIT_TOOL: &str = "zerolang_edit";
pub const ZEROLANG_GRAPH_ROUNDTRIP_TOOL: &str = "zerolang_graph_roundtrip";

#[derive(Debug, Clone, Default)]
pub struct ZerolangToolContributor {
    config: ZerolangConfig,
}

impl ZerolangToolContributor {
    pub fn new(config: ZerolangConfig) -> Self {
        Self { config }
    }
}

impl ToolContributor for ZerolangToolContributor {
    fn id(&self) -> ToolProviderId {
        "zerolang".to_string()
    }

    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        let config = self.config.clone();
        registry.register(Arc::new(SkillsGetTool::new(config.clone())))?;
        registry.register(Arc::new(CheckTool::new(config.clone())))?;
        registry.register(Arc::new(GraphDumpTool::new(config.clone())))?;
        registry.register(Arc::new(GraphViewTool::new(config.clone())))?;
        registry.register(Arc::new(FixPlanTool::new(config.clone())))?;
        registry.register(Arc::new(EditTool::new(config.clone())))?;
        registry.register(Arc::new(GraphRoundtripTool::new(config)))
    }
}

macro_rules! tool_type {
    ($($type:ident),+ $(,)?) => {
        $(
            #[derive(Debug, Clone)]
            struct $type {
                runner: ZeroCommandRunner,
            }

            impl $type {
                fn new(config: ZerolangConfig) -> Self {
                    Self {
                        runner: ZeroCommandRunner::new(config),
                    }
                }
            }
        )+
    };
}

tool_type!(
    SkillsGetTool,
    CheckTool,
    GraphDumpTool,
    GraphViewTool,
    FixPlanTool,
    EditTool,
    GraphRoundtripTool,
);

#[async_trait::async_trait]
impl ToolExecutor for SkillsGetTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: ZEROLANG_SKILLS_GET_TOOL.to_string(),
            description:
                "Read bundled Zero skill documentation such as language, graph, diagnostics, or agent."
                    .to_string(),
            parameters: json!({
                "type": "object",
                "required": [],
                "properties": {
                    "skill": { "type": "string" },
                    "full": { "type": "boolean" }
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
        let args: SkillsGetArgs = serde_json::from_value(call.arguments.clone())?;
        let skill = args
            .skill
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("zero")
            .to_string();
        let mut argv = vec!["skills".to_string(), "get".to_string(), skill];
        if args.full {
            argv.push("--full".to_string());
        }
        run_text_tool(call, &self.runner, argv, workspace_root(&ctx)).await
    }
}

#[async_trait::async_trait]
impl ToolExecutor for CheckTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: ZEROLANG_CHECK_TOOL.to_string(),
            description:
                "Run `zero check --json` for a Zero source file, package, or manifest and return diagnostics."
                    .to_string(),
            parameters: json!({
                "type": "object",
                "required": ["input"],
                "properties": {
                    "input": { "type": "string" },
                    "target": { "type": "string" },
                    "emit": { "type": "string", "enum": ["exe", "obj"] }
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
        let args: InputArgs = serde_json::from_value(call.arguments.clone())?;
        let mut argv = vec!["check".to_string(), "--json".to_string()];
        push_target_emit(&mut argv, args.target, args.emit);
        argv.push(args.input);
        run_json_tool(call, &self.runner, argv, workspace_root(&ctx)).await
    }
}

#[async_trait::async_trait]
impl ToolExecutor for GraphDumpTool {
    fn spec(&self) -> ToolSpec {
        graph_output_spec(
            ZEROLANG_GRAPH_DUMP_TOOL,
            "Run `zero graph dump --json` and optionally write a derived ProgramGraph artifact.",
        )
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args: GraphOutputArgs = serde_json::from_value(call.arguments.clone())?;
        validate_graph_output_path(args.out.as_deref(), args.allow_outside_zero)?;
        let mut argv = vec![
            "graph".to_string(),
            "dump".to_string(),
            "--json".to_string(),
        ];
        if let Some(target) = non_empty(args.target) {
            argv.extend(["--target".to_string(), target]);
        }
        if let Some(out) = non_empty(args.out) {
            argv.extend(["--out".to_string(), out]);
        }
        argv.push(args.input);
        run_json_tool(call, &self.runner, argv, workspace_root(&ctx)).await
    }
}

#[async_trait::async_trait]
impl ToolExecutor for GraphViewTool {
    fn spec(&self) -> ToolSpec {
        graph_output_spec(
            ZEROLANG_GRAPH_VIEW_TOOL,
            "Run `zero graph view --json` to render canonical Zero source from source or a ProgramGraph artifact.",
        )
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args: GraphOutputArgs = serde_json::from_value(call.arguments.clone())?;
        validate_graph_output_path(args.out.as_deref(), args.allow_outside_zero)?;
        if let Some(out) = args
            .out
            .as_deref()
            .map(str::trim)
            .filter(|out| !out.is_empty())
            && Path::new(out).extension().and_then(|ext| ext.to_str()) != Some("0")
        {
            return Ok(error_result(
                call,
                "zerolang_graph_view out must use .0 extension".to_string(),
                json!({}),
            ));
        }
        let mut argv = vec![
            "graph".to_string(),
            "view".to_string(),
            "--json".to_string(),
        ];
        if let Some(target) = non_empty(args.target) {
            argv.extend(["--target".to_string(), target]);
        }
        if let Some(out) = non_empty(args.out) {
            argv.extend(["--out".to_string(), out]);
        }
        argv.push(args.input);
        run_json_tool(call, &self.runner, argv, workspace_root(&ctx)).await
    }
}

#[async_trait::async_trait]
impl ToolExecutor for FixPlanTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: ZEROLANG_FIX_PLAN_TOOL.to_string(),
            description: "Run `zero fix --plan --json` and return Zero's typed repair plan."
                .to_string(),
            parameters: json!({
                "type": "object",
                "required": ["input"],
                "properties": {
                    "input": { "type": "string" },
                    "target": { "type": "string" }
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
        let args: FixPlanArgs = serde_json::from_value(call.arguments.clone())?;
        let mut argv = vec![
            "fix".to_string(),
            "--plan".to_string(),
            "--json".to_string(),
        ];
        if let Some(target) = non_empty(args.target) {
            argv.extend(["--target".to_string(), target]);
        }
        argv.push(args.input);
        run_json_tool(call, &self.runner, argv, workspace_root(&ctx)).await
    }
}

#[async_trait::async_trait]
impl ToolExecutor for EditTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: ZEROLANG_EDIT_TOOL.to_string(),
            description:
                "Apply checked Zero ProgramGraph edits by generating patch text with graphHash and node/value preconditions, then validating source."
                    .to_string(),
            parameters: json!({
                "type": "object",
                "required": ["input", "graphHash", "operations"],
                "properties": {
                    "input": { "type": "string" },
                    "graphHash": { "type": "string" },
                    "operations": {
                        "type": "array",
                        "items": operation_schema(),
                        "minItems": 1
                    },
                    "out": { "type": "string" },
                    "allowOutsideZero": { "type": "boolean" },
                    "validate": { "type": "boolean" }
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
        let args: EditArgs = serde_json::from_value(call.arguments.clone())?;
        validate_graph_output_path(args.out.as_deref(), args.allow_outside_zero)?;
        let patch_text = match build_patch_text(&args.graph_hash, &args.operations) {
            Ok(patch_text) => patch_text,
            Err(err) => return Ok(error_result(call, err.to_string(), json!({}))),
        };
        let cwd = workspace_root(&ctx);
        let source_path = source_path(cwd.as_deref(), &args.input);
        let before = read_source_if_present(&source_path);
        let mut argv = vec![
            "graph".to_string(),
            "patch".to_string(),
            "--json".to_string(),
        ];
        if let Some(out) = non_empty(args.out.clone()) {
            argv.extend(["--out".to_string(), out]);
        }
        argv.push(args.input.clone());
        argv.extend(["--patch-text".to_string(), patch_text.clone()]);
        let patch_output = match self.runner.run(&argv, cwd.as_deref(), true).await {
            Ok(output) => output,
            Err(err) => {
                return Ok(error_result(
                    call,
                    err.to_string(),
                    json!({ "patchText": patch_text }),
                ));
            }
        };
        let patch_failed = !patch_output.success();
        let after = read_source_if_present(&source_path);
        let hunks = source_hunks(&source_path, before.as_deref(), after.as_deref());
        let mut validations = Vec::new();
        if args.validate
            && !patch_failed
            && source_path.extension().and_then(|ext| ext.to_str()) == Some("0")
        {
            validations.push(
                self.runner
                    .run(
                        &[
                            "graph".to_string(),
                            "check".to_string(),
                            "--json".to_string(),
                            args.input.clone(),
                        ],
                        cwd.as_deref(),
                        true,
                    )
                    .await,
            );
            validations.push(
                self.runner
                    .run(
                        &["check".to_string(), "--json".to_string(), args.input],
                        cwd.as_deref(),
                        true,
                    )
                    .await,
            );
        }
        let mut validation_outputs = Vec::new();
        let mut validation_error = None;
        for validation in validations {
            match validation {
                Ok(output) => {
                    if !output.success() {
                        validation_error =
                            Some(format!("zero validation failed: {}", output.stderr.trim()));
                    }
                    validation_outputs.push(json!(output));
                }
                Err(err) => validation_error = Some(err.to_string()),
            }
        }
        let is_error = patch_failed || validation_error.is_some();
        let text = if let Some(error) = validation_error {
            error
        } else if patch_failed {
            format!(
                "zerolang graph patch failed: {}",
                patch_output.stderr.trim()
            )
        } else {
            "zerolang checked graph edit applied".to_string()
        };
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text,
            data: json!({
                "zerolang": {
                    "patchText": patch_text,
                    "command": patch_output,
                    "validations": validation_outputs,
                    "hunks": hunks
                }
            }),
            is_error,
        })
    }
}

#[async_trait::async_trait]
impl ToolExecutor for GraphRoundtripTool {
    fn spec(&self) -> ToolSpec {
        graph_output_spec(
            ZEROLANG_GRAPH_ROUNDTRIP_TOOL,
            "Run `zero graph roundtrip --json` to verify graph/source semantic stability.",
        )
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args: GraphOutputArgs = serde_json::from_value(call.arguments.clone())?;
        validate_graph_output_path(args.out.as_deref(), args.allow_outside_zero)?;
        let mut argv = vec![
            "graph".to_string(),
            "roundtrip".to_string(),
            "--json".to_string(),
        ];
        if let Some(target) = non_empty(args.target) {
            argv.extend(["--target".to_string(), target]);
        }
        if let Some(out) = non_empty(args.out) {
            argv.extend(["--out".to_string(), out]);
        }
        argv.push(args.input);
        run_json_tool(call, &self.runner, argv, workspace_root(&ctx)).await
    }
}
