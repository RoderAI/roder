//! Fork isolation and remote-adapter tests (roadmap phase 81, Tasks 5 + 8):
//! two agents editing the same source repo through separate forks have
//! disjoint write sets (fake provider and Git worktree provider), and a
//! remote runner session is representable as a `WorkspaceFork`. Offline.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::forks::{
    ForkCapabilities, ForkId, ForkPolicy, ForkProvider, ForkProviderDescriptor, ForkProvenance,
    ForkReason, ForkRequest, ForkStatus, RemoveForkPolicy, RemoveForkResult, WorkspaceFork,
};
use roder_api::remote_runner::{RemoteRunnerProvider, RunnerDestination, RunnerManifest};
use roder_core::fake_provider::FakeInferenceEngine;
use roder_core::forks::{REMOTE_RUNNER_FORK_PROVIDER_ID, RemoteRunnerForkAdapter};
use roder_core::{Runtime, RuntimeConfig};
use roder_ext_runner_unix_local::UnixLocalRunnerProvider;
use time::OffsetDateTime;

/// Copy-based fake fork provider: forks are plain directory copies.
struct CopyDirForkProvider;

#[async_trait::async_trait]
impl ForkProvider for CopyDirForkProvider {
    fn descriptor(&self) -> ForkProviderDescriptor {
        ForkProviderDescriptor {
            id: "fake-copy".to_string(),
            display_name: "Copy dir".to_string(),
            capabilities: ForkCapabilities {
                create: true,
                list: false,
                remove: true,
                resume: true,
                ..ForkCapabilities::default()
            },
        }
    }

    async fn create_fork(&self, request: ForkRequest) -> anyhow::Result<WorkspaceFork> {
        let name = request.name.clone().unwrap_or_else(|| "fork".to_string());
        let workspace = request.source_workspace.join(".forks").join(&name);
        std::fs::create_dir_all(&workspace)?;
        for entry in std::fs::read_dir(&request.source_workspace)? {
            let entry = entry?;
            if entry.path().is_file() {
                std::fs::copy(entry.path(), workspace.join(entry.file_name()))?;
            }
        }
        Ok(WorkspaceFork {
            id: workspace.display().to_string(),
            provider_id: "fake-copy".to_string(),
            source_workspace: request.source_workspace,
            workspace,
            status: ForkStatus::Active,
            provenance: ForkProvenance::at(OffsetDateTime::now_utc()),
            cleanup: Default::default(),
            metadata: serde_json::json!({}),
        })
    }

    async fn list_forks(&self, _source: &Path) -> anyhow::Result<Vec<WorkspaceFork>> {
        Ok(Vec::new())
    }

    async fn resume_fork(&self, id: &ForkId) -> anyhow::Result<WorkspaceFork> {
        anyhow::bail!("unknown fork {id}")
    }

    async fn remove_fork(
        &self,
        id: &ForkId,
        policy: RemoveForkPolicy,
    ) -> anyhow::Result<RemoveForkResult> {
        std::fs::remove_dir_all(&policy.confirm_workspace)?;
        Ok(RemoveForkResult {
            id: id.clone(),
            removed: true,
            workspace: policy.confirm_workspace,
        })
    }
}

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git").arg("-C").arg(root).args(args).output().unwrap();
    assert!(output.status.success(), "git {args:?} failed");
}

fn temp_repo(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("roder-fork-iso-{label}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "--initial-branch", "main"]);
    git(&root, &["config", "user.email", "t@t"]);
    git(&root, &["config", "user.name", "T"]);
    std::fs::write(root.join("shared.txt"), "shared\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "initial"]);
    root.canonicalize().unwrap()
}

fn runtime() -> Arc<Runtime> {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FakeInferenceEngine));
    builder.fork_provider(Arc::new(CopyDirForkProvider));
    builder.fork_provider(Arc::new(roder_ext_git::GitWorktreeForkProvider));
    Arc::new(Runtime::new(builder.build().unwrap(), RuntimeConfig::default()).unwrap())
}

