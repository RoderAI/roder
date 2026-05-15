use std::sync::Arc;

use anyhow::bail;
use roder_api::tools::{
    ToolCall, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult, ToolSpec,
};
use serde::Deserialize;
use serde_json::json;

use crate::workspace::Workspace;

const DEFAULT_READ_LIMIT: usize = 200;
const MAX_READ_LIMIT: usize = 400;

pub(crate) fn register(registry: &mut ToolRegistry, workspace: Workspace) -> anyhow::Result<()> {
    registry.register(Arc::new(ReadFileTool {
        workspace: workspace.clone(),
    }))?;
    registry.register(Arc::new(ListFilesTool { workspace }))
}

#[derive(Debug)]
struct ReadFileTool {
    workspace: Workspace,
}

#[async_trait::async_trait]
impl ToolExecutor for ReadFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "read_file".to_string(),
            description: "Read a UTF-8 text file inside the workspace, optionally by line range."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "start_line": { "type": "integer", "minimum": 1 },
                    "limit": { "type": "integer", "minimum": 1, "maximum": MAX_READ_LIMIT }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args = parse::<ReadFileArgs>(&call)?;
        let path = self.workspace.resolve_existing(&args.path)?;
        let text = std::fs::read_to_string(&path)?;
        let start_line = args.start_line.unwrap_or(1).max(1);
        let limit = args
            .limit
            .unwrap_or(DEFAULT_READ_LIMIT)
            .clamp(1, MAX_READ_LIMIT);
        let body = text
            .lines()
            .enumerate()
            .skip(start_line - 1)
            .take(limit)
            .map(|(index, line)| format!("{:>5}: {}", index + 1, line))
            .collect::<Vec<_>>()
            .join("\n");
        Ok(result(
            call,
            body,
            json!({ "path": self.workspace.display(&path), "start_line": start_line, "limit": limit }),
            false,
        ))
    }
}

#[derive(Debug)]
struct ListFilesTool {
    workspace: Workspace,
}

#[async_trait::async_trait]
impl ToolExecutor for ListFilesTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "list_files".to_string(),
            description: "List direct children of a workspace directory.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "default": "." }
                },
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args = parse::<ListFilesArgs>(&call)?;
        let path = self
            .workspace
            .resolve_existing(args.path.as_deref().unwrap_or("."))?;
        let mut names = Vec::new();
        for entry in std::fs::read_dir(&path)? {
            let entry = entry?;
            let mut name = entry.file_name().to_string_lossy().to_string();
            if entry.file_type()?.is_dir() {
                name.push('/');
            }
            names.push(name);
        }
        names.sort();
        Ok(result(
            call,
            names.join("\n"),
            json!({ "path": self.workspace.display(&path), "entries": names }),
            false,
        ))
    }
}

#[derive(Deserialize)]
struct ReadFileArgs {
    path: String,
    start_line: Option<usize>,
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct ListFilesArgs {
    path: Option<String>,
}

pub(crate) fn parse<T: for<'de> Deserialize<'de>>(call: &ToolCall) -> anyhow::Result<T> {
    serde_json::from_value(call.arguments.clone()).map_err(Into::into)
}

pub(crate) fn result(
    call: ToolCall,
    text: String,
    data: serde_json::Value,
    is_error: bool,
) -> ToolResult {
    ToolResult {
        id: call.id,
        name: call.name,
        text,
        data,
        is_error,
    }
}

pub(crate) fn require_nonempty(value: &str, name: &str) -> anyhow::Result<()> {
    if value.is_empty() {
        bail!("{name} is required");
    }
    Ok(())
}
