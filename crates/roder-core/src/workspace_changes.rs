use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use roder_api::events::{ThreadId, TurnId};
use roder_api::tools::ToolCall;
use roder_api::workspace_changes::{
    WorkspaceChangeConfidence, WorkspaceChangeObservation, WorkspaceChangeSource,
    WorkspaceChangeStatus, WorkspaceObservedFile,
};
use time::OffsetDateTime;

pub(crate) struct WorkspaceChangeBaseline {
    root: PathBuf,
    files: BTreeMap<String, FileFingerprint>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileFingerprint {
    old_path: Option<String>,
    status: WorkspaceChangeStatus,
    additions: u32,
    deletions: u32,
    binary: bool,
    size: Option<u64>,
}

impl WorkspaceChangeBaseline {
    pub(crate) async fn capture_for_tool(call: &ToolCall, workspace: Option<&str>) -> Option<Self> {
        if !should_reconcile_tool(&call.name) {
            return None;
        }
        let workspace = workspace?.to_string();
        tokio::task::spawn_blocking(move || Self::capture_blocking(&workspace))
            .await
            .ok()
            .flatten()
    }

    fn capture_blocking(workspace: &str) -> Option<Self> {
        let root = git_at(Path::new(workspace), &["rev-parse", "--show-toplevel"]).ok()?;
        let root = PathBuf::from(root.trim());
        let files = current_files(&root).ok()?;
        Some(Self { root, files })
    }

    pub(crate) async fn observed_after(
        self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        call: &ToolCall,
    ) -> Option<WorkspaceChangeObservation> {
        let thread_id = thread_id.clone();
        let turn_id = turn_id.clone();
        let tool_call_id = call.id.clone();
        let tool_name = call.name.clone();
        tokio::task::spawn_blocking(move || {
            self.observed_after_blocking(&thread_id, &turn_id, &tool_call_id, &tool_name)
        })
        .await
        .ok()
        .flatten()
    }

    fn observed_after_blocking(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        tool_call_id: &str,
        tool_name: &str,
    ) -> Option<WorkspaceChangeObservation> {
        let after = current_files(&self.root).ok()?;
        let mut files = after
            .into_iter()
            .filter(|(path, fingerprint)| self.files.get(path) != Some(fingerprint))
            .map(|(path, fingerprint)| observed_file(path, fingerprint))
            .collect::<Vec<_>>();
        if files.is_empty() {
            return None;
        }
        files.sort_by(|left, right| left.path.cmp(&right.path));
        Some(WorkspaceChangeObservation {
            id: format!("{tool_call_id}-workspace-observed"),
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            tool_call_id: tool_call_id.to_string(),
            tool_name: tool_name.to_string(),
            source: WorkspaceChangeSource::GitReconciled,
            confidence: WorkspaceChangeConfidence::ObservedAfterTool,
            files,
            created_at: OffsetDateTime::now_utc(),
        })
    }
}

fn should_reconcile_tool(name: &str) -> bool {
    matches!(name, "shell" | "exec_command")
}

fn current_files(root: &Path) -> anyhow::Result<BTreeMap<String, FileFingerprint>> {
    let mut files = tracked_files(root)?;
    for path in untracked_files(root)? {
        files.entry(path.clone()).or_insert_with(|| {
            let full_path = root.join(&path);
            let (additions, binary) = untracked_counts(&full_path);
            let size = std::fs::metadata(&full_path)
                .ok()
                .map(|metadata| metadata.len());
            FileFingerprint {
                old_path: None,
                status: WorkspaceChangeStatus::Untracked,
                additions,
                deletions: 0,
                binary,
                size,
            }
        });
    }
    apply_numstat(root, &mut files)?;
    Ok(files)
}

fn tracked_files(root: &Path) -> anyhow::Result<BTreeMap<String, FileFingerprint>> {
    let output = git_at(root, &["diff", "--name-status", "HEAD", "--"])?;
    let mut files = BTreeMap::new();
    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        let columns = line.split('\t').collect::<Vec<_>>();
        let status_text = columns.first().copied().unwrap_or_default();
        let status = match status_text.chars().next().unwrap_or('M') {
            'A' => WorkspaceChangeStatus::Added,
            'D' => WorkspaceChangeStatus::Deleted,
            'R' => WorkspaceChangeStatus::Renamed,
            _ => WorkspaceChangeStatus::Modified,
        };
        let (old_path, path) =
            if matches!(status, WorkspaceChangeStatus::Renamed) && columns.len() >= 3 {
                (Some(columns[1].to_string()), columns[2].to_string())
            } else {
                (
                    None,
                    columns.get(1).copied().unwrap_or_default().to_string(),
                )
            };
        if path.is_empty() {
            continue;
        }
        files.insert(
            path,
            FileFingerprint {
                old_path,
                status,
                additions: 0,
                deletions: 0,
                binary: false,
                size: None,
            },
        );
    }
    Ok(files)
}

