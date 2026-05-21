use std::sync::Arc;

use anyhow::bail;
use roder_api::tools::{
    ToolCall, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult, ToolSpec,
};
use serde::Deserialize;
use serde_json::json;

use crate::backend::{WorkspaceBackendHandle, backend_from_context_or_fallback};
use crate::paging::{
    DEFAULT_PAGE_LINES, MAX_PAGE_LINES, append_continuation_instruction, clamp_limit,
    omitted_lines, page_lines, page_metadata_with_continuation,
};
use crate::workspace::Workspace;

pub(crate) fn register(
    registry: &mut ToolRegistry,
    workspace: Workspace,
    backend: WorkspaceBackendHandle,
) -> anyhow::Result<()> {
    registry.register(Arc::new(ReadFileTool {
        workspace: workspace.clone(),
        backend: backend.clone(),
    }))?;
    registry.register(Arc::new(ListFilesTool { workspace, backend }))
}

struct ReadFileTool {
    workspace: Workspace,
    backend: WorkspaceBackendHandle,
}

#[async_trait::async_trait]
impl ToolExecutor for ReadFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "read_file".to_string(),
            description: "Read a UTF-8 text file, optionally by line range. Relative paths resolve from the workspace root."
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
        let backend = backend_from_context_or_fallback(&ctx, &self.workspace, &self.backend)?;
        let (path, text) = backend.read_text(&args.path).await?;
        let start_line = args.start_line.unwrap_or(1).max(1);
        let limit = clamp_limit(args.limit);
        let lines = text
            .lines()
            .enumerate()
            .map(|(index, line)| format!("{:>5}: {}", index + 1, line))
            .collect::<Vec<_>>();
        let page = page_lines(&lines, start_line - 1, limit);
        let next_start_line = page.next_offset.map(|offset| offset + 1);
        let continuation_args = next_start_line.map(|next| {
            json!({
                "path": path.clone(),
                "start_line": next,
                "limit": limit,
            })
        });
        let mut text = page.text.clone();
        if let Some(args) = continuation_args.as_ref() {
            append_continuation_instruction(&mut text, &page, "read_file", args);
        }
        Ok(result(
            call,
            text,
            json!({
                "path": path,
                "start_line": start_line,
                "limit": limit,
                "shown": page.shown,
                "total_lines": page.total,
                "omitted_lines": omitted_lines(&page),
                "next_start_line": next_start_line,
                "truncated": next_start_line.is_some(),
                "continuation_tool": if next_start_line.is_some() { json!("read_file") } else { serde_json::Value::Null },
                "continuation_args": continuation_args.unwrap_or(serde_json::Value::Null),
            }),
            false,
        ))
    }
}

struct ListFilesTool {
    workspace: Workspace,
    backend: WorkspaceBackendHandle,
}

#[async_trait::async_trait]
impl ToolExecutor for ListFilesTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "list_files".to_string(),
            description: "List direct children of a directory with paginated output. Relative paths resolve from the workspace root."
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
        let backend = backend_from_context_or_fallback(&ctx, &self.workspace, &self.backend)?;
        let (path, names) = backend
            .list_files(args.path.as_deref().unwrap_or("."))
            .await?;
        let offset = args.offset.unwrap_or_default();
        let limit = clamp_limit(args.limit);
        let page = page_lines(&names, offset, limit);
        let continuation_args = page.next_offset.map(|next| {
            json!({
                "path": path.clone(),
                "offset": next,
                "limit": limit,
            })
        });
        let mut text = page.text.clone();
        if let Some(args) = continuation_args.as_ref() {
            append_continuation_instruction(&mut text, &page, "list_files", args);
        }
        let data = page_metadata_with_continuation(
            path,
            offset,
            limit,
            &page,
            "list_files",
            continuation_args.unwrap_or(serde_json::Value::Null),
        );
        Ok(result(call, text, data, false))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::LocalWorkspaceBackend;
    use roder_api::tools::{LocalWorkspaceHandle, ToolExecutionContext};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    #[tokio::test]
    async fn read_file_paging_result_includes_continuation_text_and_data() {
        let root = test_workspace("read-file-paging");
        let file = root.join("notes.txt");
        std::fs::write(&file, "one\ntwo\nthree\n").unwrap();
        let workspace = Workspace::new(root.clone()).unwrap();
        let tool = ReadFileTool {
            workspace: workspace.clone(),
            backend: Arc::new(LocalWorkspaceBackend::new(workspace)),
        };

        let result = tool
            .execute(
                context(&root),
                call(
                    "read_file",
                    json!({"path": "notes.txt", "start_line": 1, "limit": 2}),
                ),
            )
            .await
            .unwrap();

        assert!(result.text.contains("next_offset=2"));
        assert!(result.text.contains("call read_file"));
        assert!(result.text.contains("\"start_line\":3"));
        assert_eq!(result.data["omitted_lines"], 1);
        assert_eq!(result.data["next_start_line"], 3);
        assert_eq!(result.data["continuation_tool"], "read_file");
        assert_eq!(result.data["continuation_args"]["start_line"], 3);

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn list_files_paging_result_includes_continuation_text_and_data() {
        let root = test_workspace("list-files-paging");
        let dir = root.join("src");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.rs"), "").unwrap();
        std::fs::write(dir.join("b.rs"), "").unwrap();
        let workspace = Workspace::new(root.clone()).unwrap();
        let tool = ListFilesTool {
            workspace: workspace.clone(),
            backend: Arc::new(LocalWorkspaceBackend::new(workspace)),
        };

        let result = tool
            .execute(
                context(&root),
                call("list_files", json!({"path": "src", "limit": 1})),
            )
            .await
            .unwrap();

        assert!(result.text.contains("call list_files"));
        assert!(result.text.contains("\"offset\":1"));
        assert_eq!(result.data["omitted_lines"], 1);
        assert_eq!(result.data["continuation_tool"], "list_files");
        assert_eq!(result.data["continuation_args"]["offset"], 1);

        let _ = std::fs::remove_dir_all(root);
    }

    fn context(workspace: &Path) -> ToolExecutionContext {
        ToolExecutionContext::new(
            "thread-a",
            "turn-a",
            roder_api::policy_mode::PolicyMode::Default,
        )
        .with_workspace_handle(Arc::new(LocalWorkspaceHandle::new(workspace)))
    }

    fn call(name: &str, arguments: serde_json::Value) -> ToolCall {
        ToolCall {
            id: format!("call-{name}"),
            name: name.to_string(),
            raw_arguments: arguments.to_string(),
            arguments,
            thread_id: "thread-a".to_string(),
            turn_id: "turn-a".to_string(),
        }
    }

    fn test_workspace(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("roder-tools-{name}-{stamp}"));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }
}
