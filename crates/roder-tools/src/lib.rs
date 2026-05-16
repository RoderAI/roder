mod edit;
mod files;
mod patch;
mod search;
mod workspace;

use std::path::PathBuf;
use std::sync::Arc;

use roder_api::extension::ToolProviderId;
use serde_json::json;

pub use roder_api::tools::*;

use workspace::Workspace;

#[derive(Debug, Default)]
pub struct EchoToolContributor;

impl ToolContributor for EchoToolContributor {
    fn id(&self) -> ToolProviderId {
        "builtin-echo".to_string()
    }

    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        registry.register(Arc::new(EchoTool))
    }
}

#[derive(Debug)]
pub struct EchoTool;

#[async_trait::async_trait]
impl ToolExecutor for EchoTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "echo".to_string(),
            description: "Returns the provided text argument unchanged.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "Text to return."
                    }
                },
                "required": ["text"]
            }),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let text = call
            .arguments
            .get("text")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(&call.raw_arguments)
            .to_string();
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text: text.clone(),
            data: json!({ "text": text }),
            is_error: false,
        })
    }
}

#[derive(Debug, Clone)]
pub struct BuiltinCodingToolsContributor {
    workspace: Workspace,
}

impl BuiltinCodingToolsContributor {
    pub fn new(workspace: impl Into<PathBuf>) -> anyhow::Result<Self> {
        Ok(Self {
            workspace: Workspace::new(workspace.into())?,
        })
    }
}

impl ToolContributor for BuiltinCodingToolsContributor {
    fn id(&self) -> ToolProviderId {
        "builtin-coding-tools".to_string()
    }

    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        files::register(registry, self.workspace.clone())?;
        search::register(registry, self.workspace.clone())?;
        registry.register(Arc::new(patch::ApplyPatchTool {
            workspace: self.workspace.clone(),
        }))?;
        edit::register(registry, self.workspace.clone())
    }
}

pub fn echo_tool_contributor() -> Arc<dyn ToolContributor> {
    Arc::new(EchoToolContributor)
}

pub fn builtin_coding_tools_contributor(
    workspace: impl Into<PathBuf>,
) -> anyhow::Result<Arc<dyn ToolContributor>> {
    Ok(Arc::new(BuiltinCodingToolsContributor::new(workspace)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn echo_contributor_registers_echo_spec() {
        let mut registry = ToolRegistry::default();
        EchoToolContributor.contribute(&mut registry).unwrap();

        let specs = registry.specs();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "echo");
        assert!(registry.get("echo").is_some());
    }

    #[tokio::test]
    async fn echo_tool_returns_text_argument() {
        let tool = EchoTool;
        let result = tool
            .execute(
                ToolExecutionContext {
                    thread_id: "thread-a".to_string(),
                    turn_id: "turn-a".to_string(),
                    effective_mode: roder_api::policy_mode::PolicyMode::Default,
                },
                ToolCall {
                    id: "call-a".to_string(),
                    name: "echo".to_string(),
                    arguments: json!({ "text": "hello harness" }),
                    raw_arguments: "{}".to_string(),
                    thread_id: "thread-a".to_string(),
                    turn_id: "turn-a".to_string(),
                },
            )
            .await
            .unwrap();

        assert_eq!(result.text, "hello harness");
        assert_eq!(result.data, json!({ "text": "hello harness" }));
        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn builtin_coding_tools_read_search_and_edit_workspace_files() {
        let root = test_workspace("coding-tools");
        std::fs::create_dir_all(root.join("src")).unwrap();
        let mut registry = ToolRegistry::default();
        BuiltinCodingToolsContributor::new(root.clone())
            .unwrap()
            .contribute(&mut registry)
            .unwrap();

        let write = run_tool(
            &registry,
            "write_file",
            json!({ "path": "src/main.rs", "content": "alpha\nneedle\nomega\n" }),
        )
        .await;
        assert_eq!(write.text, "wrote src/main.rs");

        let grep = run_tool(&registry, "grep", json!({ "query": "needle" })).await;
        assert!(grep.text.contains("src/main.rs:2:needle"));

        let glob = run_tool(&registry, "glob", json!({ "pattern": "src/*.rs" })).await;
        assert_eq!(glob.text, "src/main.rs");

        let edit = run_tool(
            &registry,
            "edit",
            json!({ "path": "src/main.rs", "old_string": "needle", "new_string": "NEEDLE" }),
        )
        .await;
        assert_eq!(edit.text, "edited src/main.rs");

        let multi_edit = run_tool(
            &registry,
            "multi_edit",
            json!({
                "path": "src/main.rs",
                "edits": [
                    { "old_string": "alpha", "new_string": "ALPHA" },
                    { "old_string": "omega", "new_string": "OMEGA" }
                ]
            }),
        )
        .await;
        assert_eq!(multi_edit.text, "edited src/main.rs (2 replacements)");

        let patch = run_tool(
            &registry,
            "apply_patch",
            json!({
                "patch": "*** Begin Patch\n*** Update File: src/main.rs\n@@\n-ALPHA\n+patched\n*** End Patch\n"
            }),
        )
        .await;
        assert_eq!(patch.text, "Success. Updated src/main.rs");

        let read = run_tool(
            &registry,
            "read_file",
            json!({ "path": "src/main.rs", "start_line": 2, "limit": 1 }),
        )
        .await;
        assert!(read.text.contains("2: NEEDLE"));

        let relative_read = run_tool(
            &registry,
            "read_file",
            json!({ "path": "./src/../src/main.rs", "start_line": 1, "limit": 1 }),
        )
        .await;
        assert!(relative_read.text.contains("1: patched"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn builtin_coding_tools_reject_paths_outside_workspace() {
        let root = test_workspace("path-safety");
        std::fs::create_dir_all(&root).unwrap();
        let mut registry = ToolRegistry::default();
        BuiltinCodingToolsContributor::new(root.clone())
            .unwrap()
            .contribute(&mut registry)
            .unwrap();

        let err = registry
            .get("write_file")
            .unwrap()
            .execute(
                context(),
                call(
                    "write_file",
                    json!({ "path": "../outside.txt", "content": "nope" }),
                ),
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("outside workspace"));

        let _ = std::fs::remove_dir_all(root);
    }

    async fn run_tool(
        registry: &ToolRegistry,
        name: &str,
        arguments: serde_json::Value,
    ) -> ToolResult {
        registry
            .get(name)
            .unwrap_or_else(|| panic!("missing tool {name}"))
            .execute(context(), call(name, arguments))
            .await
            .unwrap()
    }

    fn context() -> ToolExecutionContext {
        ToolExecutionContext {
            thread_id: "thread-a".to_string(),
            turn_id: "turn-a".to_string(),
            effective_mode: roder_api::policy_mode::PolicyMode::Default,
        }
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
