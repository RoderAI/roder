use std::fs;
use std::path::{Component, Path, PathBuf};

use roder_api::tools::{ToolCall, ToolExecutionContext, ToolResult, ToolSpec};
use serde_json::{Value, json};

use crate::command::{ZeroCommandOutput, ZeroCommandRunner};

pub(super) async fn run_json_tool(
    call: ToolCall,
    runner: &ZeroCommandRunner,
    argv: Vec<String>,
    cwd: Option<PathBuf>,
) -> anyhow::Result<ToolResult> {
    run_command_tool(call, runner, argv, cwd, true).await
}

pub(super) async fn run_text_tool(
    call: ToolCall,
    runner: &ZeroCommandRunner,
    argv: Vec<String>,
    cwd: Option<PathBuf>,
) -> anyhow::Result<ToolResult> {
    run_command_tool(call, runner, argv, cwd, false).await
}

async fn run_command_tool(
    call: ToolCall,
    runner: &ZeroCommandRunner,
    argv: Vec<String>,
    cwd: Option<PathBuf>,
    parse_json: bool,
) -> anyhow::Result<ToolResult> {
    match runner.run(&argv, cwd.as_deref(), parse_json).await {
        Ok(output) => command_result(call, output),
        Err(err) => Ok(error_result(call, err.to_string(), json!({}))),
    }
}

fn command_result(call: ToolCall, output: ZeroCommandOutput) -> anyhow::Result<ToolResult> {
    let is_error = !output.success();
    let text = if is_error {
        format!("zero command failed: {}", output.stderr.trim())
    } else if let Some(json) = &output.json {
        if let Some(ok) = json.get("ok").and_then(Value::as_bool) {
            format!("zero command completed with ok={ok}")
        } else {
            "zero command completed".to_string()
        }
    } else {
        output.stdout.trim().to_string()
    };
    Ok(ToolResult {
        id: call.id,
        name: call.name,
        text,
        data: json!({ "zerolang": { "command": output } }),
        is_error,
    })
}

pub(super) fn error_result(call: ToolCall, message: String, extra: Value) -> ToolResult {
    ToolResult {
        id: call.id,
        name: call.name,
        text: message.clone(),
        data: json!({ "error": { "kind": "zerolang", "message": message }, "zerolang": extra }),
        is_error: true,
    }
}

pub(super) fn workspace_root(ctx: &ToolExecutionContext) -> Option<PathBuf> {
    ctx.handles
        .workspace
        .as_ref()
        .and_then(|workspace| workspace.workspace_root())
}

pub(super) fn graph_output_spec(name: &str, description: &str) -> ToolSpec {
    ToolSpec {
        name: name.to_string(),
        description: description.to_string(),
        parameters: json!({
            "type": "object",
            "required": ["input"],
            "properties": {
                "input": { "type": "string" },
                "target": { "type": "string" },
                "out": { "type": "string" },
                "allowOutsideZero": { "type": "boolean" }
            },
            "additionalProperties": false
        }),
    }
}