fn apply_numstat(root: &Path, files: &mut BTreeMap<String, FileFingerprint>) -> anyhow::Result<()> {
    let output = git_at(root, &["diff", "--numstat", "HEAD", "--"])?;
    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        let columns = line.split('\t').collect::<Vec<_>>();
        if columns.len() < 3 {
            continue;
        }
        let path = columns[2].to_string();
        let Some(file) = files.get_mut(&path) else {
            continue;
        };
        file.binary = columns[0] == "-" || columns[1] == "-";
        file.additions = columns[0].parse().unwrap_or(0);
        file.deletions = columns[1].parse().unwrap_or(0);
    }
    Ok(())
}

fn untracked_files(root: &Path) -> anyhow::Result<Vec<String>> {
    let output = git_at(root, &["ls-files", "--others", "--exclude-standard"])?;
    Ok(output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

fn untracked_counts(path: &Path) -> (u32, bool) {
    match std::fs::read_to_string(path) {
        Ok(text) => (text.lines().count() as u32, false),
        Err(_) => (0, true),
    }
}

fn observed_file(path: String, fingerprint: FileFingerprint) -> WorkspaceObservedFile {
    WorkspaceObservedFile {
        path,
        old_path: fingerprint.old_path,
        status: fingerprint.status,
        additions: fingerprint.additions,
        deletions: fingerprint.deletions,
        binary: fingerprint.binary,
    }
}

fn git_at(cwd: &Path, args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new("git").args(args).current_dir(cwd).output()?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    anyhow::bail!(
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr).trim()
    )
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use roder_api::tools::ToolCall;
    use roder_api::workspace_changes::{WorkspaceChangeConfidence, WorkspaceChangeStatus};

    use super::WorkspaceChangeBaseline;

    #[tokio::test]
    async fn shell_reconciliation_reports_only_changes_observed_after_baseline() {
        let workspace =
            std::env::temp_dir().join(format!("roder-workspace-change-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        run_git(&workspace, &["init", "-b", "master"]);
        run_git(&workspace, &["config", "user.email", "roder@example.com"]);
        run_git(&workspace, &["config", "user.name", "Roder Test"]);
        std::fs::write(workspace.join("tracked.txt"), "base\n").unwrap();
        run_git(&workspace, &["add", "tracked.txt"]);
        run_git(&workspace, &["commit", "-m", "base"]);
        std::fs::write(workspace.join("preexisting.txt"), "already dirty\n").unwrap();

        let baseline =
            WorkspaceChangeBaseline::capture_blocking(workspace.to_str().unwrap()).unwrap();

        std::fs::write(workspace.join("tracked.txt"), "base\nchanged\n").unwrap();
        std::fs::write(workspace.join("new.txt"), "new\nfile\n").unwrap();
        std::fs::write(workspace.join("preexisting.txt"), "already dirty\n").unwrap();

        let change = baseline
            .observed_after(
                &"thread-1".to_string(),
                &"turn-1".to_string(),
                &ToolCall {
                    id: "tool-1".to_string(),
                    name: "shell".to_string(),
                    arguments: serde_json::json!({}),
                    raw_arguments: "{}".to_string(),
                    thread_id: "thread-1".to_string(),
                    turn_id: "turn-1".to_string(),
                },
            )
            .await
            .unwrap();

        assert_eq!(
            change.confidence,
            WorkspaceChangeConfidence::ObservedAfterTool
        );
        assert_eq!(change.files.len(), 2);
        assert_eq!(change.files[0].path, "new.txt");
        assert_eq!(change.files[0].status, WorkspaceChangeStatus::Untracked);
        assert_eq!(change.files[0].additions, 2);
        assert_eq!(change.files[1].path, "tracked.txt");
        assert_eq!(change.files[1].status, WorkspaceChangeStatus::Modified);
        assert_eq!(change.files[1].additions, 1);

        let _ = std::fs::remove_dir_all(workspace);
    }

    fn run_git(workspace: &std::path::Path, args: &[&str]) {
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
}
