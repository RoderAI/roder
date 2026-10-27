//! `ForkProvider` implementation backed by Git worktrees (roadmap
//! phase 81, Task 3). Wraps the phase-90 worktree helpers behind the
//! provider-neutral fork contract: fork ids are the absolute worktree
//! paths, creation fails closed on dirty sources (Roder-owned `.roder/`
//! state exempt), and removal is path-confirmed and restricted to
//! Git-registered Roder worktrees.

use std::path::{Path, PathBuf};

use roder_api::forks::{
    ForkCapabilities, ForkId, ForkProvenance, ForkProvider, ForkProviderDescriptor, ForkRequest,
    ForkStatus, RemoveForkPolicy, RemoveForkResult, WorkspaceFork,
};
use time::OffsetDateTime;

use crate::ownership::{lock_repository, read_owned};
use crate::worktree::{
    GitWorktreeForkRequest, create_worktree_fork_unlocked, list_worktree_paths,
    remove_worktree_fork, repo_root, run_git,
};

pub const GIT_WORKTREE_FORK_PROVIDER_ID: &str = "git-worktree";

#[derive(Debug, Default)]
pub struct GitWorktreeForkProvider;

impl GitWorktreeForkProvider {
    fn fork_from_worktree(root: &Path, worktree: &Path) -> WorkspaceFork {
        let branch = run_git(worktree, &["branch", "--show-current"])
            .ok()
            .map(|branch| branch.trim().to_string())
            .filter(|branch| !branch.is_empty());
        let commit = run_git(worktree, &["rev-parse", "HEAD"])
            .ok()
            .map(|commit| commit.trim().to_string());
        WorkspaceFork {
            id: worktree.display().to_string(),
            ownership: None,
            provider_id: GIT_WORKTREE_FORK_PROVIDER_ID.to_string(),
            source_workspace: root.to_path_buf(),
            workspace: worktree.to_path_buf(),
            status: ForkStatus::Active,
            provenance: ForkProvenance {
                branch,
                source_branch: None,
                source_commit: commit,
                snapshot_id: None,
                session_id: None,
                created_at: OffsetDateTime::now_utc(),
            },
            cleanup: Default::default(),
            metadata: serde_json::json!({}),
        }
    }
}

/// Resolves the primary repository root for an existing worktree path.
fn source_root_for_worktree(worktree: &Path) -> anyhow::Result<PathBuf> {
    let common = run_git(worktree, &["rev-parse", "--git-common-dir"])?;
    let common = PathBuf::from(common.trim());
    let common = if common.is_absolute() {
        common
    } else {
        worktree.join(common)
    };
    let root = common
        .parent()
        .ok_or_else(|| anyhow::anyhow!("cannot resolve repository root for the fork"))?;
    Ok(root.canonicalize().unwrap_or_else(|_| root.to_path_buf()))
}

#[async_trait::async_trait]
impl ForkProvider for GitWorktreeForkProvider {
    fn descriptor(&self) -> ForkProviderDescriptor {
        ForkProviderDescriptor {
            id: GIT_WORKTREE_FORK_PROVIDER_ID.to_string(),
            display_name: "Git worktree".to_string(),
            capabilities: ForkCapabilities {
                create: true,
                list: true,
                remove: true,
                resume: true,
                diff_summary: false,
                merge_back: false,
                copy_on_write: false,
                remote_compute: false,
            },
        }
    }