pub(super) fn operation_schema() -> Value {
    json!({
        "type": "object",
        "description": "One structured Roder graph patch operation. Use `node` for ProgramGraph node ids, not `id`; Roder converts this object to Zero patch text.",
        "required": ["op"],
        "properties": {
            "op": {
                "type": "string",
                "enum": ["set", "rename", "insert", "insertEdge", "replace", "delete"],
                "description": "Operation kind. For literal/value updates use `set`."
            },
            "node": {
                "type": "string",
                "description": "ProgramGraph node id from zerolang_graph_dump, for example `#89f1bc7e`. Do not use `id`."
            },
            "field": {
                "type": "string",
                "description": "Field name to edit for set operations, for example `value`."
            },
            "expect": {
                "type": "string",
                "description": "Optional checked precondition. For set/rename this is the expected existing field value; for delete/replace it is the expected node hash."
            },
            "value": {
                "type": "string",
                "description": "New semantic value as a string. Quote numeric values, for example use `\"66\"`, not number 66.",
                "examples": ["66"]
            },
            "kind": {
                "type": "string",
                "description": "Node kind for insert or replace operations."
            },
            "parent": {
                "type": "string",
                "description": "Parent node id for insert operations."
            },
            "edge": {
                "type": "string",
                "description": "ProgramGraph edge name for insert or insertEdge operations."
            },
            "order": {
                "type": "integer",
                "minimum": 0,
                "description": "Zero-based edge order for insert or insertEdge operations."
            },
            "name": {
                "type": "string",
                "description": "Optional node name attribute for insert or replace operations."
            },
            "type": {
                "type": "string",
                "description": "Optional node type attribute for insert or replace operations."
            },
            "path": {
                "type": "string",
                "description": "Optional path attribute for insert or replace operations."
            },
            "line": {
                "type": "integer",
                "minimum": 0,
                "description": "Optional zero-based line attribute for insert or replace operations."
            },
            "column": {
                "type": "integer",
                "minimum": 0,
                "description": "Optional zero-based column attribute for insert or replace operations."
            },
            "from": {
                "type": "string",
                "description": "Source edge-owner node id for insertEdge operations."
            },
            "to": {
                "type": "string",
                "description": "Node id that receives the inserted edge for insertEdge operations."
            },
            "target": {
                "type": "string",
                "description": "Target node id for insertEdge operations."
            },
            "public": {
                "type": "boolean",
                "description": "Optional public flag for insert or replace operations."
            },
            "mutable": {
                "type": "boolean",
                "description": "Optional mutable flag for insert or replace operations."
            },
            "static": {
                "type": "boolean",
                "description": "Optional static flag for insert or replace operations."
            },
            "fallible": {
                "type": "boolean",
                "description": "Optional fallible flag for insert or replace operations."
            },
            "exportC": {
                "type": "boolean",
                "description": "Optional exportC flag for insert or replace operations."
            }
        },
        "examples": [{
            "op": "set",
            "node": "#89f1bc7e",
            "field": "value",
            "expect": "65",
            "value": "66"
        }],
        "additionalProperties": false
    })
}

pub(super) fn push_target_emit(
    argv: &mut Vec<String>,
    target: Option<String>,
    emit: Option<String>,
) {
    if let Some(target) = non_empty(target) {
        argv.extend(["--target".to_string(), target]);
    }
    if let Some(emit) = non_empty(emit) {
        argv.extend(["--emit".to_string(), emit]);
    }
}

pub(super) fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(super) fn validate_graph_output_path(
    path: Option<&str>,
    allow_outside_zero: bool,
) -> anyhow::Result<()> {
    let Some(path) = path.map(str::trim).filter(|path| !path.is_empty()) else {
        return Ok(());
    };
    let path = Path::new(path);
    if path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        anyhow::bail!("zerolang graph output paths must be workspace-relative");
    }
    if !allow_outside_zero && !path.starts_with(".zero") {
        anyhow::bail!(
            "zerolang graph output paths default to .zero/; set allowOutsideZero for another workspace-relative path"
        );
    }
    Ok(())
}

pub(super) fn source_path(cwd: Option<&Path>, input: &str) -> PathBuf {
    let input = PathBuf::from(input);
    if input.is_absolute() {
        input
    } else if let Some(cwd) = cwd {
        cwd.join(input)
    } else {
        input
    }
}

pub(super) fn read_source_if_present(path: &Path) -> Option<String> {
    if path.extension().and_then(|ext| ext.to_str()) == Some("0") {
        fs::read_to_string(path).ok()
    } else {
        None
    }
}

pub(super) fn source_hunks(path: &Path, before: Option<&str>, after: Option<&str>) -> Vec<Value> {
    let (Some(before), Some(after)) = (before, after) else {
        return Vec::new();
    };
    if before == after {
        return Vec::new();
    }
    let before_lines = before.lines().map(ToString::to_string).collect::<Vec<_>>();
    let after_lines = after.lines().map(ToString::to_string).collect::<Vec<_>>();
    let mut prefix = 0;
    while prefix < before_lines.len()
        && prefix < after_lines.len()
        && before_lines[prefix] == after_lines[prefix]
    {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < before_lines.len().saturating_sub(prefix)
        && suffix < after_lines.len().saturating_sub(prefix)
        && before_lines[before_lines.len() - suffix - 1]
            == after_lines[after_lines.len() - suffix - 1]
    {
        suffix += 1;
    }
    let context_start = prefix.saturating_sub(2);
    let before_end = before_lines.len().saturating_sub(suffix).min(prefix + 8);
    let after_end = after_lines.len().saturating_sub(suffix).min(prefix + 8);
    vec![json!({
        "path": path.display().to_string(),
        "beforeStart": context_start + 1,
        "afterStart": context_start + 1,
        "beforeLines": before_lines[context_start..before_end].to_vec(),
        "afterLines": after_lines[context_start..after_end].to_vec()
    })]
}
