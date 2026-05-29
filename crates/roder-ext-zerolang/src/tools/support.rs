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
        "required": ["op"],
        "properties": {
            "op": { "type": "string", "enum": ["set", "rename", "insert", "insertEdge", "replace", "delete"] },
            "node": { "type": "string" },
            "field": { "type": "string" },
            "expect": { "type": "string" },
            "value": { "type": "string" },
            "kind": { "type": "string" },
            "parent": { "type": "string" },
            "edge": { "type": "string" },
            "order": { "type": "integer", "minimum": 0 },
            "name": { "type": "string" },
            "type": { "type": "string" },
            "path": { "type": "string" },
            "line": { "type": "integer", "minimum": 0 },
            "column": { "type": "integer", "minimum": 0 },
            "from": { "type": "string" },
            "to": { "type": "string" },
            "target": { "type": "string" },
            "public": { "type": "boolean" },
            "mutable": { "type": "boolean" },
            "static": { "type": "boolean" },
            "fallible": { "type": "boolean" },
            "exportC": { "type": "boolean" }
        },
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
