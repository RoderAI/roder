mod artifacts;
mod backend;
mod command_shell;
mod design;
mod discovery;
mod edit;
mod exec;
mod exec_output;
mod files;
mod goals;
mod hunk_output;
mod media;
mod paging;
mod patch;
#[cfg(test)]
mod remote_test_support;
mod response_format;
mod search;
mod shell;
mod view_image;
mod workflow;
mod workspace;

use std::path::PathBuf;
use std::sync::Arc;

use roder_api::extension::ToolProviderId;
use serde_json::json;

pub use roder_api::tools::*;

pub use workspace::ToolPathScope;
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
    command_shell: String,
}

impl BuiltinCodingToolsContributor {
    pub fn new(workspace: impl Into<PathBuf>) -> anyhow::Result<Self> {
        Self::new_with_path_scope(workspace, ToolPathScope::default())
    }

    pub fn new_with_path_scope(
        workspace: impl Into<PathBuf>,
        path_scope: ToolPathScope,
    ) -> anyhow::Result<Self> {
        Self::new_with_path_scope_and_shell(
            workspace,
            path_scope,
            roder_api::command_shell::default_command_shell(),
        )
    }

    pub fn new_with_path_scope_and_shell(
        workspace: impl Into<PathBuf>,
        path_scope: ToolPathScope,
        command_shell: impl Into<String>,
    ) -> anyhow::Result<Self> {
        let workspace = Workspace::new_with_scope(workspace.into(), path_scope)?;
        let backend = Arc::new(LocalWorkspaceBackend::new(workspace.clone()));
        Ok(Self {
            workspace,
            backend,
            command_shell: command_shell.into(),
        })
    }

    #[cfg(test)]
    fn new_with_backend(
        workspace: impl Into<PathBuf>,
        backend: WorkspaceBackendHandle,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            workspace: Workspace::new(workspace.into())?,
            backend,
            command_shell: roder_api::command_shell::default_command_shell(),
        })
    }
}

impl ToolContributor for BuiltinCodingToolsContributor {
    fn id(&self) -> ToolProviderId {
        "builtin-coding-tools".to_string()
    }

    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        files::register(registry, self.workspace.clone(), self.backend.clone())?;
        search::register(registry, self.workspace.clone(), self.backend.clone())?;
        shell::register(
            registry,
            self.workspace.clone(),
            self.command_shell.clone(),
            Some(self.backend.clone()),
        )?;
        exec::register(
            registry,
            self.workspace.clone(),
            self.command_shell.clone(),
            Some(self.backend.clone()),
        )?;
        registry.register(Arc::new(patch::ApplyPatchTool {
            workspace: self.workspace.clone(),
            backend: self.backend.clone(),
        }))?;
        edit::register(registry, self.workspace.clone(), self.backend.clone())?;
        design::register(registry, self.workspace.clone())?;
        workflow::register(registry)?;
        media::register(registry)?;
        view_image::register(registry, self.workspace.clone(), self.backend.clone())?;
        artifacts::register(registry)?;
        discovery::register(registry)
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

pub fn builtin_coding_tools_contributor_with_path_scope(
    workspace: impl Into<PathBuf>,
    path_scope: ToolPathScope,
) -> anyhow::Result<Arc<dyn ToolContributor>> {
    Ok(Arc::new(
        BuiltinCodingToolsContributor::new_with_path_scope(workspace, path_scope)?,
    ))
}

pub fn builtin_coding_tools_contributor_with_path_scope_and_shell(
    workspace: impl Into<PathBuf>,
    path_scope: ToolPathScope,
    command_shell: impl Into<String>,
) -> anyhow::Result<Arc<dyn ToolContributor>> {
    Ok(Arc::new(
        BuiltinCodingToolsContributor::new_with_path_scope_and_shell(
            workspace,
            path_scope,
            command_shell,
        )?,
    ))
}

#[cfg(test)]
mod tool_search_catalog_tests {
    use super::*;
    use roder_api::inference::ToolSearchConfig;
    use roder_api::tool_search_catalog::ToolSearchCatalog;
    use roder_api::tools::ToolRegistry;

