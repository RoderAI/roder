//! Safe Git worktree creation and removal for native conversation forks
//! (roadmap phase 90).
//!
//! These helpers are deliberately conservative: forks refuse dirty sources
//! (including untracked files) so child worktrees never silently diverge
//! from what the user sees, worktree paths live under a Roder-owned base
//! directory, and removal only operates on paths that Git itself reports as
//! registered worktrees of the source repository.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail, ensure};

/// Default base directory (relative to the repository root) for fork worktrees.
pub const DEFAULT_WORKTREE_BASE: &str = ".roder/worktrees";

#[derive(Debug, Clone)]
pub struct GitWorktreeForkRequest {
    /// Any path inside the source repository (usually the thread workspace).
    pub source_workspace: PathBuf,
    /// User-facing fork name; sanitized into directory/branch names.
    pub fork_name: String,
    /// Base directory for the new worktree. Defaults to
    /// `<repo-root>/.roder/worktrees` when `None`.
    pub base_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitWorktreeFork {
    pub fork_id: String,
    /// Repository root the fork was created from.
    pub source_workspace: PathBuf,
    /// Absolute path of the new worktree.
    pub worktree_path: PathBuf,
    /// Branch created for the fork.
    pub branch: String,
    /// Source branch name; `None` when HEAD was detached.
    pub source_branch: Option<String>,
    /// Commit the worktree was created at.
    pub source_commit: String,
}

/// Creates a new Git worktree for a conversation fork.
///
/// Fails closed when the source repository has tracked or untracked changes,
/// when the fork name is unsafe, or when the resolved worktree path would
/// escape the configured base directory.
pub fn create_worktree_fork(request: &GitWorktreeForkRequest) -> anyhow::Result<GitWorktreeFork> {
    let root = repo_root(&request.source_workspace)?;
    let name = sanitize_fork_name(&request.fork_name)?;

    // Roder-owned state (including previously created fork worktrees under
    // `.roder/`) never counts as user dirtiness.
    let status = run_git(&root, &["status", "--porcelain"])?;
    let dirty: Vec<&str> = status
        .lines()
        .filter(|line| {
            let path = line.get(3..).unwrap_or_default();
            !path.starts_with(".roder/") && path != ".roder"
        })
        .collect();
    if !dirty.is_empty() {
        let summary: Vec<&str> = dirty.iter().take(5).copied().collect();
        bail!(
            "source workspace has uncommitted changes (tracked or untracked); commit or stash \
             them before forking so the child worktree matches what you see:\n{}",
            summary.join("\n")
        );
    }

    let base_dir = match &request.base_dir {
        Some(base) => base.clone(),
        None => root.join(DEFAULT_WORKTREE_BASE),
    };
    std::fs::create_dir_all(&base_dir)
        .with_context(|| format!("create worktree base {}", base_dir.display()))?;
    let base_dir = base_dir
        .canonicalize()
        .with_context(|| format!("canonicalize worktree base {}", base_dir.display()))?;

    // Deterministic, collision-resistant directory naming: `<name>`, then
    // `<name>-2`, `<name>-3`, ...
    let mut dir_name = name.clone();
    let mut suffix = 1u32;
    while base_dir.join(&dir_name).exists() {
        suffix += 1;
        ensure!(suffix <= 1000, "too many existing forks named {name}");
        dir_name = format!("{name}-{suffix}");
    }
    let worktree_path = base_dir.join(&dir_name);
    ensure!(
        worktree_path.starts_with(&base_dir),
        "fork worktree path {} escapes the configured base {}",
        worktree_path.display(),
        base_dir.display()
    );

    let source_commit = run_git(&root, &["rev-parse", "HEAD"])?.trim().to_string();
    let source_branch = run_git(&root, &["symbolic-ref", "--short", "-q", "HEAD"])
        .ok()
        .map(|branch| branch.trim().to_string())
        .filter(|branch| !branch.is_empty());

    let branch = unique_fork_branch(&root, &dir_name)?;
    let worktree_str = worktree_path.display().to_string();
    run_git(
        &root,
        &["worktree", "add", "-b", &branch, &worktree_str, "HEAD"],
    )
    .with_context(|| format!("create git worktree at {worktree_str}"))?;

    Ok(GitWorktreeFork {
        fork_id: format!("fork-{dir_name}"),
        source_workspace: root,
        worktree_path,
        branch,
        source_branch,
        source_commit,
    })
}

/// Removes a Roder-owned fork worktree.
///
/// The path must be a worktree that Git reports as registered for the source
/// repository; arbitrary directories are never deleted. After removal the
/// repository's worktree metadata is pruned. The fork branch is kept for
/// provenance.
pub fn remove_worktree_fork(source_workspace: &Path, worktree_path: &Path) -> anyhow::Result<()> {
    let root = repo_root(source_workspace)?;
    let registered = list_worktree_paths(&root)?;
    let target = worktree_path
        .canonicalize()
        .with_context(|| format!("fork worktree {} does not exist", worktree_path.display()))?;
    ensure!(
        target != root,
        "refusing to remove the primary worktree {}",
        root.display()
    );
    ensure!(
        registered.iter().any(|path| path == &target),
        "{} is not a registered worktree of {}; refusing to remove",
        target.display(),
        root.display()
    );

    let target_str = target.display().to_string();
    run_git(&root, &["worktree", "remove", &target_str])
        .with_context(|| format!("remove git worktree {target_str}"))?;
    run_git(&root, &["worktree", "prune"])?;
    Ok(())
}

/// Lists the absolute paths of all worktrees registered for the repository.
pub fn list_worktree_paths(repo_root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let output = run_git(repo_root, &["worktree", "list", "--porcelain"])?;
    Ok(output
        .lines()
        .filter_map(|line| line.strip_prefix("worktree "))
        .filter_map(|path| PathBuf::from(path).canonicalize().ok())
        .collect())
}

fn repo_root(workspace: &Path) -> anyhow::Result<PathBuf> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .context("run git rev-parse --show-toplevel")?;
    if !output.status.success() {
        bail!(
            "{} is not inside a Git repository; native worktree forks require Git \
             (a future sandbox fork provider will support non-Git workspaces)",
            workspace.display()
        );
    }
    let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
    PathBuf::from(&root)
        .canonicalize()
        .with_context(|| format!("canonicalize repo root {root}"))
}

fn sanitize_fork_name(name: &str) -> anyhow::Result<String> {
    let name = name.trim();
    ensure!(!name.is_empty(), "fork name is required");
    ensure!(
        name.len() <= 64,
        "fork name is too long (max 64 characters)"
    );
    ensure!(
        name.chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.')),
        "fork name {name:?} may only contain letters, digits, '-', '_', and '.'"
    );
    ensure!(
        !name.starts_with(['-', '.']),
        "fork name {name:?} cannot start with '-' or '.'"
    );
    Ok(name.to_string())
}

fn unique_fork_branch(root: &Path, dir_name: &str) -> anyhow::Result<String> {
    let mut branch = format!("roder/fork/{dir_name}");
    let mut suffix = 1u32;
    while branch_exists(root, &branch)? {
        suffix += 1;
        ensure!(suffix <= 1000, "too many existing fork branches for {dir_name}");
        branch = format!("roder/fork/{dir_name}-b{suffix}");
    }
    Ok(branch)
}

fn branch_exists(root: &Path, branch: &str) -> anyhow::Result<bool> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--verify", "-q", &format!("refs/heads/{branch}")])
        .output()
        .context("run git rev-parse --verify")?;
    Ok(output.status.success())
}

fn run_git(root: &Path, args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .with_context(|| format!("run git {}", args.join(" ")))?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
