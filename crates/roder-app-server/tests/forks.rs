//! App-server e2e coverage for workspace forks (roadmap phases 90 + 81). Uses a temporary local Git repository, the offline
//! fake provider, and a JSONL thread store — no live providers.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::forks::ForkStatus;
use roder_app_server::{AppServer, AppServerFeatureConfig, LocalAppClient};
use roder_core::fake_provider::FakeInferenceEngine;
use roder_core::{Runtime, RuntimeConfig};
use roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory;
use roder_protocol::{
    ForksCreateParams, ForksCreateResult, ForksListParams, ForksListResult,
    ForksProvidersListResult, ForksRemoveParams, ForksRemoveResult, JsonRpcRequest,
    ThreadForkParams, ThreadForkResult, ThreadForkStatusParams, ThreadForkStatusResult,
    ThreadListParams, ThreadListResult, ThreadRemoveForkParams, ThreadRemoveForkResult,
    ThreadStartParams, ThreadStartResult, WorkspaceCreateParams, WorkspaceCreateResult,
    WorkspaceRootInput,
};

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "roder-fork-e2e-{label}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn temp_repo(label: &str) -> PathBuf {
    let root = temp_dir(label);
    git(&root, &["init", "--initial-branch", "main"]);
    git(&root, &["config", "user.email", "test@roder.dev"]);
    git(&root, &["config", "user.name", "Roder Test"]);
    std::fs::write(root.join("README.md"), "# fixture\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "initial"]);
    root.canonicalize().unwrap()
}

fn client(label: &str) -> LocalAppClient {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FakeInferenceEngine));
    builder.fork_provider(Arc::new(roder_ext_git::GitWorktreeForkProvider));
    builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
        base_path: temp_dir(&format!("{label}-threads")),
    }));
    let runtime =
        Arc::new(Runtime::new(builder.build().unwrap(), RuntimeConfig::default()).unwrap());
    let feature_config = AppServerFeatureConfig::default().with_workspace_registry_path(
        temp_dir(&format!("{label}-registry")).join("workspaces.json"),
    );
    LocalAppClient::new(Arc::new(AppServer::with_feature_config(
        runtime,
        feature_config,
    )))
}

async fn request<T: serde::de::DeserializeOwned>(
    client: &LocalAppClient,
    method: &str,
    params: serde_json::Value,
) -> T {
    let response = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(method)),
            method: method.to_string(),
            params: Some(params),
        })
        .await;
    assert!(
        response.error.is_none(),
        "RPC error for {method}: {:?}",
        response.error
    );
    serde_json::from_value(response.result.unwrap()).unwrap()
}

async fn request_error(client: &LocalAppClient, method: &str, params: serde_json::Value) -> String {
    let response = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(method)),
            method: method.to_string(),
            params: Some(params),
        })
        .await;
    response
        .error
        .unwrap_or_else(|| panic!("{method} unexpectedly succeeded"))
        .message
}

async fn start_parent_thread(client: &LocalAppClient, repo: &Path) -> String {
    let workspace: WorkspaceCreateResult = request(
        client,
        "workspace/create",
        serde_json::to_value(WorkspaceCreateParams {
            name: None,
            roots: vec![WorkspaceRootInput {
                path: repo.display().to_string(),
                name: None,
            }],
            default_root_path: Some(repo.display().to_string()),
        })
        .unwrap(),
    )
    .await;
    let started: ThreadStartResult = request(
        client,
        "thread/start",
        serde_json::to_value(ThreadStartParams {
            selection: None,
            workspace_id: workspace.workspace.id.clone(),
            root_id: Some(workspace.workspace.default_root_id.clone()),
            model: Some("mock".to_string()),
            model_provider: None,
            reasoning: None,
            cwd: None,
            tool_allowlist: None,
            developer_instructions: None,
            external_tools: None,
            mcp_auth_token: None,
            runner: None,
            ephemeral: false,
        })
        .unwrap(),
    )
    .await;
    started.thread.id
}

