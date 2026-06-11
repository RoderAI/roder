//! Git worktree `ForkProvider` tests (roadmap phase 81, Task 3): create,
//! list, resume, remove against temp repositories, offline.

use std::path::{Path, PathBuf};
use std::process::Command;

use roder_api::forks::{
    ForkPolicy, ForkProvider, ForkReason, ForkRequest, ForkStatus, RemoveForkPolicy,
};
use roder_ext_git::GitWorktreeForkProvider;

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

fn temp_repo(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "roder-fork-provider-{label}-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "--initial-branch", "main"]);
    git(&root, &["config", "user.email", "test@roder.dev"]);
    git(&root, &["config", "user.name", "Roder Test"]);
    std::fs::write(root.join("README.md"), "# fixture\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "initial"]);
    root.canonicalize().unwrap()
}

fn request(repo: &Path, name: &str) -> ForkRequest {
    ForkRequest {
        source_workspace: repo.to_path_buf(),
        name: Some(name.to_string()),
        reason: ForkReason::Experiment,
        policy: ForkPolicy::default(),
        provider_config: serde_json::json!({}),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn create_list_resume_remove_round_trip() {
    let repo = temp_repo("lifecycle");
    let provider = GitWorktreeForkProvider;

    let descriptor = provider.descriptor();
    assert_eq!(descriptor.id, "git-worktree");
    assert!(descriptor.capabilities.create && descriptor.capabilities.remove);
    assert!(!descriptor.capabilities.remote_compute);

    let fork = provider.create_fork(request(&repo, "parser-fix")).await.unwrap();
    assert_eq!(fork.status, ForkStatus::Active);
    assert_eq!(fork.provider_id, "git-worktree");
    assert_eq!(fork.source_workspace, repo);
    assert!(fork.workspace.join("README.md").exists());
    assert_eq!(
        fork.provenance.branch.as_deref(),
        Some("roder/fork/parser-fix")
    );
    assert_eq!(fork.provenance.source_branch.as_deref(), Some("main"));
    assert!(fork.provenance.source_commit.is_some());

    // Listing sees the fork (and not the primary worktree).
    let listed = provider.list_forks(&repo).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].workspace, fork.workspace);
    assert_eq!(listed[0].provenance.branch, fork.provenance.branch);

    // Resume by id re-resolves provenance.
    let resumed = provider.resume_fork(&fork.id).await.unwrap();
    assert_eq!(resumed.status, ForkStatus::Active);
    assert_eq!(resumed.source_workspace, repo);

    // Removal requires the exact workspace path.
    let denied = provider
        .remove_fork(
            &fork.id,
            RemoveForkPolicy {
                confirm_workspace: PathBuf::from("/wrong"),
            },
        )
        .await;
    assert!(denied.is_err(), "removal must be path-confirmed");

    let removed = provider
        .remove_fork(
            &fork.id,
            RemoveForkPolicy {
                confirm_workspace: fork.workspace.clone(),
            },
        )
        .await
        .unwrap();
    assert!(removed.removed);
    assert!(!fork.workspace.exists());
    assert!(provider.list_forks(&repo).await.unwrap().is_empty());

    let _ = std::fs::remove_dir_all(&repo);
}

#[tokio::test(flavor = "multi_thread")]
async fn dirty_sources_fail_closed_and_policy_opt_in_is_rejected_explicitly() {
    let repo = temp_repo("dirty");
    let provider = GitWorktreeForkProvider;
    std::fs::write(repo.join("wip.txt"), "uncommitted").unwrap();

    let error = provider
        .create_fork(request(&repo, "blocked"))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("uncommitted changes"), "{error}");

    // The provider cannot honor allow_dirty_source and says so instead of
    // silently copying dirty state.
    let mut dirty_request = request(&repo, "blocked");
    dirty_request.policy.allow_dirty_source = true;
    let error = provider
        .create_fork(dirty_request)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("cannot fork dirty sources"), "{error}");

    let _ = std::fs::remove_dir_all(&repo);
}

#[tokio::test(flavor = "multi_thread")]
async fn non_git_workspaces_and_missing_forks_report_clearly() {
    let provider = GitWorktreeForkProvider;
    let plain_dir = std::env::temp_dir().join(format!("roder-fork-plain-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&plain_dir).unwrap();

    let error = provider
        .create_fork(request(&plain_dir, "nope"))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("not inside a Git repository"), "{error}");

    // Resuming a vanished fork reports Missing rather than failing.
    let gone = plain_dir.join("never-existed");
    let resumed = provider
        .resume_fork(&gone.display().to_string())
        .await
        .unwrap();
    assert_eq!(resumed.status, ForkStatus::Missing);

    let _ = std::fs::remove_dir_all(&plain_dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn two_forks_have_isolated_write_sets() {
    let repo = temp_repo("isolation");
    let provider = GitWorktreeForkProvider;

    let fork_a = provider.create_fork(request(&repo, "lane-a")).await.unwrap();
    let fork_b = provider.create_fork(request(&repo, "lane-b")).await.unwrap();
    assert_ne!(fork_a.workspace, fork_b.workspace);

    std::fs::write(fork_a.workspace.join("a.txt"), "from lane a").unwrap();
    std::fs::write(fork_b.workspace.join("b.txt"), "from lane b").unwrap();

    assert!(!fork_a.workspace.join("b.txt").exists());
    assert!(!fork_b.workspace.join("a.txt").exists());
    assert!(!repo.join("a.txt").exists(), "source repo stays untouched");
    assert!(!repo.join("b.txt").exists());

    let _ = std::fs::remove_dir_all(&repo);
}