fn request(source: &Path, name: &str) -> ForkRequest {
    ForkRequest {
        source_workspace: source.to_path_buf(),
        name: Some(name.to_string()),
        reason: ForkReason::SubagentLane,
        policy: ForkPolicy::default(),
        provider_config: serde_json::json!({}),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn fork_isolation_holds_for_fake_and_git_providers() {
    let runtime = runtime();
    for provider in ["fake-copy", "git-worktree"] {
        let repo = temp_repo(provider);
        let lane_a = runtime
            .create_workspace_fork(provider, request(&repo, "lane-a"))
            .await
            .unwrap();
        let lane_b = runtime
            .create_workspace_fork(provider, request(&repo, "lane-b"))
            .await
            .unwrap();
        assert_ne!(lane_a.workspace, lane_b.workspace);

        std::fs::write(lane_a.workspace.join("a.txt"), "agent a").unwrap();
        std::fs::write(lane_b.workspace.join("b.txt"), "agent b").unwrap();

        assert!(!lane_a.workspace.join("b.txt").exists(), "{provider}");
        assert!(!lane_b.workspace.join("a.txt").exists(), "{provider}");
        assert!(!repo.join("a.txt").exists(), "{provider}: source untouched");
        assert!(!repo.join("b.txt").exists(), "{provider}: source untouched");
        let _ = std::fs::remove_dir_all(&repo);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_fork_manager_rejects_unknown_providers_and_bad_requests() {
    let runtime = runtime();
    let error = runtime
        .create_workspace_fork("missing-provider", request(Path::new("/tmp"), "x"))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("not installed"), "{error}");
    assert!(error.contains("fake-copy") && error.contains("git-worktree"), "{error}");

    let error = runtime
        .create_workspace_fork("fake-copy", request(Path::new("relative/path"), "x"))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("absolute path"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn remote_runner_sessions_are_representable_as_forks() {
    let workspace = std::env::temp_dir().join(format!("roder-runner-fork-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&workspace).unwrap();
    let destination = RunnerDestination {
        id: "local-dest".to_string(),
        provider_id: "unix-local".to_string(),
        config: serde_json::json!({ "root": workspace.display().to_string() }),
        default_manifest: RunnerManifest::default(),
    };
    let adapter = RemoteRunnerForkAdapter::new(
        Arc::new(UnixLocalRunnerProvider::default()) as Arc<dyn RemoteRunnerProvider>,
        destination,
        workspace.clone(),
    );

    let descriptor = adapter.descriptor();
    assert_eq!(descriptor.id, REMOTE_RUNNER_FORK_PROVIDER_ID);
    assert!(descriptor.capabilities.remote_compute);

    let fork = adapter
        .create_fork(ForkRequest {
            source_workspace: workspace.clone(),
            name: Some("remote-lane".to_string()),
            reason: ForkReason::TaskLane,
            policy: ForkPolicy::default(),
            provider_config: serde_json::json!({}),
        })
        .await
        .unwrap();
    assert_eq!(fork.status, ForkStatus::Active);
    assert!(fork.provenance.session_id.is_some());
    assert_eq!(fork.workspace, workspace);

    // The live session is reachable for tool wiring; file/process APIs stay
    // on the RemoteRunnerSession contract.
    let session = adapter.session(&fork.id).await.expect("live session");
    session
        .write_file(roder_api::remote_runner::RunnerFileWriteRequest {
            path: "hello.txt".into(),
            contents: b"from remote fork".to_vec(),
        })
        .await
        .unwrap();
    assert!(workspace.join("hello.txt").exists());

    // Removal is path-confirmed and closes the session deterministically.
    let denied = adapter
        .remove_fork(
            &fork.id,
            RemoveForkPolicy {
                confirm_workspace: PathBuf::from("/wrong"),
            },
        )
        .await;
    assert!(denied.is_err());
    let removed = adapter
        .remove_fork(
            &fork.id,
            RemoveForkPolicy {
                confirm_workspace: workspace.clone(),
            },
        )
        .await
        .unwrap();
    assert!(removed.removed);
    assert!(adapter.session(&fork.id).await.is_none());

    let _ = std::fs::remove_dir_all(&workspace);
}
