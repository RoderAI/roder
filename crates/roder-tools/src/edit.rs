use std::sync::Arc;

use roder_api::tools::{
    ToolCall, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult, ToolSpec,
};
use serde::Deserialize;
use serde_json::json;

use crate::backend::{TextEdit, WorkspaceBackendHandle, backend_from_context_or_fallback};
use crate::files::{parse, require_nonempty, result};
use crate::hunk_output;
use crate::workspace::Workspace;

pub(crate) fn register(
    registry: &mut ToolRegistry,
    workspace: Workspace,
    backend: WorkspaceBackendHandle,
) -> anyhow::Result<()> {
    registry.register(Arc::new(WriteFileTool {
        workspace: workspace.clone(),
        backend: backend.clone(),
    }))?;
    registry.register(Arc::new(EditTool {
        workspace: workspace.clone(),
        backend: backend.clone(),
    }))?;
    registry.register(Arc::new(MultiEditTool { workspace, backend }))
}

struct WriteFileTool {
    workspace: Workspace,
    backend: WorkspaceBackendHandle,
}

#[async_trait::async_trait]
impl ToolExecutor for WriteFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "write_file".to_string(),
            description: "Write a UTF-8 text file. Relative paths resolve from the workspace root."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["path", "content"],
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
        let args = parse::<WriteFileArgs>(&call)?;
        let backend = backend_from_context_or_fallback(&ctx, &self.workspace, &self.backend)?;
        let rel = backend.write_text(&args.path, args.content).await?;
        Ok(result(
            call,
            format!("wrote {rel}"),
            json!({ "path": rel }),
            false,
        ))
    }
}

struct EditTool {
    workspace: Workspace,
    backend: WorkspaceBackendHandle,
}

#[async_trait::async_trait]
impl ToolExecutor for EditTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "edit".to_string(),
            description: "Replace one exact text range inside a file. Relative paths resolve from the workspace root."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "old_string": { "type": "string" },
                    "new_string": { "type": "string" }
                },
                "required": ["path", "old_string", "new_string"],
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
        let args = parse::<EditArgs>(&call)?;
        require_nonempty(&args.old_string, "old_string")?;
        let backend = backend_from_context_or_fallback(&ctx, &self.workspace, &self.backend)?;
        let Some(outcome) = backend
            .edit_text(&args.path, &args.old_string, &args.new_string)
            .await?
        else {
            return Ok(result(
                call,
                "old_string does not match file".to_string(),
                json!({ "error": { "kind": "old_string_not_found" } }),
                true,
            ));
        };
        let hunks = vec![hunk_output::record(
            &ctx,
            &call,
            0,
            outcome.path.clone(),
            args.old_string.lines().map(str::to_string).collect(),
            args.new_string.lines().map(str::to_string).collect(),
        )];
        Ok(result(
            call,
            format!("edited {}", outcome.path),
            json!({ "path": outcome.path, "replacements": outcome.replacements, "hunks": hunks }),
            false,
        ))
    }
}

struct MultiEditTool {
    workspace: Workspace,
    backend: WorkspaceBackendHandle,
}

#[async_trait::async_trait]
impl ToolExecutor for MultiEditTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "multi_edit".to_string(),
            description: "Apply multiple exact text replacements to one file. Relative paths resolve from the workspace root."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "edits": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "old_string": { "type": "string" },
                                "new_string": { "type": "string" }
                            },
                            "required": ["old_string", "new_string"],
                            "additionalProperties": false
                        }
                    }
                },
                "required": ["path", "edits"],
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
        let args = parse::<MultiEditArgs>(&call)?;
        if args.edits.is_empty() {
            anyhow::bail!("edits are required");
        }
        for edit in &args.edits {
            require_nonempty(&edit.old_string, "old_string")?;
        }
        let hunk_edits = args.edits.clone();
        let edits = args
            .edits
            .into_iter()
            .map(|edit| TextEdit {
                old_string: edit.old_string,
                new_string: edit.new_string,
            })
            .collect::<Vec<_>>();
        let backend = backend_from_context_or_fallback(&ctx, &self.workspace, &self.backend)?;
        let outcome = match backend.multi_edit_text(&args.path, edits).await? {
            Ok(outcome) => outcome,
            Err(index) => {
                return Ok(result(
                    call,
                    format!("edit {index} old_string does not match file"),
                    json!({ "error": { "kind": "old_string_not_found", "edit": index } }),
                    true,
                ));
            }
        };
        let hunks = hunk_edits
            .iter()
            .enumerate()
            .map(|(index, edit)| {
                hunk_output::record(
                    &ctx,
                    &call,
                    index,
                    outcome.path.clone(),
                    edit.old_string.lines().map(str::to_string).collect(),
                    edit.new_string.lines().map(str::to_string).collect(),
                )
            })
            .collect::<Vec<_>>();
        Ok(result(
            call,
            format!(
                "edited {} ({} replacements)",
                outcome.path, outcome.replacements
            ),
            json!({ "path": outcome.path, "replacements": outcome.replacements, "hunks": hunks }),
            false,
        ))
    }
}

#[derive(Deserialize)]
struct WriteFileArgs {
    path: String,
    content: String,
}

#[derive(Deserialize)]
struct EditArgs {
    path: String,
    old_string: String,
    new_string: String,
}

#[derive(Deserialize)]
struct MultiEditArgs {
    path: String,
    edits: Vec<TextEditArgs>,
}

#[derive(Clone, Deserialize)]
struct TextEditArgs {
    old_string: String,
    new_string: String,
}
