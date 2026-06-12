//! git fetcher: clones into a staging sibling and swaps into the store,
//! keeping `.git` so updates can reconcile the clone in place.

use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::Context;

use super::fsutil::{staging_sibling, swap_dir_into_place};
use super::npm::install_node_dependencies;

#[derive(Debug, Default)]
pub(crate) struct GitFetchOutcome {
    /// Commit SHA of the checked-out tree.
    pub resolved_commit: Option<String>,
    pub warnings: Vec<String>,
}

/// Fresh clone of `url` (optionally at `ref_name`) into `store_path`.
pub(crate) fn git_fetch_into_store(
    url: &str,
    ref_name: Option<&str>,
    store_path: &Path,
    npm_command: &[String],
    allow_scripts: bool,
) -> anyhow::Result<GitFetchOutcome> {
    let staging = staging_sibling(store_path, "git-stage");
    if let Some(parent) = staging.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create store directory {}", parent.display()))?;
    }
    let staging_str = staging.display().to_string();

    match ref_name {
        Some(ref_name) => {
            // `--branch` covers branches and tags in one shallow clone; a
            // commit SHA needs the full history before checkout.
            let shallow = run_git(
                &[
                    "clone",
                    "--depth",
                    "1",
                    "--branch",
                    ref_name,
                    url,
                    &staging_str,
                ],
                None,
            );
            if shallow.is_err() {
                let _ = fs::remove_dir_all(&staging);
                run_git(&["clone", url, &staging_str], None)
                    .with_context(|| format!("git clone {url} failed"))?;
                run_git(&["checkout", "--detach", ref_name], Some(&staging))
                    .with_context(|| format!("git checkout {ref_name} in clone of {url} failed"))?;
            }
        }
        None => {
            run_git(&["clone", "--depth", "1", url, &staging_str], None)
                .with_context(|| format!("git clone {url} failed"))?;
        }
    }

    let resolved_commit = run_git(&["rev-parse", "HEAD"], Some(&staging)).ok();
    let mut warnings = Vec::new();
    install_node_dependencies(&staging, npm_command, allow_scripts, &mut warnings)?;
    swap_dir_into_place(&staging, store_path)?;
    Ok(GitFetchOutcome {
        resolved_commit,
        warnings,
    })
}

/// Reconciles an existing clone: fetch, then move the working tree to the
/// pinned ref (or the remote default branch when unpinned) and clean
/// untracked files. Node dependencies are reinstalled afterwards because
/// `git clean -fdx` removes `node_modules`.
pub(crate) fn git_reconcile_existing(
    store_path: &Path,
    ref_name: Option<&str>,
    npm_command: &[String],
    allow_scripts: bool,
) -> anyhow::Result<GitFetchOutcome> {
    anyhow::ensure!(
        store_path.join(".git").exists(),
        "{} is not a git clone; reinstall the package",
        store_path.display()
    );
    if run_git(&["fetch", "--tags", "--force", "origin"], Some(store_path)).is_err() {
        run_git(&["fetch", "origin"], Some(store_path))
            .with_context(|| format!("git fetch in {} failed", store_path.display()))?;
    }

    match ref_name {
        Some(ref_name) => {
            let remote_branch = format!("refs/remotes/origin/{ref_name}");
            if run_git(&["rev-parse", "--verify", &remote_branch], Some(store_path)).is_ok() {
                // Branch pin: hard-reset to the remote tip.
                run_git(&["checkout", "--force", ref_name], Some(store_path))?;
                run_git(
                    &["reset", "--hard", &format!("origin/{ref_name}")],
                    Some(store_path),
                )?;
            } else if run_git(
                &["checkout", "--force", "--detach", ref_name],
                Some(store_path),
            )
            .is_err()
            {
                // Tag or SHA missing locally (shallow clone): fetch it
                // explicitly, then detach onto what arrived.
                run_git(&["fetch", "origin", ref_name], Some(store_path))
                    .with_context(|| format!("git fetch of ref {ref_name} failed"))?;
                run_git(
                    &["checkout", "--force", "--detach", "FETCH_HEAD"],
                    Some(store_path),
                )
                .with_context(|| format!("git checkout of ref {ref_name} failed"))?;
            }
        }
        None => {
            if let Some(branch) = remote_default_branch(store_path) {
                run_git(
                    &["reset", "--hard", &format!("origin/{branch}")],
                    Some(store_path),
                )?;
            } else {
                run_git(&["pull", "--ff-only"], Some(store_path))
                    .with_context(|| format!("git pull in {} failed", store_path.display()))?;
            }
        }
    }
    run_git(&["clean", "-fdx"], Some(store_path))?;

    let resolved_commit = run_git(&["rev-parse", "HEAD"], Some(store_path)).ok();
    let mut warnings = Vec::new();
    install_node_dependencies(store_path, npm_command, allow_scripts, &mut warnings)?;
    Ok(GitFetchOutcome {
        resolved_commit,
        warnings,
    })
}

/// Default remote branch, via the `origin/HEAD` symref with a
/// `git remote show origin` fallback.
fn remote_default_branch(clone: &Path) -> Option<String> {
    if let Ok(symref) = run_git(&["symbolic-ref", "refs/remotes/origin/HEAD"], Some(clone)) {
        return symref
            .strip_prefix("refs/remotes/origin/")
            .map(str::to_string);
    }
    let show = run_git(&["remote", "show", "origin"], Some(clone)).ok()?;
    show.lines().find_map(|line| {
        line.trim()
            .strip_prefix("HEAD branch:")
            .map(|branch| branch.trim().to_string())
            .filter(|branch| !branch.is_empty() && branch != "(unknown)")
    })
}

/// Runs git with prompts disabled, returning trimmed stdout.
fn run_git(args: &[&str], cwd: Option<&Path>) -> anyhow::Result<String> {
    let mut command = Command::new("git");
    command.env("GIT_TERMINAL_PROMPT", "0").args(args);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let output = command.output().map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            anyhow::anyhow!("git not found on PATH; install git to use git package sources")
        } else {
            anyhow::Error::from(err).context("run git")
        }
    })?;
    if !output.status.success() {
        anyhow::bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
