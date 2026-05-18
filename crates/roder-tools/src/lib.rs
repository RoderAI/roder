mod backend;
mod edit;
mod exec;
mod files;
mod hunk_output;
mod media;
mod paging;
mod patch;
mod search;
mod shell;
mod workflow;
mod workspace;

use std::path::PathBuf;
use std::sync::Arc;

use roder_api::extension::ToolProviderId;
use serde_json::json;

pub use roder_api::tools::*;

use workspace::Workspace;

use backend::{LocalWorkspaceBackend, WorkspaceBackendHandle};

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

#[derive(Clone)]
pub struct BuiltinCodingToolsContributor {
    workspace: Workspace,
    backend: WorkspaceBackendHandle,
}

impl BuiltinCodingToolsContributor {
    pub fn new(workspace: impl Into<PathBuf>) -> anyhow::Result<Self> {
        let workspace = Workspace::new(workspace.into())?;
        let backend = Arc::new(LocalWorkspaceBackend::new(workspace.clone()));
        Ok(Self { workspace, backend })
    }

    #[cfg(test)]
    fn new_with_backend(
        workspace: impl Into<PathBuf>,
        backend: WorkspaceBackendHandle,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            workspace: Workspace::new(workspace.into())?,
            backend,
        })
    }
}

impl ToolContributor for BuiltinCodingToolsContributor {
    fn id(&self) -> ToolProviderId {
        "builtin-coding-tools".to_string()
    }

    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        files::register(registry, self.backend.clone())?;
        search::register(registry, self.backend.clone())?;
        shell::register(registry, self.workspace.clone())?;
        exec::register(registry, self.workspace.clone())?;
        registry.register(Arc::new(patch::ApplyPatchTool {
            backend: self.backend.clone(),
        }))?;
        edit::register(registry, self.backend.clone())?;
        workflow::register(registry)?;
        media::register(registry)
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
    use crate::backend::RunnerWorkspaceBackend;
    use roder_api::remote_runner::{RemoteRunnerProvider, RunnerDestination, RunnerManifest};
    use roder_ext_runner_unix_local::UnixLocalRunnerProvider;

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
                context(),
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
        assert_eq!(read.data["next_start_line"], 3);

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
    async fn builtin_coding_tools_match_direct_local_and_unix_local_runner_backends() {
        let direct_root = test_workspace("coding-tools-direct");
        let runner_root = test_workspace("coding-tools-runner");
        let direct_outputs = run_coding_tool_sequence(
            BuiltinCodingToolsContributor::new(direct_root.clone()).unwrap(),
        )
        .await;

        let guard = Workspace::new(runner_root.clone()).unwrap();
        let provider = UnixLocalRunnerProvider::default();
        let session = provider
            .create_session(RunnerDestination {
                id: "unix-local".to_string(),
                provider_id: "unix-local".to_string(),
                config: serde_json::json!({ "root": runner_root.display().to_string() }),
                default_manifest: RunnerManifest::default(),
            })
            .await
            .unwrap();
        let runner_backend = Arc::new(RunnerWorkspaceBackend::new(guard, session));
        let runner_outputs = run_coding_tool_sequence(
            BuiltinCodingToolsContributor::new_with_backend(runner_root.clone(), runner_backend)
                .unwrap(),
        )
        .await;

        assert_eq!(runner_outputs, direct_outputs);

        let _ = std::fs::remove_dir_all(direct_root);
        let _ = std::fs::remove_dir_all(runner_root);
    }

