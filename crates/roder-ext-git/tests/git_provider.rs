use std::path::Path;
use std::process::Command;

use roder_api::version_control::{
    VcsCapabilityState, VcsChangeArea, VcsChangedFileStatus, VcsError, VcsLineSwitchRequest,
    VcsListChangesRequest, VcsListFilesRequest, VcsOperation, VcsProvider,
    VcsReadChangedContentRequest, VcsRestoreRequest, VcsSelectionGranularity, VcsSelectionRequest,
    VcsSnapshotCreateRequest, VcsStatusRequest, VcsSyncOperation,
};
use roder_ext_git::{GIT_VCS_PROVIDER_ID, GitProvider};

#[tokio::test]
async fn git_provider_reports_full_branch_delta() {
    let workspace = temp_workspace("roder-git-provider");
    init_repo(&workspace);
    std::fs::write(workspace.join("committed.txt"), "base\n").unwrap();
    std::fs::write(workspace.join("dirty.txt"), "base\n").unwrap();
    run_git(&workspace, &["add", "committed.txt"]);
    run_git(&workspace, &["add", "dirty.txt"]);
    run_git(&workspace, &["commit", "-m", "base"]);
    run_git(&workspace, &["checkout", "-b", "feature"]);
    std::fs::write(workspace.join("committed.txt"), "base\nbranch\n").unwrap();
    run_git(&workspace, &["add", "committed.txt"]);
    run_git(&workspace, &["commit", "-m", "branch change"]);
    std::fs::write(workspace.join("staged.txt"), "staged\n").unwrap();
    run_git(&workspace, &["add", "staged.txt"]);
    std::fs::write(workspace.join("dirty.txt"), "base\ndirty\n").unwrap();
    std::fs::write(workspace.join("untracked.txt"), "untracked\n").unwrap();
    std::fs::write(workspace.join("untracked.jpg"), [0xff, 0xd8, 0xff, 0x00]).unwrap();

    let provider = GitProvider;
    let status = provider
        .status(VcsStatusRequest {
            workspace_root: workspace.clone(),
        })
        .await
        .expect("status");
    let files = provider
        .list_changes(VcsListChangesRequest {
            workspace_root: workspace.clone(),
        })
        .await
        .expect("list changes");

    assert_eq!(status.provider.id, GIT_VCS_PROVIDER_ID);
    assert_eq!(status.active_line.unwrap().name, "feature");
    assert_eq!(status.base.unwrap().ref_name.unwrap(), "master");
    assert_eq!(status.changed_file_count, 5);

    let paths = files
        .iter()
        .map(|file| file.path.to_string_lossy().to_string())
        .collect::<Vec<_>>();
    assert!(paths.contains(&"committed.txt".to_string()));
    assert!(paths.contains(&"staged.txt".to_string()));
    assert!(paths.contains(&"dirty.txt".to_string()));
    assert!(paths.contains(&"untracked.txt".to_string()));
    assert!(paths.contains(&"untracked.jpg".to_string()));

    let staged = files
        .iter()
        .find(|file| file.path == Path::new("staged.txt"))
        .unwrap();
    assert_eq!(staged.areas, vec![VcsChangeArea::Staged]);
    let dirty = files
        .iter()
        .find(|file| file.path == Path::new("dirty.txt"))
        .unwrap();
    assert_eq!(dirty.areas, vec![VcsChangeArea::Unstaged]);

    let binary = files
        .iter()
        .find(|file| file.path == Path::new("untracked.jpg"))
        .unwrap();
    assert_eq!(binary.status, VcsChangedFileStatus::Untracked);
    assert_eq!(binary.areas, vec![VcsChangeArea::Untracked]);
    assert!(binary.binary);
    assert_eq!(binary.additions, 0);

    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn git_provider_can_ignore_whitespace_when_reading_changed_content() {
    let workspace = temp_workspace("roder-git-provider-ignore-whitespace");
    init_repo(&workspace);
    std::fs::write(workspace.join("file.txt"), "first\nsecond\n").unwrap();
    run_git(&workspace, &["add", "file.txt"]);
    run_git(&workspace, &["commit", "-m", "base"]);
    std::fs::write(workspace.join("file.txt"), "first\n  second\n").unwrap();

    let provider = GitProvider;
    let normal_page = provider
        .read_changed_content(VcsReadChangedContentRequest {
            workspace_root: workspace.clone(),
            path: "file.txt".into(),
            offset: 0,
            limit: 20,
            area: None,
            ignore_whitespace: false,
        })
        .await
        .expect("read content");
    let ignored_page = provider
        .read_changed_content(VcsReadChangedContentRequest {
            workspace_root: workspace.clone(),
            path: "file.txt".into(),
            offset: 0,
            limit: 20,
            area: None,
            ignore_whitespace: true,
        })
        .await
        .expect("read content ignoring whitespace");

    assert!(normal_page.content.unwrap().contains("+  second"));
    assert_eq!(ignored_page.content, Some(String::new()));
    assert_eq!(ignored_page.total_lines, 0);

    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn git_provider_reads_staged_and_unstaged_changed_content_by_area() {
    let workspace = temp_workspace("roder-git-provider-read-areas");
    init_repo(&workspace);
    std::fs::write(workspace.join("file.txt"), "base\n").unwrap();
    run_git(&workspace, &["add", "file.txt"]);
    run_git(&workspace, &["commit", "-m", "base"]);
    std::fs::write(workspace.join("file.txt"), "base\nstaged\n").unwrap();
    run_git(&workspace, &["add", "file.txt"]);
    std::fs::write(workspace.join("file.txt"), "base\nstaged\nunstaged\n").unwrap();

    let provider = GitProvider;
    let staged_page = provider
        .read_changed_content(VcsReadChangedContentRequest {
            workspace_root: workspace.clone(),
            path: "file.txt".into(),
            offset: 0,
            limit: 20,
            area: Some(VcsChangeArea::Staged),
            ignore_whitespace: false,
        })
        .await
        .expect("read staged content");
    let unstaged_page = provider
        .read_changed_content(VcsReadChangedContentRequest {
            workspace_root: workspace.clone(),
            path: "file.txt".into(),
            offset: 0,
            limit: 20,
            area: Some(VcsChangeArea::Unstaged),
            ignore_whitespace: false,
        })
        .await
        .expect("read unstaged content");

    let staged_content = staged_page.content.unwrap();
    let unstaged_content = unstaged_page.content.unwrap();
    assert!(staged_content.contains("+staged"));
    assert!(!staged_content.contains("+unstaged"));
    assert!(unstaged_content.contains("+unstaged"));
    assert!(!unstaged_content.contains("+staged"));

    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn git_provider_reads_paged_changed_content_and_validates_paths() {
    let workspace = temp_workspace("roder-git-provider-read");
    init_repo(&workspace);
    std::fs::write(workspace.join("file.txt"), "base\n").unwrap();
    run_git(&workspace, &["add", "file.txt"]);
    run_git(&workspace, &["commit", "-m", "base"]);
    std::fs::write(workspace.join("file.txt"), "base\nchanged\n").unwrap();
    std::fs::write(workspace.join("untracked.jpg"), [0xff, 0xd8, 0xff, 0x00]).unwrap();

    let provider = GitProvider;
    let page = provider
        .read_changed_content(VcsReadChangedContentRequest {
            workspace_root: workspace.clone(),
            path: "file.txt".into(),
            offset: 0,
            limit: 2,
            area: None,
            ignore_whitespace: false,
        })
        .await
        .expect("read content");

    assert_eq!(page.offset, 0);
    assert_eq!(page.next_offset, Some(2));
    assert!(page.total_lines > 2);

    let binary_page = provider
        .read_changed_content(VcsReadChangedContentRequest {
            workspace_root: workspace.clone(),
            path: "untracked.jpg".into(),
            offset: 0,
            limit: 20,
            area: None,
            ignore_whitespace: false,
        })
        .await
        .expect("read binary content");
    assert!(binary_page.binary);
    assert!(
        binary_page
            .content
            .unwrap()
            .contains("Binary files /dev/null")
    );

    let invalid = provider
        .read_changed_content(VcsReadChangedContentRequest {
            workspace_root: workspace.clone(),
            path: "../outside.txt".into(),
            offset: 0,
            limit: 20,
            area: None,
            ignore_whitespace: false,
        })
        .await
        .expect_err("invalid path should fail");
    assert!(matches!(invalid, VcsError::PathInvalid { .. }));

    let pathspec_magic = provider
        .read_changed_content(VcsReadChangedContentRequest {
            workspace_root: workspace.clone(),
            path: ":/".into(),
            offset: 0,
            limit: 20,
            area: None,
            ignore_whitespace: false,
        })
        .await
        .expect_err("git pathspec magic should fail validation");
    assert!(matches!(pathspec_magic, VcsError::PathInvalid { .. }));

    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn git_provider_normalizes_windows_style_relative_paths() {
    let workspace = temp_workspace("roder-git-provider-windows-paths");
    init_repo(&workspace);
    std::fs::write(workspace.join("base.txt"), "base\n").unwrap();
    run_git(&workspace, &["add", "base.txt"]);
    run_git(&workspace, &["commit", "-m", "base"]);
    std::fs::create_dir_all(workspace.join("src")).unwrap();
    std::fs::write(workspace.join("src/file.txt"), "hello\n").unwrap();

    let provider = GitProvider;
    let page = provider
        .read_changed_content(VcsReadChangedContentRequest {
            workspace_root: workspace.clone(),
            path: "src\\file.txt".into(),
            offset: 0,
            limit: 20,
            area: None,
            ignore_whitespace: false,
        })
        .await
        .expect("read untracked content with windows separators");

    assert_eq!(page.path, Path::new("src\\file.txt"));
    assert!(page.content.unwrap().contains("+hello"));

    provider
        .select(VcsSelectionRequest {
            workspace_root: workspace.clone(),
            paths: vec!["src\\file.txt".into()],
            granularity: VcsSelectionGranularity::Path,
        })
        .await
        .expect("select path with windows separators");
    assert_eq!(
        git_output(&workspace, &["diff", "--cached", "--name-only"]).trim(),
        "src/file.txt"
    );

    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn git_provider_detects_git_and_ignores_non_git_workspace() {
    let workspace = temp_workspace("roder-git-provider-detect");
    std::fs::create_dir_all(&workspace).unwrap();
    let provider = GitProvider;

    assert!(provider.detect(&workspace).await.unwrap().is_none());

    init_repo(&workspace);
    let claim = provider
        .detect(&workspace)
        .await
        .unwrap()
        .expect("git claim");

    assert_eq!(claim.priority, 100);
    assert_eq!(claim.workspace.root, workspace.canonicalize().unwrap());

    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn git_provider_capabilities_mark_mutations_unsupported_until_workflow_unit() {
    let workspace = temp_workspace("roder-git-provider-capabilities");
    init_repo(&workspace);
    std::fs::write(workspace.join("file.txt"), "base\n").unwrap();
    run_git(&workspace, &["add", "file.txt"]);
    run_git(&workspace, &["commit", "-m", "base"]);

    let provider = GitProvider;
    let status = provider
        .status(VcsStatusRequest {
            workspace_root: workspace.clone(),
        })
        .await
        .expect("status");

    let snapshot = status
        .capabilities
        .capability_for(VcsOperation::SnapshotCreate)
        .unwrap();
    assert_eq!(snapshot.state, VcsCapabilityState::Supported);

    let sync = status
        .capabilities
        .capability_for(VcsOperation::SyncFetch)
        .unwrap();
    assert_eq!(sync.state, VcsCapabilityState::Unsupported);
    assert!(sync.reason.as_deref().unwrap().contains("no remotes"));

    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn git_provider_creates_path_scoped_snapshot() {
    let workspace = temp_workspace("roder-git-provider-snapshot");
    init_repo(&workspace);
    std::fs::write(workspace.join("included.txt"), "base\n").unwrap();
    std::fs::write(workspace.join("excluded.txt"), "base\n").unwrap();
    run_git(&workspace, &["add", "."]);
    run_git(&workspace, &["commit", "-m", "base"]);
    std::fs::write(workspace.join("included.txt"), "base\nincluded\n").unwrap();
    std::fs::write(workspace.join("excluded.txt"), "base\nexcluded\n").unwrap();

    let provider = GitProvider;
    let snapshot = provider
        .create_snapshot(VcsSnapshotCreateRequest {
            workspace_root: workspace.clone(),
            message: "include one path".to_string(),
            paths: vec!["included.txt".into()],
        })
        .await
        .expect("create snapshot");

    assert_eq!(snapshot.provider_id, GIT_VCS_PROVIDER_ID);
    assert!(
        git_output(&workspace, &["show", "--name-only", "--format=", "HEAD"])
            .contains("included.txt")
    );
    assert!(
        !git_output(&workspace, &["show", "--name-only", "--format=", "HEAD"])
            .contains("excluded.txt")
    );
    assert!(git_output(&workspace, &["status", "--short"]).contains("excluded.txt"));

    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn git_provider_reports_nothing_to_snapshot_on_clean_tree() {
    let workspace = temp_workspace("roder-git-provider-clean-snapshot");
    init_repo(&workspace);
    std::fs::write(workspace.join("file.txt"), "base\n").unwrap();
    run_git(&workspace, &["add", "file.txt"]);
    run_git(&workspace, &["commit", "-m", "base"]);

    let provider = GitProvider;
    let error = provider
        .create_snapshot(VcsSnapshotCreateRequest {
            workspace_root: workspace.clone(),
            message: "no changes".to_string(),
            paths: Vec::new(),
        })
        .await
        .expect_err("clean tree should not snapshot");

    assert!(matches!(error, VcsError::UnsupportedOperation { .. }));
    assert!(error.to_string().contains("nothing to snapshot"));

    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn git_provider_selects_only_requested_literal_paths() {
    let workspace = temp_workspace("roder-git-provider-select-literal");
    init_repo(&workspace);
    std::fs::write(workspace.join("included.txt"), "base\n").unwrap();
    std::fs::write(workspace.join("excluded.txt"), "base\n").unwrap();
    run_git(&workspace, &["add", "."]);
    run_git(&workspace, &["commit", "-m", "base"]);
    std::fs::write(workspace.join("included.txt"), "base\nincluded\n").unwrap();
    std::fs::write(workspace.join("excluded.txt"), "base\nexcluded\n").unwrap();

    let provider = GitProvider;
    provider
        .select(VcsSelectionRequest {
            workspace_root: workspace.clone(),
            paths: vec!["included.txt".into()],
            granularity: VcsSelectionGranularity::Path,
        })
        .await
        .expect("select literal path");

    let staged = git_output(&workspace, &["diff", "--cached", "--name-only"]);
    assert!(staged.contains("included.txt"));
    assert!(!staged.contains("excluded.txt"));

    let pathspec_magic = provider
        .select(VcsSelectionRequest {
            workspace_root: workspace.clone(),
            paths: vec![":/".into()],
            granularity: VcsSelectionGranularity::Path,
        })
        .await
        .expect_err("git pathspec magic should fail validation");
    assert!(matches!(pathspec_magic, VcsError::PathInvalid { .. }));

    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn git_provider_restores_paths_and_rejects_escaping_paths() {
    let workspace = temp_workspace("roder-git-provider-restore");
    init_repo(&workspace);
    std::fs::write(workspace.join("file.txt"), "base\n").unwrap();
    run_git(&workspace, &["add", "file.txt"]);
    run_git(&workspace, &["commit", "-m", "base"]);
    std::fs::write(workspace.join("file.txt"), "changed\n").unwrap();

    let provider = GitProvider;
    provider
        .restore(VcsRestoreRequest {
            workspace_root: workspace.clone(),
            paths: vec!["file.txt".into()],
        })
        .await
        .expect("restore path");
    assert_eq!(
        std::fs::read_to_string(workspace.join("file.txt")).unwrap(),
        "base\n"
    );

    let error = provider
        .restore(VcsRestoreRequest {
            workspace_root: workspace.clone(),
            paths: vec!["../outside.txt".into()],
        })
        .await
        .expect_err("escaping path should fail");
    assert!(matches!(error, VcsError::PathInvalid { .. }));

    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn git_provider_restores_untracked_paths() {
    let workspace = temp_workspace("roder-git-provider-restore-untracked");
    init_repo(&workspace);
    std::fs::write(workspace.join("file.txt"), "base\n").unwrap();
    run_git(&workspace, &["add", "file.txt"]);
    run_git(&workspace, &["commit", "-m", "base"]);
    std::fs::write(workspace.join("untracked.txt"), "scratch\n").unwrap();

    let provider = GitProvider;
    provider
        .restore(VcsRestoreRequest {
            workspace_root: workspace.clone(),
            paths: vec!["untracked.txt".into()],
        })
        .await
        .expect("restore untracked path");

    assert!(!workspace.join("untracked.txt").exists());

    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn git_provider_restores_untracked_directories() {
    let workspace = temp_workspace("roder-git-provider-restore-untracked-dir");
    init_repo(&workspace);
    std::fs::write(workspace.join("file.txt"), "base\n").unwrap();
    run_git(&workspace, &["add", "file.txt"]);
    run_git(&workspace, &["commit", "-m", "base"]);
    std::fs::create_dir_all(workspace.join("scratch")).unwrap();
    std::fs::write(workspace.join("scratch/nested.txt"), "scratch\n").unwrap();

    let provider = GitProvider;
    provider
        .restore(VcsRestoreRequest {
            workspace_root: workspace.clone(),
            paths: vec!["scratch".into()],
        })
        .await
        .expect("restore untracked directory");

    assert!(!workspace.join("scratch").exists());

    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn git_provider_marks_tracked_binary_content_as_binary() {
    let workspace = temp_workspace("roder-git-provider-tracked-binary");
    init_repo(&workspace);
    std::fs::write(workspace.join("image.bin"), [0, 1, 2, 3]).unwrap();
    run_git(&workspace, &["add", "image.bin"]);
    run_git(&workspace, &["commit", "-m", "base"]);
    std::fs::write(workspace.join("image.bin"), [0xff, 0xfe, 0xfd, 0xfc]).unwrap();

    let provider = GitProvider;
    let page = provider
        .read_changed_content(VcsReadChangedContentRequest {
            workspace_root: workspace.clone(),
            path: "image.bin".into(),
            offset: 0,
            limit: 20,
            area: None,
            ignore_whitespace: false,
        })
        .await
        .expect("read tracked binary content");

    assert!(page.binary);

    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn git_provider_lists_and_switches_lines_with_dirty_workspace_guard() {
    let workspace = temp_workspace("roder-git-provider-lines");
    init_repo(&workspace);
    std::fs::write(workspace.join("file.txt"), "base\n").unwrap();
    run_git(&workspace, &["add", "file.txt"]);
    run_git(&workspace, &["commit", "-m", "base"]);
    run_git(&workspace, &["checkout", "-b", "feature"]);
    run_git(&workspace, &["checkout", "master"]);

    let provider = GitProvider;
    let lines = provider.list_lines(workspace.clone()).await.expect("lines");
    assert!(lines.iter().any(|line| line.name == "feature"));

    std::fs::write(workspace.join("file.txt"), "dirty\n").unwrap();
    let dirty = provider
        .switch_line(VcsLineSwitchRequest {
            workspace_root: workspace.clone(),
            line_id: "feature".to_string(),
        })
        .await
        .expect_err("dirty workspace should block switch");
    assert!(matches!(dirty, VcsError::DirtyWorkspace { .. }));

    run_git(&workspace, &["restore", "file.txt"]);
    provider
        .switch_line(VcsLineSwitchRequest {
            workspace_root: workspace.clone(),
            line_id: "feature".to_string(),
        })
        .await
        .expect("switch branch");
    assert_eq!(
        git_output(&workspace, &["branch", "--show-current"]).trim(),
        "feature"
    );

    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn git_provider_select_rejects_hunk_granularity() {
    let workspace = temp_workspace("roder-git-provider-select");
    init_repo(&workspace);
    let provider = GitProvider;

    let error = provider
        .select(VcsSelectionRequest {
            workspace_root: workspace.clone(),
            paths: vec!["file.txt".into()],
            granularity: VcsSelectionGranularity::Hunk,
        })
        .await
        .expect_err("hunk selection should be unsupported");

    assert!(matches!(error, VcsError::UnsupportedOperation { .. }));

    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn git_provider_sync_rejects_missing_remote_with_capability_error() {
    let workspace = temp_workspace("roder-git-provider-sync");
    init_repo(&workspace);
    let provider = GitProvider;

    let error = provider
        .sync(roder_api::version_control::VcsSyncRequest {
            workspace_root: workspace.clone(),
            operation: VcsSyncOperation::Fetch,
        })
        .await
        .expect_err("missing remote should be unsupported");

    assert!(matches!(error, VcsError::UnsupportedOperation { .. }));

    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn git_provider_list_files_returns_tracked_untracked_and_scoped_paths() {
    let workspace = temp_workspace("roder-git-provider-list-files");
    init_repo(&workspace);
    std::fs::create_dir_all(workspace.join("src")).unwrap();
    std::fs::write(workspace.join(".gitignore"), "ignored.txt\n").unwrap();
    std::fs::write(workspace.join("tracked.txt"), "tracked\n").unwrap();
    std::fs::write(workspace.join("src/lib.rs"), "pub fn lib() {}\n").unwrap();
    std::fs::write(workspace.join("untracked.txt"), "untracked\n").unwrap();
    std::fs::write(workspace.join("ignored.txt"), "ignored\n").unwrap();
    run_git(
        &workspace,
        &["add", ".gitignore", "tracked.txt", "src/lib.rs"],
    );

    let provider = GitProvider;
    let listing = provider
        .list_files(VcsListFilesRequest {
            workspace_root: workspace.clone(),
        })
        .await
        .expect("list files");

    assert_eq!(listing.provider_id, GIT_VCS_PROVIDER_ID);
    let canonical = workspace.canonicalize().unwrap();
    let names = listing
        .files
        .iter()
        .map(|path| {
            assert!(path.is_absolute(), "expected absolute path, got {path:?}");
            path.strip_prefix(&canonical)
                .expect("file under workspace root")
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert!(names.contains(".gitignore"));
    assert!(names.contains("tracked.txt"));
    assert!(names.contains("src/lib.rs"));
    assert!(names.contains("untracked.txt"));
    assert!(!names.contains("ignored.txt"));

    // A subdirectory root only enumerates its own files (R4: subdir roots).
    let scoped = provider
        .list_files(VcsListFilesRequest {
            workspace_root: workspace.join("src"),
        })
        .await
        .expect("list files in subdir");
    let scoped_names = scoped
        .files
        .iter()
        .filter_map(|path| path.file_name())
        .map(|name| name.to_string_lossy().to_string())
        .collect::<Vec<_>>();
    assert_eq!(scoped_names, vec!["lib.rs".to_string()]);

    let _ = std::fs::remove_dir_all(workspace);
}

fn temp_workspace(prefix: &str) -> std::path::PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{nonce}"))
}

fn init_repo(workspace: &Path) {
    std::fs::create_dir_all(workspace).unwrap();
    run_git(workspace, &["init", "-b", "master"]);
    run_git(workspace, &["config", "user.email", "roder@example.com"]);
    run_git(workspace, &["config", "user.name", "Roder Test"]);
}

fn run_git(workspace: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(workspace)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_output(workspace: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(workspace)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}