#[tokio::test(flavor = "multi_thread")]
async fn fork_worktree_lifecycle_works_over_public_json_rpc() {
    let repo = temp_repo("lifecycle");
    let client = client("lifecycle");
    let parent_id = start_parent_thread(&client, &repo).await;

    // Fork the parent into a worktree-backed child thread.
    let forked: ThreadForkResult = request(
        &client,
        "thread/fork",
        serde_json::to_value(ThreadForkParams {
            thread_id: parent_id.clone(),
            name: "parser-experiment".to_string(),
            from_turn_id: None,
            provider: None,
            provider_config: None,
        })
        .unwrap(),
    )
    .await;
    assert_eq!(
        forked.thread.parent_thread_id.as_deref(),
        Some(parent_id.as_str())
    );
    assert_eq!(forked.fork.status, ForkStatus::Active);
    assert_eq!(
        forked.thread.cwd,
        forked.fork.workspace.display().to_string()
    );
    assert_ne!(forked.thread.cwd, repo.display().to_string());
    assert!(Path::new(&forked.thread.cwd).join("README.md").exists());

    // thread/list exposes compact fork metadata for UI clients.
    let listed: ThreadListResult = request(
        &client,
        "thread/list",
        serde_json::to_value(ThreadListParams {
            limit: Some(10),
            cursor: None,
        })
        .unwrap(),
    )
    .await;
    let child_row = listed
        .data
        .iter()
        .find(|thread| thread.id == forked.thread.id)
        .expect("child thread listed");
    assert_eq!(
        child_row.parent_thread_id.as_deref(),
        Some(parent_id.as_str())
    );
    assert_eq!(
        child_row.workspace_fork.as_ref().map(|fork| fork.status),
        Some(ForkStatus::Active)
    );

    // fork_status reports an intact worktree.
    let status: ThreadForkStatusResult = request(
        &client,
        "thread/fork_status",
        serde_json::to_value(ThreadForkStatusParams {
            thread_id: forked.thread.id.clone(),
        })
        .unwrap(),
    )
    .await;
    assert!(!status.workspace_missing);
    assert_eq!(status.parent_thread_id.as_deref(), Some(parent_id.as_str()));

    // Removal requires the exact worktree path.
    let message = request_error(
        &client,
        "thread/remove_fork",
        serde_json::to_value(ThreadRemoveForkParams {
            thread_id: forked.thread.id.clone(),
            confirm_path: "/definitely/wrong".to_string(),
        })
        .unwrap(),
    )
    .await;
    assert!(message.contains("path-confirmed"), "{message}");
    assert!(Path::new(&forked.thread.cwd).exists());

    let removed: ThreadRemoveForkResult = request(
        &client,
        "thread/remove_fork",
        serde_json::to_value(ThreadRemoveForkParams {
            thread_id: forked.thread.id.clone(),
            confirm_path: forked.fork.workspace.display().to_string().clone(),
        })
        .unwrap(),
    )
    .await;
    assert_eq!(removed.fork.status, ForkStatus::Removed);
    assert!(!Path::new(&forked.thread.cwd).exists());

    // The provider-facing forks/* surface works over the same registry.
    let providers: ForksProvidersListResult =
        request(&client, "forks/providers/list", serde_json::json!({})).await;
    assert!(
        providers
            .providers
            .iter()
            .any(|provider| provider.id == "git-worktree")
    );

    let created: ForksCreateResult = request(
        &client,
        "forks/create",
        serde_json::to_value(ForksCreateParams {
            source_workspace: repo.display().to_string(),
            name: Some("bare-fork".to_string()),
            provider: None,
            provider_config: None,
        })
        .unwrap(),
    )
    .await;
    assert_eq!(created.fork.status, ForkStatus::Active);

    let listed: ForksListResult = request(
        &client,
        "forks/list",
        serde_json::to_value(ForksListParams {
            source_workspace: repo.display().to_string(),
            provider: None,
        })
        .unwrap(),
    )
    .await;
    assert!(
        listed
            .forks
            .iter()
            .any(|fork| fork.workspace == created.fork.workspace)
    );

    let removed: ForksRemoveResult = request(
        &client,
        "forks/remove",
        serde_json::to_value(ForksRemoveParams {
            fork_id: created.fork.id.clone(),
            provider: None,
            confirm_workspace: created.fork.workspace.display().to_string(),
        })
        .unwrap(),
    )
    .await;
    assert!(removed.removed);
    assert!(!created.fork.workspace.exists());

    let _ = std::fs::remove_dir_all(&repo);
}

#[tokio::test(flavor = "multi_thread")]
async fn fork_worktree_fails_closed_for_dirty_parents_and_missing_worktrees() {
    let repo = temp_repo("dirty");
    let client = client("dirty");
    let parent_id = start_parent_thread(&client, &repo).await;

    // Dirty parent refuses the fork over the public path.
    std::fs::write(repo.join("wip.txt"), "uncommitted").unwrap();
    let message = request_error(
        &client,
        "thread/fork",
        serde_json::to_value(ThreadForkParams {
            thread_id: parent_id.clone(),
            name: "blocked".to_string(),
            from_turn_id: None,
            provider: None,
            provider_config: None,
        })
        .unwrap(),
    )
    .await;
    assert!(message.contains("uncommitted changes"), "{message}");

    // Clean up and fork successfully, then break the worktree out-of-band.
    std::fs::remove_file(repo.join("wip.txt")).unwrap();
    let forked: ThreadForkResult = request(
        &client,
        "thread/fork",
        serde_json::to_value(ThreadForkParams {
            thread_id: parent_id,
            name: "orphaned".to_string(),
            from_turn_id: None,
            provider: None,
            provider_config: None,
        })
        .unwrap(),
    )
    .await;
    std::fs::remove_dir_all(&forked.thread.cwd).unwrap();

    let status: ThreadForkStatusResult = request(
        &client,
        "thread/fork_status",
        serde_json::to_value(ThreadForkStatusParams {
            thread_id: forked.thread.id.clone(),
        })
        .unwrap(),
    )
    .await;
    assert!(
        status.workspace_missing,
        "missing worktree must be reported"
    );

    let _ = std::fs::remove_dir_all(&repo);
}
