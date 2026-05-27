use super::*;
use crate::backend::LocalWorkspaceBackend;
use roder_api::tools::{LocalWorkspaceHandle, ToolExecutionContext};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[test]
fn wildcard_match_supports_star_and_question_mark() {
    assert!(wildcard_match("src/*.rs", "src/main.rs"));
    assert!(wildcard_match("src/??.rs", "src/io.rs"));
    assert!(!wildcard_match("src/*.rs", "README.md"));
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
