//! Native worktree fork helper tests (roadmap phase 90, Task 2). All tests
//! use temporary local Git repositories; no remotes are contacted.

use std::path::{Path, PathBuf};
use std::process::Command;

use roder_ext_git::{
    GitWorktreeForkRequest, create_worktree_fork, list_worktree_paths, remove_worktree_fork,
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

fn temp_repo(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "roder-worktree-fork-{label}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "--initial-branch", "main"]);
    git(&root, &["config", "user.email", "test@roder.dev"]);
    git(&root, &["config", "user.name", "Roder Test"]);
    std::fs::write(root.join("README.md"), "# fixture\n").unwrap();
    std::fs::write(root.join("src.rs"), "fn main() {}\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "initial"]);
    root.canonicalize().unwrap()
}

fn request(root: &Path, name: &str) -> GitWorktreeForkRequest {
    GitWorktreeForkRequest {
        source_workspace: root.to_path_buf(),
        fork_name: name.to_string(),
        base_dir: None,
    }
}

#[test]
fn two_forks_from_one_source_stay_isolated() {
    let root = temp_repo("isolated");

    let first = create_worktree_fork(&request(&root, "experiment")).unwrap();
    let second = create_worktree_fork(&request(&root, "experiment")).unwrap();

    assert_ne!(first.worktree_path, second.worktree_path);
    assert_ne!(first.branch, second.branch);
    assert_eq!(first.source_branch.as_deref(), Some("main"));
    assert!(!first.source_commit.is_empty());
    // Both worktrees carry the committed source files.
    assert!(first.worktree_path.join("README.md").exists());
    assert!(second.worktree_path.join("src.rs").exists());

    // Writes in one fork never appear in the other or the parent.
    std::fs::write(first.worktree_path.join("first-only.txt"), "a").unwrap();
    std::fs::write(second.worktree_path.join("second-only.txt"), "b").unwrap();
    assert!(!second.worktree_path.join("first-only.txt").exists());
    assert!(!first.worktree_path.join("second-only.txt").exists());
    assert!(!root.join("first-only.txt").exists());
    assert!(!root.join("second-only.txt").exists());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn dirty_source_fails_closed_with_actionable_message() {
    let root = temp_repo("dirty");

    // An untracked file counts as dirty: it would silently vanish from the
    // child worktree otherwise.
    std::fs::write(root.join("untracked-note.txt"), "wip").unwrap();
    let error = create_worktree_fork(&request(&root, "blocked"))
        .unwrap_err()
        .to_string();
    assert!(error.contains("uncommitted changes"), "{error}");
    assert!(error.contains("untracked-note.txt"), "{error}");

    // Tracked modifications are refused too.
    std::fs::remove_file(root.join("untracked-note.txt")).unwrap();
    std::fs::write(root.join("README.md"), "# changed\n").unwrap();
    let error = create_worktree_fork(&request(&root, "blocked"))
        .unwrap_err()
        .to_string();
    assert!(error.contains("uncommitted changes"), "{error}");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn detached_head_forks_keep_commit_provenance() {
    let root = temp_repo("detached");
    let head = String::from_utf8(
        Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string();
    git(&root, &["checkout", "--detach", "HEAD"]);

    let fork = create_worktree_fork(&request(&root, "from-detached")).unwrap();

    assert_eq!(fork.source_branch, None, "detached HEAD has no branch");
    assert_eq!(fork.source_commit, head);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn removal_only_deletes_registered_roder_worktrees() {
    let root = temp_repo("removal");
    let fork = create_worktree_fork(&request(&root, "short-lived")).unwrap();
    assert!(fork.worktree_path.exists());
    assert!(
        list_worktree_paths(&root)
            .unwrap()
            .contains(&fork.worktree_path)
    );

    // An arbitrary directory is never deleted, even when it sits under the
    // Roder base directory.
    let impostor = root.join(".roder/worktrees/impostor");
    std::fs::create_dir_all(&impostor).unwrap();
    let error = remove_worktree_fork(&root, &impostor)
        .unwrap_err()
        .to_string();
    assert!(error.contains("not a registered worktree"), "{error}");
    assert!(impostor.exists());

    // The primary worktree is always refused.
    let error = remove_worktree_fork(&root, &root).unwrap_err().to_string();
    assert!(error.contains("primary worktree"), "{error}");

    // The recorded fork removes cleanly and disappears from the registry.
    remove_worktree_fork(&root, &fork.worktree_path).unwrap();
    assert!(!fork.worktree_path.exists());
    assert!(
        !list_worktree_paths(&root)
            .unwrap()
            .contains(&fork.worktree_path)
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn unsafe_fork_names_are_rejected() {
    let root = temp_repo("names");
    for name in [
        "",
        "  ",
        "-leading-dash",
        ".hidden",
        "a/b",
        "a b",
        "x".repeat(65).as_str(),
    ] {
        let error = create_worktree_fork(&request(&root, name))
            .unwrap_err()
            .to_string();
        assert!(error.contains("fork name"), "{name:?}: {error}");
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn non_git_workspace_fails_with_sandbox_hint() {
    let plain = std::env::temp_dir().join(format!(
        "roder-worktree-fork-plain-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&plain).unwrap();

    let error = create_worktree_fork(&request(&plain, "nope"))
        .unwrap_err()
        .to_string();
    assert!(error.contains("not inside a Git repository"), "{error}");
    assert!(error.contains("sandbox fork"), "{error}");

    let _ = std::fs::remove_dir_all(&plain);
}
