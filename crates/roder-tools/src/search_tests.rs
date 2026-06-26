use super::*;
use crate::backend::LocalWorkspaceBackend;
use crate::workspace::ToolPathScope;
use roder_api::tools::{LocalWorkspaceHandle, ToolExecutionContext};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[test]
fn compiled_globs_support_star_question_mark_braces_and_classes() {
    let matches = |pattern: &str, text: &str| compile_glob(pattern).unwrap().is_match(text);
    assert!(matches("src/*.rs", "src/main.rs"));
    assert!(matches("src/??.rs", "src/io.rs"));
    assert!(!matches("src/*.rs", "README.md"));
    assert!(matches("**/*.{toml,json,md}", "docs/api.md"));
    assert!(matches("**/*.{toml,json,md}", "Cargo.toml"));
    assert!(!matches("**/*.{toml,json,md}", "src/main.rs"));
    assert!(matches("src/[ab].rs", "src/a.rs"));
    assert!(!matches("src/[ab].rs", "src/c.rs"));
}

#[test]
fn prepare_glob_pattern_resolves_workspace_absolute_prefixes() {
    let workspace = Workspace::remote(PathBuf::from("/workspace/project"), ToolPathScope::Global)
        .unwrap();
    let prepared = prepare_glob_pattern(&workspace, "/workspace/project/src/**/*.rs").unwrap();
    assert_eq!(
        prepared.matcher_pattern,
        "src/**/*.rs"
    );
    assert_eq!(prepared.search_root, PathBuf::from("/workspace/project"));

    let prepared = prepare_glob_pattern(&workspace, "src/*.rs").unwrap();
    assert_eq!(prepared.matcher_pattern, "src/*.rs");
    assert_eq!(prepared.search_root, PathBuf::from("/workspace/project"));
}

#[test]
fn prepare_glob_pattern_allows_external_absolute_prefixes_when_scope_is_global() {
    let workspace = Workspace::remote(PathBuf::from("/workspace/project"), ToolPathScope::Global)
        .unwrap();
    let prepared = prepare_glob_pattern(&workspace, "/elsewhere/**/*.rs").unwrap();

    assert_eq!(prepared.search_root, PathBuf::from("/elsewhere"));
    assert_eq!(prepared.matcher_pattern, "/elsewhere/**/*.rs");
}

#[test]
fn prepare_glob_pattern_rejects_external_absolute_prefixes_when_scope_is_workspace() {
    let workspace = Workspace::remote(PathBuf::from("/workspace/project"), ToolPathScope::Workspace)
        .unwrap();
    let err = prepare_glob_pattern(&workspace, "/elsewhere/**/*.rs").unwrap_err();

    assert!(err.to_string().contains("outside workspace"));
}

#[test]
fn relative_glob_patterns_normalize_parent_segments() {
    assert_eq!(
        normalize_relative_pattern("crates/roder-tools/../roder-app-server/src/*.rs"),
        "crates/roder-app-server/src/*.rs"
    );
    assert_eq!(normalize_relative_pattern("./../crates/*"), "crates/*");
}

#[tokio::test]
async fn grep_paging_result_includes_continuation_text_and_data() {
    let root = test_workspace("grep-paging");
    let dir = root.join("src");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.rs"), "needle a\n").unwrap();
    std::fs::write(dir.join("b.rs"), "needle b\n").unwrap();
    let workspace = Workspace::new(root.clone()).unwrap();
    let tool = GrepTool {
        workspace: workspace.clone(),
        backend: Arc::new(LocalWorkspaceBackend::new(workspace)),
    };

    let result = tool
        .execute(
            context(&root),
            call(
                "grep",
                json!({"query": "needle", "path": "src", "limit": 1}),
            ),
        )
        .await
        .unwrap();

    assert!(result.text.contains("call grep"));
    assert!(result.text.contains("\"offset\":1"));
    assert_eq!(result.data["omitted_lines"], 1);
    assert_eq!(result.data["continuation_tool"], "grep");
    assert_eq!(result.data["continuation_args"]["query"], "needle");
    assert_eq!(result.data["continuation_args"]["offset"], 1);

    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(outside);
}