    #[tokio::test]
    async fn builtin_coding_tools_paginate_line_outputs() {
        let root = test_workspace("paging");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.rs"), "needle a\n").unwrap();
        std::fs::write(root.join("src/b.rs"), "needle b\n").unwrap();
        let mut registry = ToolRegistry::default();
        BuiltinCodingToolsContributor::new(root.clone())
            .unwrap()
            .contribute(&mut registry)
            .unwrap();

        let files = run_tool(
            &registry,
            "list_files",
            json!({ "path": "src", "limit": 1 }),
        )
        .await;
        assert_eq!(files.text.lines().next(), Some("a.rs"));
        assert_eq!(files.data["next_offset"], 1);

        let grep = run_tool(
            &registry,
            "grep",
            json!({ "query": "needle", "path": "src", "limit": 1 }),
        )
        .await;
        assert!(grep.text.contains("src/a.rs:1:needle a"));
        assert_eq!(grep.data["next_offset"], 1);

        let glob = run_tool(
            &registry,
            "glob",
            json!({ "pattern": "src/*.rs", "offset": 1, "limit": 1 }),
        )
        .await;
        assert_eq!(glob.text.lines().next(), Some("src/b.rs"));
        assert_eq!(glob.data["offset"], 1);

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

    #[tokio::test]
    async fn workspace_tools_require_scoped_workspace_handle() {
        let root = test_workspace("missing-workspace-handle");
        std::fs::write(root.join("note.txt"), "secret\n").unwrap();
        let mut registry = ToolRegistry::default();
        BuiltinCodingToolsContributor::new(root.clone())
            .unwrap()
            .contribute(&mut registry)
            .unwrap();

        let err = registry
            .get("read_file")
            .unwrap()
            .execute(
                context_without_handles(),
                call("read_file", json!({ "path": "note.txt" })),
            )
            .await
            .unwrap_err()
            .to_string();

        assert!(err.contains("workspace handle is not available"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn shell_tool_requires_scoped_process_runner() {
        let root = test_workspace("missing-process-handle");
        let mut registry = ToolRegistry::default();
        BuiltinCodingToolsContributor::new(root.clone())
            .unwrap()
            .contribute(&mut registry)
            .unwrap();

        let err = registry
            .get("shell")
            .unwrap()
            .execute(
                context_without_handles(),
                call("shell", json!({ "command": "printf hi" })),
            )
            .await
            .unwrap_err()
            .to_string();

        assert!(err.contains("process runner is not available"));

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
        context_without_handles()
            .with_workspace_handle(Arc::new(LocalWorkspaceHandle::new(".")))
            .with_process_runner(Arc::new(LocalProcessRunnerHandle))
    }

    fn context_without_handles() -> ToolExecutionContext {
        ToolExecutionContext::new(
            "thread-a",
            "turn-a",
            roder_api::policy_mode::PolicyMode::Default,
        )
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

    async fn run_coding_tool_sequence(contributor: BuiltinCodingToolsContributor) -> Vec<String> {
        let mut registry = ToolRegistry::default();
        contributor.contribute(&mut registry).unwrap();
        let calls = [
            (
                "write_file",
                json!({ "path": "src/main.rs", "content": "alpha\nneedle\nomega\n" }),
            ),
            ("list_files", json!({ "path": "src" })),
            ("grep", json!({ "query": "needle" })),
            ("glob", json!({ "pattern": "src/*.rs" })),
            (
                "edit",
                json!({ "path": "src/main.rs", "old_string": "needle", "new_string": "NEEDLE" }),
            ),
            (
                "multi_edit",
                json!({
                    "path": "src/main.rs",
                    "edits": [
                        { "old_string": "alpha", "new_string": "ALPHA" },
                        { "old_string": "omega", "new_string": "OMEGA" }
                    ]
                }),
            ),
            (
                "apply_patch",
                json!({
                    "patch": "*** Begin Patch\n*** Update File: src/main.rs\n@@\n-ALPHA\n+patched\n*** End Patch\n"
                }),
            ),
            (
                "read_file",
                json!({ "path": "src/main.rs", "start_line": 1, "limit": 3 }),
            ),
        ];
        let mut outputs = Vec::new();
        for (name, args) in calls {
            let result = run_tool(&registry, name, args).await;
            assert!(!result.is_error, "{name}: {}", result.text);
            outputs.push(format!("{}\n{}", result.text, result.data));
        }
        outputs
    }
}
