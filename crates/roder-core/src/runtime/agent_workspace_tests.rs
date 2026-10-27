use super::*;
use roder_api::catalog::PROVIDER_MOCK;
use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::forks::ForkStatus;
use roder_api::inference::{
    InferenceCapabilities, InferenceEventStream, InferenceProviderContext, ModelDescriptor,
};
use roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory;
use std::process::Command;

struct WriteInChild {
    finished: tokio::sync::mpsc::UnboundedSender<(ThreadId, bool)>,
}

#[async_trait::async_trait]
impl InferenceEngine for WriteInChild {
    fn id(&self) -> String {
        PROVIDER_MOCK.into()
    }
    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities::coding_agent_default()
    }
    async fn list_models(
        &self,
        _: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(roder_api::catalog::models_for_provider(PROVIDER_MOCK, true))
    }
    async fn stream_turn(
        &self,
        ctx: InferenceTurnContext<'_>,
        _: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let result = ctx
            .tool_executor
            .as_ref()
            .unwrap()
            .execute(ToolCallCompleted {
                id: format!("write-{}", ctx.thread_id),
                name: "write_file".into(),
                arguments: serde_json::json!({
                    "path": "child.txt", "content": ctx.thread_id,
                })
                .to_string(),
            })
            .await?;
        let _ = self.finished.send((ctx.thread_id.into(), result.is_error));
        Ok(Box::pin(futures::stream::pending()))
    }
}

fn git(root: &std::path::Path, args: &[&str]) {
    let result = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

async fn fixture() -> (
    tempfile::TempDir,
    Arc<Runtime>,
    ThreadId,
    tokio::sync::mpsc::UnboundedReceiver<(ThreadId, bool)>,
) {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "--initial-branch", "main"]);
    git(&repo, &["config", "user.email", "test@roder.dev"]);
    git(&repo, &["config", "user.name", "Roder test"]);
    std::fs::write(repo.join("README.md"), "base").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "base"]);
    let (finished, receiver) = tokio::sync::mpsc::unbounded_channel();
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(WriteInChild { finished }));
    builder.fork_provider(Arc::new(roder_ext_git::GitWorktreeForkProvider));
    builder.tool_contributor(Arc::new(
        roder_tools::BuiltinCodingToolsContributor::new(&repo).unwrap(),
    ));
    builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
        base_path: temp.path().join("threads"),
    }));
    let runtime = Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                workspace: Some(repo.display().to_string()),
                default_provider: PROVIDER_MOCK.into(),
                default_model: "mock".into(),
                policy_mode: PolicyMode::Bypass,
                team_data_dir: Some(temp.path().join("teams")),
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    );
    let parent = runtime
        .create_thread_with(CreateThreadRequest {
            title: None,
            workspace: repo.display().to_string(),
            workspace_id: None,
            root_id: None,
            provider: Some(PROVIDER_MOCK.into()),
            model: Some("mock".into()),
            selection_mode: Some(ModelSelectionMode::manual(PROVIDER_MOCK, "mock", None)),
            tool_allowlist: Vec::new(),
            service_tier: None,
            developer_instructions: None,
            external_tools: Vec::new(),
            runner: None,
        })
        .await
        .unwrap();
    runtime.active_turn_contexts.write().await.insert(
        "parent-turn".into(),
        InheritedTurnContext {
            workspace: repo.display().to_string(),
            instructions: crate::default_instructions(),
            developer_context: None,
        },
    );
    (temp, runtime, parent.thread_id, receiver)
}

async fn spawn(
    runtime: &Arc<Runtime>,
    parent: &ThreadId,
    name: &str,
) -> anyhow::Result<(TeamState, TurnId)> {
    runtime
        .spawn_and_start_team_member_for_caller(
            parent,
            &"parent-turn".into(),
            TeamMemberStartRequest {
                name: name.into(),
                model_provider: None,
                model: None,
            },
            None,
            "none".into(),
            None,
            "Write in your own workspace".into(),
            AgentWorkspaceIsolation::Worktree,
        )
        .await
}

