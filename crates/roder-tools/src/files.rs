use std::sync::Arc;

use anyhow::bail;
use roder_api::tools::{
    ToolCall, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult, ToolSpec,
};
use serde::Deserialize;
use serde_json::json;

use crate::paging::{DEFAULT_PAGE_LINES, MAX_PAGE_LINES, clamp_limit, page_lines, page_metadata};
use crate::workspace::Workspace;

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
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": MAX_PAGE_LINES,
                        "default": DEFAULT_PAGE_LINES,
                        "description": "Maximum number of lines to return. Use start_line from the response to continue reading."
                    }
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
        let limit = clamp_limit(args.limit);
        let lines = text
            .lines()
            .enumerate()
            .map(|(index, line)| format!("{:>5}: {}", index + 1, line))
            .collect::<Vec<_>>();
        let page = page_lines(&lines, start_line - 1, limit);
        let next_start_line = page.next_offset.map(|offset| offset + 1);
        Ok(result(
            call,
            page.text,
            json!({
                "path": self.workspace.display(&path),
                "start_line": start_line,
                "limit": limit,
                "shown": page.shown,
                "total_lines": page.total,
                "next_start_line": next_start_line,
                "truncated": next_start_line.is_some(),
            }),
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
            description: "List direct children of a workspace directory with paginated output."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "default": "." },
                    "offset": {
                        "type": "integer",
                        "minimum": 0,
                        "default": 0,
                        "description": "Zero-based line offset for pagination."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": MAX_PAGE_LINES,
                        "default": DEFAULT_PAGE_LINES,
                        "description": "Maximum number of entries to return."
                    }
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
        let offset = args.offset.unwrap_or_default();
        let limit = clamp_limit(args.limit);
        let page = page_lines(&names, offset, limit);
        let data = page_metadata(self.workspace.display(&path), offset, limit, &page);
        Ok(result(call, page.text, data, false))
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
    offset: Option<usize>,
    limit: Option<usize>,
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