#[tokio::test]
async fn glob_paging_result_includes_continuation_text_and_data() {
    let root = test_workspace("glob-paging");
    let dir = root.join("src");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.rs"), "").unwrap();
    std::fs::write(dir.join("b.rs"), "").unwrap();
    let workspace = Workspace::new(root.clone()).unwrap();
    let tool = GlobTool {
        workspace: workspace.clone(),
        backend: Arc::new(LocalWorkspaceBackend::new(workspace)),
    };

    let result = tool
        .execute(
            context(&root),
            call("glob", json!({"pattern": "src/*.rs", "limit": 1})),
        )
        .await
        .unwrap();

    assert!(result.text.contains("call glob"));
    assert!(result.text.contains("\"offset\":1"));
    assert_eq!(result.data["omitted_lines"], 1);
    assert_eq!(result.data["continuation_tool"], "glob");
    assert_eq!(result.data["continuation_args"]["pattern"], "src/*.rs");
    assert_eq!(result.data["continuation_args"]["offset"], 1);

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn glob_accepts_relative_parent_segments() {
    let root = test_workspace("glob-relative-parent");
    std::fs::create_dir_all(root.join("crates/roder-tools/src")).unwrap();
    std::fs::create_dir_all(root.join("crates/roder-app-server/src")).unwrap();
    std::fs::write(root.join("crates/roder-tools/src/lib.rs"), "").unwrap();
    std::fs::write(root.join("crates/roder-app-server/src/lib.rs"), "").unwrap();
    let workspace = Workspace::new(root.clone()).unwrap();
    let tool = GlobTool {
        workspace: workspace.clone(),
        backend: Arc::new(LocalWorkspaceBackend::new(workspace)),
    };

    let result = tool
        .execute(
            context(&root),
            call(
                "glob",
                json!({"pattern": "crates/roder-tools/../roder-app-server/src/*.rs"}),
            ),
        )
        .await
        .unwrap();

    assert_eq!(result.text, "crates/roder-app-server/src/lib.rs");

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn grep_response_format_concise_truncates_long_matches() {
    let root = test_workspace("grep-response-format");
    let dir = root.join("src");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.rs"), format!("needle {}\n", "x".repeat(400))).unwrap();
    let workspace = Workspace::new(root.clone()).unwrap();
    let tool = GrepTool {
        workspace: workspace.clone(),
        backend: Arc::new(LocalWorkspaceBackend::new(workspace)),
    };

    let concise = tool
        .execute(
            context(&root),
            call("grep", json!({"query": "needle", "path": "src"})),
        )
        .await
        .unwrap();
    let detailed = tool
        .execute(
            context(&root),
            call(
                "grep",
                json!({"query": "needle", "path": "src", "response_format": "detailed"}),
            ),
        )
        .await
        .unwrap();

    assert_eq!(concise.data["response_format"], "concise");
    assert!(concise.text.contains("..."));
    assert!(concise.text.len() < detailed.text.len());
    assert_eq!(detailed.data["response_format"], "detailed");
    assert!(!detailed.text.contains("..."));

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn grep_and_glob_skip_gitignored_paths() {
    let root = test_workspace("search-gitignore");
    std::fs::write(root.join(".gitignore"), ".claude/\nignored.log\n").unwrap();
    std::fs::create_dir_all(root.join(".claude/worktrees/agent")).unwrap();
    std::fs::write(
        root.join(".claude/worktrees/agent/app.rs"),
        "needle ignored\n",
    )
    .unwrap();
    std::fs::write(root.join("ignored.log"), "needle ignored\n").unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/lib.rs"), "needle kept\n").unwrap();
    let workspace = Workspace::new(root.clone()).unwrap();
    let backend = Arc::new(LocalWorkspaceBackend::new(workspace.clone()));
    let grep_tool = GrepTool {
        workspace: workspace.clone(),
        backend: backend.clone(),
    };
    let glob_tool = GlobTool { workspace, backend };

    let scan = grep_tool
        .execute(
            context(&root),
            call("grep", json!({"query": "needle", "mode": "scan"})),
        )
        .await
        .unwrap();
    let indexed = grep_tool
        .execute(
            context(&root),
            call("grep", json!({"query": "needle", "mode": "indexed"})),
        )
        .await
        .unwrap();
    let glob = glob_tool
        .execute(context(&root), call("glob", json!({"pattern": "*.rs"})))
        .await
        .unwrap();

    assert_eq!(scan.text, "src/lib.rs:1:needle kept");
    assert_eq!(indexed.text, scan.text);
    assert_eq!(glob.text, "src/lib.rs");

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn grep_treats_query_as_regex_by_default() {
    let root = test_workspace("grep-regex-default");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/a.rs"), "toolName here\ntool_name there\n").unwrap();
    let workspace = Workspace::new(root.clone()).unwrap();
    let tool = GrepTool {
        workspace: workspace.clone(),
        backend: Arc::new(LocalWorkspaceBackend::new(workspace)),
    };

    let result = tool
        .execute(
            context(&root),
            call("grep", json!({"query": "toolName|tool_name"})),
        )
        .await
        .unwrap();

    assert!(result.text.contains("src/a.rs:1:toolName here"));
    assert!(result.text.contains("src/a.rs:2:tool_name there"));

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn grep_invalid_regex_errors_with_literal_hint() {
    let root = test_workspace("grep-invalid-regex");
    let workspace = Workspace::new(root.clone()).unwrap();
    let tool = GrepTool {
        workspace: workspace.clone(),
        backend: Arc::new(LocalWorkspaceBackend::new(workspace)),
    };

    let err = tool
        .execute(context(&root), call("grep", json!({"query": "fetch("})))
        .await
        .unwrap_err()
        .to_string();

    assert!(err.contains("invalid regex"));
    assert!(err.contains("\"regex\": false"));

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn grep_zero_matches_explains_scope_and_literal_mode() {
    let root = test_workspace("grep-zero-match");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/a.rs"), "toolName here\n").unwrap();
    let workspace = Workspace::new(root.clone()).unwrap();
    let tool = GrepTool {
        workspace: workspace.clone(),
        backend: Arc::new(LocalWorkspaceBackend::new(workspace)),
    };

    let literal = tool
        .execute(
            context(&root),
            call(
                "grep",
                json!({"query": "toolName|tool_name", "regex": false}),
            ),
        )
        .await
        .unwrap();
    assert!(literal.text.contains("No matches for"));
    assert!(literal.text.contains("literal string"));
    assert!(literal.text.contains("retry with \"regex\": true"));

    let regex = tool
        .execute(
            context(&root),
            call("grep", json!({"query": "definitely_absent_needle"})),
        )
        .await
        .unwrap();
    assert!(regex.text.contains("No matches for"));
    assert!(!regex.text.contains("retry with"));

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn grep_searches_explicitly_scoped_ignored_directories() {
    let root = test_workspace("grep-ignored-scope");
    std::fs::write(root.join(".gitignore"), "node_modules/\n").unwrap();
    std::fs::create_dir_all(root.join("node_modules/pkg/dist")).unwrap();
    std::fs::write(
        root.join("node_modules/pkg/dist/index.js"),
        "export const syncApi = 1;\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/lib.rs"), "fn main() {}\n").unwrap();
    let workspace = Workspace::new(root.clone()).unwrap();
    let tool = GrepTool {
        workspace: workspace.clone(),
        backend: Arc::new(LocalWorkspaceBackend::new(workspace)),
    };

    // A workspace-root search still skips ignored directories...
    let from_root = tool
        .execute(context(&root), call("grep", json!({"query": "syncApi"})))
        .await
        .unwrap();
    assert!(from_root.text.contains("No matches for"));

    // ...but explicitly scoping into one searches it.
    for mode in ["auto", "scan", "indexed"] {
        let scoped = tool
            .execute(
                context(&root),
                call(
                    "grep",
                    json!({"query": "syncApi", "path": "node_modules/pkg/dist", "mode": mode}),
                ),
            )
            .await
            .unwrap();
        assert!(
            scoped
                .text
                .contains("node_modules/pkg/dist/index.js:1:export const syncApi = 1;"),
            "mode {mode}: {}",
            scoped.text
        );
    }

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn grep_accepts_home_relative_paths() {
    let root = test_workspace("grep-home-path");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("sub/notes.txt"), "needle here\n").unwrap();
    let workspace = Workspace::new(root.clone()).unwrap();
    let tool = GrepTool {
        workspace: workspace.clone(),
        backend: Arc::new(LocalWorkspaceBackend::new(workspace)),
    };

    // An absolute path stands in for `~` expansion: both resolve through
    // `resolve_existing`, which previously was discarded before searching.
    let result = tool
        .execute(
            context(&root),
            call(
                "grep",
                json!({"query": "needle", "path": root.join("sub").display().to_string()}),
            ),
        )
        .await
        .unwrap();

    assert!(result.text.contains("notes.txt:1:needle here"));

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn glob_supports_brace_patterns_and_explains_zero_matches() {
    let root = test_workspace("glob-braces");
    std::fs::create_dir_all(root.join("docs")).unwrap();
    std::fs::write(root.join("Cargo.toml"), "").unwrap();
    std::fs::write(root.join("docs/api.md"), "").unwrap();
    std::fs::write(root.join("main.rs"), "").unwrap();
    let workspace = Workspace::new(root.clone()).unwrap();
    let tool = GlobTool {
        workspace: workspace.clone(),
        backend: Arc::new(LocalWorkspaceBackend::new(workspace)),
    };

    let braces = tool
        .execute(
            context(&root),
            call("glob", json!({"pattern": "**/*.{toml,md}"})),
        )
        .await
        .unwrap();
    assert_eq!(braces.text, "Cargo.toml\ndocs/api.md");

    let empty = tool
        .execute(
            context(&root),
            call("glob", json!({"pattern": "**/*.{py,ipynb}"})),
        )
        .await
        .unwrap();
    assert!(empty.text.contains("No files matched pattern"));
    assert!(empty.text.contains("3 files considered"));

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn glob_accepts_absolute_patterns_outside_the_workspace_by_default() {
    let root = test_workspace("glob-outside-root");
    let outside = test_workspace("glob-outside-target");
    std::fs::create_dir_all(outside.join("src")).unwrap();
    let outside_file = outside.join("src/lib.rs");
    std::fs::write(&outside_file, "").unwrap();
    std::fs::write(outside.join("README.md"), "").unwrap();
    let workspace = Workspace::new(root.clone()).unwrap();
    let tool = GlobTool {
        workspace: workspace.clone(),
        backend: Arc::new(LocalWorkspaceBackend::new(workspace)),
    };

    let result = tool
        .execute(
            context(&root),
            call(
                "glob",
                json!({"pattern": format!("{}/**/*.rs", outside.display())}),
            ),
        )
        .await
        .unwrap();

    assert_eq!(result.text, outside_file.display().to_string().replace('\\', "/"));
    assert_eq!(result.data["files_considered"], 2);

    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(outside);
}

#[tokio::test]
async fn glob_rejects_absolute_patterns_outside_workspace_when_scope_is_workspace() {
    let root = test_workspace("glob-scoped-root");
    let outside = test_workspace("glob-scoped-outside");
    let workspace = Workspace::new_with_scope(root.clone(), ToolPathScope::Workspace).unwrap();
    let tool = GlobTool {
        workspace: workspace.clone(),
        backend: Arc::new(LocalWorkspaceBackend::new(workspace)),
    };

    let err = tool
        .execute(
            context(&root),
            call(
                "glob",
                json!({"pattern": format!("{}/**/*.rs", outside.display())}),
            ),
        )
        .await
        .unwrap_err()
        .to_string();

    assert!(err.contains("outside workspace"));

    let inside = tool
        .execute(
            context(&root),
            call(
                "glob",
                json!({"pattern": format!("{}/**/*.rs", root.display())}),
            ),
        )
        .await
        .unwrap();
    assert!(inside.text.contains("No files matched pattern"));

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
