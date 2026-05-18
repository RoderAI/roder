use std::sync::Arc;

use anyhow::bail;
use roder_api::tools::{
    ToolCall, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult, ToolSpec,
};
use serde::Deserialize;
use serde_json::json;

use crate::backend::WorkspaceBackendHandle;
use crate::paging::{DEFAULT_PAGE_LINES, MAX_PAGE_LINES, clamp_limit, page_lines, page_metadata};

pub(crate) fn register(
    registry: &mut ToolRegistry,
    backend: WorkspaceBackendHandle,
) -> anyhow::Result<()> {
    registry.register(Arc::new(ReadFileTool {
        backend: backend.clone(),
    }))?;
    registry.register(Arc::new(ListFilesTool { backend }))
}

struct ReadFileTool {
    backend: WorkspaceBackendHandle,
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
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        ctx.require_workspace()?;
        let args = parse::<ReadFileArgs>(&call)?;
        let (path, text) = self.backend.read_text(&args.path).await?;
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
                "path": path,
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

struct ListFilesTool {
    backend: WorkspaceBackendHandle,
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
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        ctx.require_workspace()?;
        let args = parse::<ListFilesArgs>(&call)?;
        let (path, names) = self
            .backend
            .list_files(args.path.as_deref().unwrap_or("."))
            .await?;
        let offset = args.offset.unwrap_or_default();
        let limit = clamp_limit(args.limit);
        let page = page_lines(&names, offset, limit);
        let data = page_metadata(path, offset, limit, &page);
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
