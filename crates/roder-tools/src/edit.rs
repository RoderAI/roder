use std::sync::Arc;

use roder_api::tools::{
    ToolCall, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult, ToolSpec,
};
use serde::Deserialize;
use serde_json::json;

use crate::files::{parse, require_nonempty, result};
use crate::workspace::Workspace;

pub(crate) fn register(registry: &mut ToolRegistry, workspace: Workspace) -> anyhow::Result<()> {
    registry.register(Arc::new(WriteFileTool {
        workspace: workspace.clone(),
    }))?;
    registry.register(Arc::new(EditTool {
        workspace: workspace.clone(),
    }))?;
    registry.register(Arc::new(MultiEditTool { workspace }))
}

#[derive(Debug)]
struct WriteFileTool {
    workspace: Workspace,
}

#[async_trait::async_trait]
impl ToolExecutor for WriteFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "write_file".to_string(),
            description: "Write a UTF-8 text file inside the workspace.".to_string(),
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
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args = parse::<WriteFileArgs>(&call)?;
        let path = self.workspace.resolve_for_write(&args.path)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, args.content)?;
        let rel = self.workspace.display(&path);
        Ok(result(
            call,
            format!("wrote {rel}"),
            json!({ "path": rel }),
            false,
        ))
    }
}

#[derive(Debug)]
struct EditTool {
    workspace: Workspace,
}

#[async_trait::async_trait]
impl ToolExecutor for EditTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "edit".to_string(),
            description: "Replace one exact text range inside a workspace file.".to_string(),
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
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args = parse::<EditArgs>(&call)?;
        require_nonempty(&args.old_string, "old_string")?;
        let path = self.workspace.resolve_existing(&args.path)?;
        let text = std::fs::read_to_string(&path)?;
        let Some(index) = text.find(&args.old_string) else {
            return Ok(result(
                call,
                "old_string does not match file".to_string(),
                json!({ "error": { "kind": "old_string_not_found" } }),
                true,
            ));
        };
        let mut updated = text;
        updated.replace_range(index..index + args.old_string.len(), &args.new_string);
        std::fs::write(&path, updated)?;
        let rel = self.workspace.display(&path);
        Ok(result(
            call,
            format!("edited {rel}"),
            json!({ "path": rel, "replacements": 1 }),
            false,
        ))
    }
}

#[derive(Debug)]
struct MultiEditTool {
    workspace: Workspace,
}

#[async_trait::async_trait]
impl ToolExecutor for MultiEditTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "multi_edit".to_string(),
            description: "Apply multiple exact text replacements to one workspace file."
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
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args = parse::<MultiEditArgs>(&call)?;
        if args.edits.is_empty() {
            anyhow::bail!("edits are required");
        }
        let path = self.workspace.resolve_existing(&args.path)?;
        let mut text = std::fs::read_to_string(&path)?;
        for (index, edit) in args.edits.iter().enumerate() {
            require_nonempty(&edit.old_string, "old_string")?;
            let Some(position) = text.find(&edit.old_string) else {
                return Ok(result(
                    call,
                    format!("edit {index} old_string does not match file"),
                    json!({ "error": { "kind": "old_string_not_found", "edit": index } }),
                    true,
                ));
            };
            text.replace_range(position..position + edit.old_string.len(), &edit.new_string);
        }
        std::fs::write(&path, text)?;
        let rel = self.workspace.display(&path);
        Ok(result(
            call,
            format!("edited {rel} ({} replacements)", args.edits.len()),
            json!({ "path": rel, "replacements": args.edits.len() }),
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
    edits: Vec<EditArgs>,
}