    async fn create_fork(&self, request: ForkRequest) -> anyhow::Result<WorkspaceFork> {
        anyhow::ensure!(
            !request.policy.allow_dirty_source,
            "the git-worktree provider cannot fork dirty sources; commit or stash changes first"
        );
        let base_dir = request
            .provider_config
            .get("baseDir")
            .and_then(serde_json::Value::as_str)
            .map(PathBuf::from);
        let worktree_request = GitWorktreeForkRequest {
            source_workspace: request.source_workspace.clone(),
            fork_name: request.name.clone().unwrap_or_else(|| "fork".to_string()),
            base_dir,
        };
        tokio::task::spawn_blocking(move || {
            let root = repo_root(&worktree_request.source_workspace)?;
            let _lock = lock_repository(&root)?;
            // An owner thread gets exactly one fork per repository, even if
            // callers retry concurrently or after restarting the process.
            if let Some(owner) = &request.ownership
                && owner.owner_thread_id.is_some()
            {
                for path in list_worktree_paths(&root)? {
                    if let Some(existing) = read_owned(&path)?
                        && existing
                            .ownership
                            .as_ref()
                            .and_then(|value| value.owner_thread_id.as_ref())
                            == owner.owner_thread_id.as_ref()
                    {
                        anyhow::ensure!(
                            existing.ownership.as_ref() == Some(owner),
                            "fork owner already belongs to a different creator"
                        );
                        return Ok(existing);
                    }
                }
            }
            let created = create_worktree_fork_unlocked(
                &worktree_request,
                request.ownership.unwrap_or_default(),
            )?;
            read_owned(&created.worktree_path)?
                .ok_or_else(|| anyhow::anyhow!("new worktree has no ownership record"))
        })
        .await
        .map_err(|err| anyhow::anyhow!("worktree fork task panicked: {err}"))?
    }

    async fn list_forks(&self, source_workspace: &Path) -> anyhow::Result<Vec<WorkspaceFork>> {
        let source = source_workspace.to_path_buf();
        tokio::task::spawn_blocking(move || {
            let root = repo_root(&source)?;
            let mut forks = Vec::new();
            for worktree in list_worktree_paths(&root)? {
                if worktree == root {
                    continue;
                }
                forks.push(match read_owned(&worktree)? {
                    Some(fork) => fork,
                    None => GitWorktreeForkProvider::fork_from_worktree(&root, &worktree),
                });
            }
            Ok(forks)
        })
        .await
        .map_err(|err| anyhow::anyhow!("worktree list task panicked: {err}"))?
    }

    async fn resume_fork(&self, id: &ForkId) -> anyhow::Result<WorkspaceFork> {
        let path = PathBuf::from(id);
        let id = id.clone();
        tokio::task::spawn_blocking(move || {
            if !path.is_dir() {
                // The workspace disappeared out-of-band; report it rather
                // than guessing at provenance.
                return Ok(WorkspaceFork {
                    id: id.clone(),
                    ownership: None,
                    provider_id: GIT_WORKTREE_FORK_PROVIDER_ID.to_string(),
                    source_workspace: PathBuf::new(),
                    workspace: path,
                    status: ForkStatus::Missing,
                    provenance: ForkProvenance::at(OffsetDateTime::now_utc()),
                    cleanup: Default::default(),
                    metadata: serde_json::json!({}),
                });
            }
            let root = source_root_for_worktree(&path)?;
            anyhow::ensure!(
                list_worktree_paths(&root)?
                    .iter()
                    .any(|registered| registered == &path),
                "{} is not a registered worktree of its repository",
                path.display()
            );
            read_owned(&path)?.ok_or_else(|| {
                anyhow::anyhow!(
                    "{} is an external worktree without Roder ownership; refusing to adopt",
                    path.display()
                )
            })
        })
        .await
        .map_err(|err| anyhow::anyhow!("worktree resume task panicked: {err}"))?
    }

    async fn remove_fork(
        &self,
        id: &ForkId,
        policy: RemoveForkPolicy,
    ) -> anyhow::Result<RemoveForkResult> {
        let path = PathBuf::from(id);
        anyhow::ensure!(
            policy.confirm_workspace == path,
            "removal is path-confirmed: confirm the exact fork workspace {}",
            path.display()
        );
        let id = id.clone();
        tokio::task::spawn_blocking(move || {
            let root = source_root_for_worktree(&path)?;
            remove_worktree_fork(&root, &path)?;
            Ok(RemoveForkResult {
                id,
                removed: true,
                workspace: path,
            })
        })
        .await
        .map_err(|err| anyhow::anyhow!("worktree removal task panicked: {err}"))?
    }
}
