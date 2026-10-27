//! Ownership lives in Git's worktree administration directory, not in a
//! user-editable checkout file or a branch-name heuristic.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use anyhow::{Context, ensure};
use roder_api::forks::{ForkOwnership, ForkProvenance, ForkStatus, WorkspaceFork};
use time::OffsetDateTime;

use crate::fork_provider::GIT_WORKTREE_FORK_PROVIDER_ID;
use crate::worktree::{GitWorktreeFork, run_git};

fn git_path(workspace: &Path, flag: &str) -> anyhow::Result<PathBuf> {
    let path = run_git(workspace, &["rev-parse", "--path-format=absolute", flag])?;
    Ok(PathBuf::from(path.trim()))
}

/// Serializes creation/removal and same-owner retries across Roder processes.
/// OS advisory locks are released on crash; no stale lockfile recovery needed.
pub(crate) fn lock_repository(workspace: &Path) -> anyhow::Result<File> {
    let path = git_path(workspace, "--git-common-dir")?.join("roder-forks.lock");
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    file.lock().context("lock Git fork ownership")?;
    Ok(file)
}

fn record_path(workspace: &Path) -> anyhow::Result<PathBuf> {
    Ok(git_path(workspace, "--git-dir")?.join("roder-fork.json"))
}

pub(crate) fn read_owned(workspace: &Path) -> anyhow::Result<Option<WorkspaceFork>> {
    let path = record_path(workspace)?;
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let fork: WorkspaceFork =
        serde_json::from_slice(&bytes).context("invalid Git fork ownership record")?;
    ensure!(
        fork.ownership.is_some()
            && fork.provider_id == GIT_WORKTREE_FORK_PROVIDER_ID
            && fork.workspace == workspace.canonicalize()?
            && fork.id == fork.workspace.to_string_lossy().as_ref(),
        "Git fork ownership record does not match {}",
        workspace.display()
    );
    Ok(Some(fork))
}

pub(crate) fn write_owned(fork: &WorkspaceFork) -> anyhow::Result<()> {
    let path = record_path(&fork.workspace)?;
    let temporary = path.with_extension("json.tmp");
    let mut file = File::create(&temporary)?;
    use std::io::Write;
    file.write_all(&serde_json::to_vec(fork)?)?;
    file.sync_all()?;
    std::fs::rename(&temporary, &path)?;
    #[cfg(unix)]
    File::open(path.parent().expect("Git metadata has a parent"))?.sync_all()?;
    Ok(())
}

pub(crate) fn record_created(
    created: &GitWorktreeFork,
    ownership: ForkOwnership,
) -> anyhow::Result<()> {
    write_owned(&WorkspaceFork {
        id: created.worktree_path.display().to_string(),
        ownership: Some(ownership),
        provider_id: GIT_WORKTREE_FORK_PROVIDER_ID.to_string(),
        source_workspace: created.source_workspace.clone(),
        workspace: created.worktree_path.clone(),
        status: ForkStatus::Active,
        provenance: ForkProvenance {
            branch: Some(created.branch.clone()),
            source_branch: created.source_branch.clone(),
            source_commit: Some(created.source_commit.clone()),
            snapshot_id: None,
            session_id: None,
            created_at: OffsetDateTime::now_utc(),
        },
        cleanup: Default::default(),
        metadata: serde_json::json!({"forkName": created.fork_id}),
    })
}