#[tokio::test]
async fn isolated_agents_execute_tools_in_separate_owned_workspaces_and_retain_dirty_work() {
    let (_temp, runtime, parent, mut receiver) = fixture().await;
    let mut children = Vec::new();
    for name in ["lane_a", "lane_b"] {
        let (team, turn) = spawn(&runtime, &parent, name).await.unwrap();
        let child = team.members.last().unwrap().thread_id.clone();
        let (executed, failed) =
            tokio::time::timeout(std::time::Duration::from_secs(10), receiver.recv())
                .await
                .unwrap()
                .unwrap();
        assert_eq!(executed, child);
        assert!(!failed);
        let metadata = runtime.load_thread_metadata(&child).await.unwrap().unwrap();
        let fork = metadata.workspace_fork.as_ref().unwrap();
        let error = runtime
            .remove_workspace_fork(
                &fork.provider_id,
                &fork.id,
                roder_api::forks::RemoveForkPolicy {
                    confirm_workspace: fork.workspace.clone(),
                },
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("active turn"), "{error}");
        let owner = fork.ownership.as_ref().unwrap();
        assert_eq!(owner.creator_thread_id.as_ref(), Some(&parent));
        assert_eq!(owner.owner_thread_id.as_ref(), Some(&child));
        assert_eq!(
            runtime.workspace_for_thread(&child).await.unwrap(),
            metadata.workspace
        );
        assert_eq!(
            runtime
                .team_member_turn_contexts
                .lock()
                .await
                .get(&child)
                .unwrap()
                .workspace,
            metadata.workspace,
        );
        assert_eq!(
            std::fs::read_to_string(fork.workspace.join("child.txt")).unwrap(),
            child
        );
        children.push((child, turn, fork.workspace.clone()));
    }
    assert_ne!(children[0].2, children[1].2);
    assert!(!runtime.workspace.join("child.txt").exists());
    for (child, turn, workspace) in children {
        runtime.interrupt_turn(child.clone(), turn).await.unwrap();
        // Failed-start cleanup is deliberately conservative too.
        runtime.discard_failed_agent_workspace(&child).await;
        assert!(workspace.join("child.txt").exists());
    }
}

#[tokio::test]
async fn failed_start_cleanup_removes_only_clean_owned_workspace() {
    let (_temp, runtime, parent, _receiver) = fixture().await;
    let team = runtime
        .spawn_team_member_for_caller_locked(
            &parent,
            &"parent-turn".into(),
            TeamMemberStartRequest {
                name: "never_started".into(),
                model_provider: None,
                model: None,
            },
            None,
            "none".into(),
            None,
            AgentWorkspaceIsolation::Worktree,
        )
        .await
        .unwrap();
    let child = &team.members.last().unwrap().thread_id;
    let before = runtime.load_thread_metadata(child).await.unwrap().unwrap();
    assert!(std::path::Path::new(&before.workspace).is_dir());
    runtime.discard_failed_agent_workspace(child).await;
    assert!(!std::path::Path::new(&before.workspace).exists());
    let after = runtime.load_thread_metadata(child).await.unwrap().unwrap();
    assert_eq!(after.workspace_fork.unwrap().status, ForkStatus::Removed);
}

#[tokio::test]
async fn dirty_source_refuses_isolation_without_creating_an_agent_checkout() {
    let (_temp, runtime, parent, _receiver) = fixture().await;
    std::fs::write(runtime.workspace.join("dirty.txt"), "not committed").unwrap();
    let error = spawn(&runtime, &parent, "blocked")
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("uncommitted changes"), "{error}");
    assert!(
        runtime
            .list_workspace_forks("git-worktree", &runtime.workspace)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        std::fs::read_to_string(runtime.workspace.join("dirty.txt")).unwrap(),
        "not committed"
    );
}

#[tokio::test]
async fn remote_parent_cannot_silently_spawn_a_host_worktree() {
    let (_temp, runtime, parent, _receiver) = fixture().await;
    let mut metadata = runtime
        .load_thread_metadata(&parent)
        .await
        .unwrap()
        .unwrap();
    metadata.runner_destination = Some(roder_api::remote_runner::RunnerDestination {
        id: "remote".into(),
        provider_id: "not-installed".into(),
        config: serde_json::json!({}),
        default_manifest: Default::default(),
    });
    runtime
        .thread_store
        .as_ref()
        .unwrap()
        .update_thread_metadata(metadata)
        .await
        .unwrap();
    let error = spawn(&runtime, &parent, "remote_blocked")
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("requires a local workspace"), "{error}");
    assert!(
        runtime
            .list_workspace_forks("git-worktree", &runtime.workspace)
            .await
            .unwrap()
            .is_empty()
    );
}