    #[test]
    fn tool_search_catalog_over_builtin_tools_is_stable_and_provider_safe() {
        let workspace = std::env::temp_dir();
        let contributor = builtin_coding_tools_contributor(&workspace).expect("contributor");
        let mut registry = ToolRegistry::default();
        contributor.contribute(&mut registry).expect("register");
        let specs = registry.specs();
        assert!(!specs.is_empty());

        let config = ToolSearchConfig::default();
        let first = ToolSearchCatalog::build(&specs, &config);
        let second = ToolSearchCatalog::build(&specs, &config);
        assert_eq!(first, second, "catalog is stable across runs");

        // Every catalog item resolves back to a registered executor.
        for item in &first.items {
            assert!(
                registry.get(&item.name).is_some(),
                "{} must map to a canonical executor",
                item.name
            );
        }

        // Nothing credential-like or process-local leaves through payloads.
        let serialized = serde_json::to_string(&first).unwrap();
        for needle in ["sk-", "Bearer ", "api_key\":\"", "/Users/", "x-roder-"] {
            assert!(!serialized.contains(needle), "leaked {needle:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use crate::backend::RunnerWorkspaceBackend;
    #[cfg(unix)]
    use roder_api::remote_runner::{RemoteRunnerProvider, RunnerDestination, RunnerManifest};
    #[cfg(unix)]
    use roder_ext_runner_unix_local::UnixLocalRunnerProvider;
    #[cfg(not(windows))]
    use std::sync::{Mutex, OnceLock};

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
            &root,
            "write_file",
            json!({ "path": "src/main.rs", "content": "alpha\nneedle\nomega\n" }),
        )
        .await;
        assert_eq!(write.text, "wrote src/main.rs");

        let grep = run_tool(&registry, &root, "grep", json!({ "query": "needle" })).await;
        assert!(grep.text.contains("src/main.rs:2:needle"));

        let glob = run_tool(&registry, &root, "glob", json!({ "pattern": "src/*.rs" })).await;
        assert_eq!(glob.text, "src/main.rs");

        let edit = run_tool(
            &registry,
            &root,
            "edit",
            json!({ "path": "src/main.rs", "old_string": "needle", "new_string": "NEEDLE" }),
        )
        .await;
        assert_eq!(edit.text, "edited src/main.rs");

        let multi_edit = run_tool(
            &registry,
            &root,
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
            &root,
            "apply_patch",
            json!({
                "patch": "*** Begin Patch\n*** Update File: src/main.rs\n@@\n-ALPHA\n+patched\n*** End Patch\n"
            }),
        )
        .await;
        assert_eq!(patch.text, "Success. Updated src/main.rs");

        let read = run_tool(
            &registry,
            &root,
            "read_file",
            json!({ "path": "src/main.rs", "start_line": 2, "limit": 1 }),
        )
        .await;
        assert!(read.text.contains("2: NEEDLE"));
        assert_eq!(read.data["next_start_line"], 3);

        let relative_read = run_tool(
            &registry,
            &root,
            "read_file",
            json!({ "path": "./src/../src/main.rs", "start_line": 1, "limit": 1 }),
        )
        .await;
        assert!(relative_read.text.contains("1: patched"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn builtin_apply_patch_respects_path_scope() {
        let root = test_workspace("apply-patch-root");
        let outside = test_workspace("apply-patch-outside");
        let target = outside.join("patched.txt");

        let patch = format!(
            "*** Begin Patch\n*** Add File: {}\n+yes\n*** End Patch\n",
            target.display()
        );

        let mut global_registry = ToolRegistry::default();
        BuiltinCodingToolsContributor::new(root.clone())
            .unwrap()
            .contribute(&mut global_registry)
            .unwrap();
        let result = run_tool(
            &global_registry,
            &root,
            "apply_patch",
            json!({ "patch": patch }),
        )
        .await;
        assert!(result.text.contains("Success. Added"));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "yes\n");

        let mut workspace_registry = ToolRegistry::default();
        BuiltinCodingToolsContributor::new_with_path_scope(root.clone(), ToolPathScope::Workspace)
            .unwrap()
            .contribute(&mut workspace_registry)
            .unwrap();
        let err = registry_apply_patch_error(&workspace_registry, &root, &target).await;
        assert!(err.contains("outside workspace"));

        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(outside);
    }

    #[test]
    fn schema_snapshots_cover_model_facing_builtin_coding_tools() {
        let root = test_workspace("schema-snapshots");
        let mut registry = ToolRegistry::default();
        BuiltinCodingToolsContributor::new(root.clone())
            .unwrap()
            .contribute(&mut registry)
            .unwrap();
        let schemas = registry
            .specs()
            .into_iter()
            .filter(|spec| {
                matches!(
                    spec.name.as_str(),
                    "read_file"
                        | "grep"
                        | "glob"
                        | "edit"
                        | "multi_edit"
                        | "apply_patch"
                        | "shell"
                        | "exec_command"
                )
            })
            .map(|spec| (spec.name, serde_json::to_string(&spec.parameters).unwrap()))
            .collect::<std::collections::BTreeMap<_, _>>();

        assert!(
            schemas["read_file"]
                .starts_with(r#"{"type":"object","required":["path"],"properties":"#)
        );
        assert!(
            schemas["grep"].starts_with(r#"{"type":"object","required":["query"],"properties":"#)
        );
        assert!(
            schemas["glob"].starts_with(r#"{"type":"object","required":["pattern"],"properties":"#)
        );
        assert!(schemas["edit"].starts_with(
            r#"{"type":"object","required":["path","old_string","new_string"],"properties":"#
        ));
        assert!(
            schemas["apply_patch"]
                .starts_with(r#"{"type":"object","required":["patch"],"properties":"#)
        );
        assert!(
            schemas["shell"]
                .starts_with(r#"{"type":"object","required":["command"],"properties":"#)
        );
        assert!(
            schemas["exec_command"]
                .starts_with(r#"{"type":"object","required":["cmd"],"properties":"#)
        );
        assert_eq!(
            schemas["multi_edit"],
            r#"{"type":"object","required":["path","edits"],"properties":{"edits":{"type":"array","items":{"type":"object","required":["old_string","new_string"],"properties":{"new_string":{"type":"string"},"old_string":{"type":"string"}},"additionalProperties":false}},"path":{"type":"string"}},"additionalProperties":false}"#
        );
        assert!(
            schemas
                .values()
                .all(|schema| schema.contains(r#""additionalProperties":false"#))
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
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
            &root,
            "list_files",
            json!({ "path": "src", "limit": 1 }),
        )
        .await;
        assert_eq!(files.text.lines().next(), Some("a.rs"));
        assert_eq!(files.data["next_offset"], 1);

        let grep = run_tool(
            &registry,
            &root,
            "grep",
            json!({ "query": "needle", "path": "src", "limit": 1 }),
        )
        .await;
        assert!(grep.text.contains("src/a.rs:1:needle a"));
        assert_eq!(grep.data["next_offset"], 1);

        let glob = run_tool(
            &registry,
            &root,
            "glob",
            json!({ "pattern": "src/*.rs", "offset": 1, "limit": 1 }),
        )
        .await;
        assert_eq!(glob.text.lines().next(), Some("src/b.rs"));
        assert_eq!(glob.data["offset"], 1);

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn builtin_grep_supports_regex_modes_and_metadata() {
        let root = test_workspace("grep-search-index");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.rs"), "ErrorKind\nterror\nerror\n").unwrap();
        std::fs::write(root.join("src/b.rs"), "nothing\n").unwrap();
        let mut registry = ToolRegistry::default();
        BuiltinCodingToolsContributor::new(root.clone())
            .unwrap()
            .contribute(&mut registry)
            .unwrap();

        let grep = run_tool(
            &registry,
            &root,
            "grep",
            json!({
                "query": "error",
                "path": "src",
                "regex": true,
                "case_sensitive": false,
                "word_boundary": true,
                "mode": "auto"
            }),
        )
        .await;

        assert!(grep.text.contains("src/a.rs:3:error"));
        assert!(!grep.text.contains("terror"));
        assert_eq!(grep.data["engine"], "indexed");
        assert_eq!(grep.data["candidate_files"], 1);
        assert_eq!(grep.data["verified_files"], 1);
        assert_eq!(grep.data["stale"], false);
        assert_eq!(grep.data["index_version"], "roder-search-v2");
        assert_eq!(grep.data["retrieval_mode"], "exact_text");

        let scan = run_tool(
            &registry,
            &root,
            "grep",
            json!({ "query": "nothing", "path": "src", "mode": "scan" }),
        )
        .await;
        assert!(scan.text.contains("src/b.rs:1:nothing"));
        assert_eq!(scan.data["engine"], "scan");

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn search_tools_advertise_retrieval_metadata() {
        let root = test_workspace("retrieval-metadata");
        let mut registry = ToolRegistry::default();
        BuiltinCodingToolsContributor::new(root.clone())
            .unwrap()
            .contribute(&mut registry)
            .unwrap();

        let grep = registry.get("grep").unwrap().spec();
        assert_eq!(grep.parameters["x-roder"]["retrievalMode"], "exact_text");
        let glob = registry.get("glob").unwrap().spec();
        assert_eq!(glob.parameters["x-roder"]["retrievalMode"], "file_name");

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn builtin_grep_refreshes_index_after_workspace_writes() {
        let root = test_workspace("grep-search-refresh");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.rs"), "needle a\n").unwrap();
        let mut registry = ToolRegistry::default();
        BuiltinCodingToolsContributor::new(root.clone())
            .unwrap()
            .contribute(&mut registry)
            .unwrap();

        let first = run_tool(
            &registry,
            &root,
            "grep",
            json!({ "query": "needle", "mode": "indexed" }),
        )
        .await;
        assert_eq!(first.data["engine"], "indexed");
        assert!(first.text.contains("src/a.rs:1:needle a"));

        let write = run_tool(
            &registry,
            &root,
            "write_file",
            json!({ "path": "src/b.rs", "content": "needle b\n" }),
        )
        .await;
        assert_eq!(write.text, "wrote src/b.rs");

        let second = run_tool(
            &registry,
            &root,
            "grep",
            json!({ "query": "needle", "mode": "indexed" }),
        )
        .await;
        assert!(second.text.contains("src/a.rs:1:needle a"));
        assert!(second.text.contains("src/b.rs:1:needle b"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn artifacts_tools_read_grep_and_tail_current_thread_store() {
        let mut registry = ToolRegistry::default();
        artifacts::register(&mut registry).unwrap();
        let store = Arc::new(FakeArtifactStore);
        let ctx = context_without_handles().with_context_artifacts(store);

        let read = registry
            .get("read_artifact")
            .unwrap()
            .execute(
                ctx.clone(),
                call(
                    "read_artifact",
                    json!({ "artifact_id": "artifact-1", "start_line": 2, "limit": 1 }),
                ),
            )
            .await
            .unwrap();
        assert_eq!(read.text, "    2: needle");
        assert_eq!(read.data["nextStartLine"], 3);

        let grep = registry
            .get("grep_artifact")
            .unwrap()
            .execute(
                ctx.clone(),
                call(
                    "grep_artifact",
                    json!({ "artifact_id": "artifact-1", "query": "needle" }),
                ),
            )
            .await
            .unwrap();
        assert_eq!(grep.text, "2: needle");

        let tail = registry
            .get("tail_artifact")
            .unwrap()
            .execute(
                ctx,
                call(
                    "tail_artifact",
                    json!({ "artifact_id": "artifact-1", "lines": 1 }),
                ),
            )
            .await
            .unwrap();
        assert_eq!(tail.text, "    3: omega");
    }

    #[tokio::test]
    async fn builtin_coding_tools_allow_paths_outside_workspace_by_default() {
        let root = test_workspace("path-global-root");
        let outside = test_workspace("path-global-outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let outside_file = outside.join("outside.txt");
        let mut registry = ToolRegistry::default();
        BuiltinCodingToolsContributor::new(root.clone())
            .unwrap()
            .contribute(&mut registry)
            .unwrap();

        let write = run_tool(
            &registry,
            &root,
            "write_file",
            json!({ "path": outside_file.display().to_string(), "content": "yes" }),
        )
        .await;
        assert_eq!(
            write.text,
            format!("wrote {}", outside_file.display()).replace('\\', "/")
        );

        let list = run_tool(
            &registry,
            &root,
            "list_files",
            json!({ "path": outside.display().to_string() }),
        )
        .await;
        assert_eq!(list.text, "outside.txt");

        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(outside);
    }

    #[cfg(not(windows))]
    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn builtin_coding_tools_list_files_expands_home_directory() {
        let _guard = env_lock().lock().unwrap();
        let previous_home = std::env::var_os("HOME");
        let previous_userprofile = std::env::var_os("USERPROFILE");
        let root = test_workspace("home-root");
        let home = test_workspace("home-dir");
        std::fs::write(home.join("home-file.txt"), "yes").unwrap();
        let mut registry = ToolRegistry::default();
        BuiltinCodingToolsContributor::new(root.clone())
            .unwrap()
            .contribute(&mut registry)
            .unwrap();

        // SAFETY: this test holds a process-wide mutex while mutating HOME.
        unsafe {
            std::env::set_var("HOME", &home);
            std::env::set_var("USERPROFILE", &home);
        }
        let list = run_tool(&registry, &root, "list_files", json!({ "path": "~/" })).await;
        restore_home(previous_home);
        restore_userprofile(previous_userprofile);

        assert_eq!(list.text, "home-file.txt");

        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(home);
    }

    #[tokio::test]
    async fn builtin_coding_tools_can_restrict_paths_to_workspace() {
        let root = test_workspace("path-safety");
        std::fs::create_dir_all(&root).unwrap();
        let mut registry = ToolRegistry::default();
        BuiltinCodingToolsContributor::new_with_path_scope(root.clone(), ToolPathScope::Workspace)
            .unwrap()
            .contribute(&mut registry)
            .unwrap();

        let err = registry
            .get("write_file")
            .unwrap()
            .execute(
                context_with_workspace(&root),
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
        workspace: &std::path::Path,
        name: &str,
        arguments: serde_json::Value,
    ) -> ToolResult {
        registry
            .get(name)
            .unwrap_or_else(|| panic!("missing tool {name}"))
            .execute(context_with_workspace(workspace), call(name, arguments))
            .await
            .unwrap()
    }

    async fn registry_apply_patch_error(
        registry: &ToolRegistry,
        workspace: &std::path::Path,
        target: &std::path::Path,
    ) -> String {
        let result = registry
            .get("apply_patch")
            .unwrap()
            .execute(
                context_with_workspace(workspace),
                call(
                    "apply_patch",
                    json!({
                        "patch": format!(
                            "*** Begin Patch\n*** Add File: {}\n+no\n*** End Patch\n",
                            target.display()
                        )
                    }),
                ),
            )
            .await
            .unwrap();
        assert!(result.is_error);
        result.text
    }

    fn context() -> ToolExecutionContext {
        context_with_workspace(std::path::Path::new("."))
    }

    fn context_with_workspace(workspace: &std::path::Path) -> ToolExecutionContext {
        context_without_handles()
            .with_workspace_handle(Arc::new(LocalWorkspaceHandle::new(workspace)))
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

    #[cfg(not(windows))]
    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[cfg(not(windows))]
    fn restore_home(previous_home: Option<std::ffi::OsString>) {
        // SAFETY: callers hold env_lock while restoring HOME.
        unsafe {
            if let Some(previous_home) = previous_home {
                std::env::set_var("HOME", previous_home);
            } else {
                std::env::remove_var("HOME");
            }
        }
    }

    #[cfg(not(windows))]
    fn restore_userprofile(previous_userprofile: Option<std::ffi::OsString>) {
        // SAFETY: callers hold env_lock while restoring USERPROFILE.
        unsafe {
            if let Some(previous_userprofile) = previous_userprofile {
                std::env::set_var("USERPROFILE", previous_userprofile);
            } else {
                std::env::remove_var("USERPROFILE");
            }
        }
    }

    struct FakeArtifactStore;

    impl roder_api::artifacts::ContextArtifactAccess for FakeArtifactStore {
        fn create_artifact(
            &self,
            request: roder_api::artifacts::CreateArtifactRequest<'_>,
        ) -> anyhow::Result<roder_api::artifacts::ContextArtifact> {
            Ok(roder_api::artifacts::ContextArtifact {
                id: "artifact-1".to_string(),
                kind: request.kind,
                thread_id: request.thread_id.to_string(),
                turn_id: request.turn_id.to_string(),
                byte_count: request.bytes.len() as u64,
                line_count: String::from_utf8_lossy(request.bytes).lines().count() as u64,
                source_tool_id: request.source_tool_id.map(ToString::to_string),
                label: request.label.map(ToString::to_string),
                store_path: "/private/artifact-1.txt".to_string(),
                retention_expires_at: None,
                created_at: time::OffsetDateTime::UNIX_EPOCH,
                roder_owned: true,
            })
        }

        fn append_artifact(
            &self,
            thread_id: &roder_api::events::ThreadId,
            _artifact_id: &roder_api::artifacts::ContextArtifactId,
            _bytes: &[u8],
        ) -> anyhow::Result<roder_api::artifacts::ContextArtifact> {
            Ok(artifact(thread_id))
        }

        fn list_artifacts(
            &self,
            thread_id: &roder_api::events::ThreadId,
        ) -> anyhow::Result<Vec<roder_api::artifacts::ContextArtifact>> {
            Ok(vec![artifact(thread_id)])
        }

        fn read_artifact(
            &self,
            thread_id: &roder_api::events::ThreadId,
            _artifact_id: &roder_api::artifacts::ContextArtifactId,
            start_line: usize,
            _limit: usize,
        ) -> anyhow::Result<roder_api::artifacts::ArtifactReadPage> {
            let lines = ["    1: alpha", "    2: needle", "    3: omega"];
            Ok(roder_api::artifacts::ArtifactReadPage {
                artifact: artifact(thread_id).descriptor(),
                text: lines[start_line - 1].to_string(),
                start_line,
                limit: 1,
                shown: 1,
                total_lines: 3,
                next_start_line: Some(start_line + 1).filter(|line| *line <= 3),
                truncated: start_line < 3,
            })
        }

        fn grep_artifact(
            &self,
            thread_id: &roder_api::events::ThreadId,
            _artifact_id: &roder_api::artifacts::ContextArtifactId,
            query: &str,
            _offset: usize,
            _limit: usize,
        ) -> anyhow::Result<roder_api::artifacts::ArtifactGrepPage> {
            Ok(roder_api::artifacts::ArtifactGrepPage {
                artifact: artifact(thread_id).descriptor(),
                query: query.to_string(),
                text: "2: needle".to_string(),
                offset: 0,
                limit: 200,
                shown: 1,
                total_matches: 1,
                next_offset: None,
                truncated: false,
            })
        }

        fn tail_artifact(
            &self,
            thread_id: &roder_api::events::ThreadId,
            _artifact_id: &roder_api::artifacts::ContextArtifactId,
            lines: usize,
        ) -> anyhow::Result<roder_api::artifacts::ArtifactTailPage> {
            Ok(roder_api::artifacts::ArtifactTailPage {
                artifact: artifact(thread_id).descriptor(),
                text: "    3: omega".to_string(),
                start_line: 3,
                lines,
                shown: 1,
                total_lines: 3,
                truncated: true,
            })
        }

        fn delete_artifact(
            &self,
            _thread_id: &roder_api::events::ThreadId,
            _artifact_id: &roder_api::artifacts::ContextArtifactId,
        ) -> anyhow::Result<bool> {
            Ok(true)
        }
    }

    fn artifact(thread_id: &str) -> roder_api::artifacts::ContextArtifact {
        roder_api::artifacts::ContextArtifact {
            id: "artifact-1".to_string(),
            kind: roder_api::artifacts::ContextArtifactKind::ToolOutput,
            thread_id: thread_id.to_string(),
            turn_id: "turn-a".to_string(),
            byte_count: 18,
            line_count: 3,
            source_tool_id: Some("call-a".to_string()),
            label: Some("stdout".to_string()),
            store_path: "/private/artifact-1.txt".to_string(),
            retention_expires_at: None,
            created_at: time::OffsetDateTime::UNIX_EPOCH,
            roder_owned: true,
        }
    }

    async fn run_coding_tool_sequence(contributor: BuiltinCodingToolsContributor) -> Vec<String> {
        let workspace = contributor.workspace.root().to_path_buf();
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
            let result = run_tool(&registry, &workspace, name, args).await;
            assert!(!result.is_error, "{name}: {}", result.text);
            outputs.push(format!("{}\n{}", result.text, redact_volatile(result.data)));
        }
        outputs
    }

    /// Strip search-index metadata that legitimately varies between backends
    /// (the local backend can build an on-disk index; the runner backend falls
    /// back to scanning) or between runs (timings), so the comparison checks the
    /// tool results themselves rather than these implementation details.
    fn redact_volatile(mut data: serde_json::Value) -> serde_json::Value {
        if let Some(obj) = data.as_object_mut() {
            for key in ["engine", "elapsed_ms", "index_build_time_ms", "index_bytes"] {
                if obj.contains_key(key) {
                    obj.insert(key.to_string(), serde_json::Value::Null);
                }
            }
        }
        data
    }
}
