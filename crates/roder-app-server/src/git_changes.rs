use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use roder_protocol::{
    GitChangeStatus, GitChangedFile, GitChangesListParams, GitChangesListResult,
    GitChangesReadParams, GitChangesReadResult, GitChangesTotals, JsonRpcError,
};

const DEFAULT_PATCH_LIMIT: usize = 400;
const DEFAULT_LIST_LIMIT: usize = 500;

pub(crate) fn list_changes(
    runtime_workspace: Option<String>,
    params: GitChangesListParams,
) -> Result<GitChangesListResult, JsonRpcError> {
    let workspace = workspace_path(params.workspace, runtime_workspace)?;
    let repo = GitRepo::open(&workspace)?;
    let base = repo.base()?;
    let mut files = tracked_files(&repo, &base.merge_base)?;
    for path in untracked_files(&repo)? {
        files.entry(path.clone()).or_insert_with(|| GitChangedFile {
            path,
            old_path: None,
            status: GitChangeStatus::Untracked,
            additions: 0,
            deletions: 0,
            binary: false,
        });
    }
    apply_numstat(&repo, &base.merge_base, &mut files)?;
    apply_untracked_counts(&repo, &mut files)?;

    let mut files = files.into_values().collect::<Vec<_>>();
    files.sort_by(|left, right| left.path.cmp(&right.path));
    let total_files = files.len();
    let totals = GitChangesTotals {
        files: total_files as u32,
        additions: files.iter().map(|file| file.additions).sum(),
        deletions: files.iter().map(|file| file.deletions).sum(),
    };
    let limit = params.limit.unwrap_or(DEFAULT_LIST_LIMIT).max(1);
    let truncated = files.len() > limit;
    files.truncate(limit);

    Ok(GitChangesListResult {
        repository_root: repo.root.display().to_string(),
        branch: repo.branch()?,
        base_ref: Some(base.name),
        base_sha: Some(base.merge_base),
        head_sha: repo.rev_parse("HEAD").ok(),
        files,
        totals,
        truncated,
    })
}

pub(crate) fn read_change(
    runtime_workspace: Option<String>,
    params: GitChangesReadParams,
) -> Result<GitChangesReadResult, JsonRpcError> {
    let workspace = workspace_path(params.workspace, runtime_workspace)?;
    validate_relative_path(&params.path)?;
    let repo = GitRepo::open(&workspace)?;
    let base = repo.base()?;
    let untracked = untracked_files(&repo)?.contains(&params.path);
    let patch = if untracked {
        untracked_patch(&repo, &params.path)?
    } else {
        repo.git([
            "diff",
            "--no-ext-diff",
            "--unified=80",
            &base.merge_base,
            "--",
            &params.path,
        ])?
    };
    let lines = patch.lines().collect::<Vec<_>>();
    let offset = params.offset.min(lines.len());
    let limit = params.limit.unwrap_or(DEFAULT_PATCH_LIMIT).max(1);
    let end = offset.saturating_add(limit).min(lines.len());
    let mut page = lines[offset..end].join("\n");
    if !page.is_empty() {
        page.push('\n');
    }

    Ok(GitChangesReadResult {
        path: params.path,
        patch: page,
        offset,
        limit,
        total_lines: lines.len(),
        next_offset: (end < lines.len()).then_some(end),
    })
}

fn workspace_path(
    requested: Option<String>,
    runtime_workspace: Option<String>,
) -> Result<PathBuf, JsonRpcError> {
    let path = requested
        .or(runtime_workspace)
        .ok_or_else(|| invalid_params("workspace is required"))?;
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return Err(invalid_params("workspace must be absolute"));
    }
    Ok(path)
}

fn validate_relative_path(path: &str) -> Result<(), JsonRpcError> {
    let path = Path::new(path);
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(invalid_params("path must be a relative repository path"));
    }
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(invalid_params("path must stay inside the repository"));
    }
    Ok(())
}

fn tracked_files(
    repo: &GitRepo,
    merge_base: &str,
) -> Result<BTreeMap<String, GitChangedFile>, JsonRpcError> {
    let output = repo.git(["diff", "--name-status", merge_base, "--"])?;
    let mut files = BTreeMap::new();
    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        let columns = line.split('\t').collect::<Vec<_>>();
        let status_text = columns.first().copied().unwrap_or_default();
        let status = match status_text.chars().next().unwrap_or('M') {
            'A' => GitChangeStatus::Added,
            'D' => GitChangeStatus::Deleted,
            'R' => GitChangeStatus::Renamed,
            _ => GitChangeStatus::Modified,
        };
        let (old_path, path) = if matches!(status, GitChangeStatus::Renamed) && columns.len() >= 3 {
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
            path.clone(),
            GitChangedFile {
                path,
                old_path,
                status,
                additions: 0,
                deletions: 0,
                binary: false,
            },
        );
    }
    Ok(files)
}

fn apply_numstat(
    repo: &GitRepo,
    merge_base: &str,
    files: &mut BTreeMap<String, GitChangedFile>,
) -> Result<(), JsonRpcError> {
    let output = repo.git(["diff", "--numstat", merge_base, "--"])?;
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

fn untracked_files(repo: &GitRepo) -> Result<BTreeSet<String>, JsonRpcError> {
    let output = repo.git(["ls-files", "--others", "--exclude-standard"])?;
    Ok(output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

fn apply_untracked_counts(
    repo: &GitRepo,
    files: &mut BTreeMap<String, GitChangedFile>,
) -> Result<(), JsonRpcError> {
    for file in files
        .values_mut()
        .filter(|file| matches!(file.status, GitChangeStatus::Untracked))
    {
        let path = repo.root.join(&file.path);
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        file.additions = text.lines().count() as u32;
    }
    Ok(())
}

fn untracked_patch(repo: &GitRepo, path: &str) -> Result<String, JsonRpcError> {
    let full_path = repo.root.join(path);
    let text = std::fs::read_to_string(&full_path).map_err(internal_error)?;
    let mut patch = format!(
        "diff --git a/{path} b/{path}\nnew file mode 100644\n--- /dev/null\n+++ b/{path}\n"
    );
    patch.push_str(&format!("@@ -0,0 +1,{} @@\n", text.lines().count()));
    for line in text.lines() {
        patch.push('+');
        patch.push_str(line);
        patch.push('\n');
    }
    Ok(patch)
}

struct GitRepo {
    root: PathBuf,
}

struct GitBase {
    name: String,
    merge_base: String,
}

impl GitRepo {
    fn open(workspace: &Path) -> Result<Self, JsonRpcError> {
        let root = git_at(workspace, ["rev-parse", "--show-toplevel"])?;
        Ok(Self {
            root: PathBuf::from(root.trim()),
        })
    }

    fn git<const N: usize>(&self, args: [&str; N]) -> Result<String, JsonRpcError> {
        git_at(&self.root, args)
    }

    fn branch(&self) -> Result<Option<String>, JsonRpcError> {
        let branch = self.git(["rev-parse", "--abbrev-ref", "HEAD"])?;
        let branch = branch.trim();
        Ok((!branch.is_empty() && branch != "HEAD").then(|| branch.to_string()))
    }

    fn rev_parse(&self, rev: &str) -> Result<String, JsonRpcError> {
        Ok(self.git(["rev-parse", rev])?.trim().to_string())
    }

    fn base(&self) -> Result<GitBase, JsonRpcError> {
        for candidate in self.base_candidates()? {
            if self.rev_parse(&candidate).is_err() {
                continue;
            }
            if let Ok(merge_base) = self.git(["merge-base", "HEAD", &candidate]) {
                return Ok(GitBase {
                    name: candidate,
                    merge_base: merge_base.trim().to_string(),
                });
            }
        }
        let head = self.rev_parse("HEAD")?;
        Ok(GitBase {
            name: "HEAD".to_string(),
            merge_base: head,
        })
    }

    fn base_candidates(&self) -> Result<Vec<String>, JsonRpcError> {
        let mut candidates = Vec::new();
        if let Ok(upstream) = self.git([
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ]) {
            let upstream = upstream.trim();
            if !upstream.is_empty() {
                candidates.push(upstream.to_string());
            }
        }
        if let Ok(default_remote) =
            self.git(["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
        {
            let default_remote = default_remote.trim();
            if !default_remote.is_empty() {
                candidates.push(default_remote.to_string());
            }
        }
        candidates.extend(
            ["origin/master", "origin/main", "master", "main"]
                .into_iter()
                .map(ToOwned::to_owned),
        );
        candidates.dedup();
        Ok(candidates)
    }
}

fn git_at<const N: usize>(cwd: &Path, args: [&str; N]) -> Result<String, JsonRpcError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(internal_error)?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    Err(internal_error(format!(
        "git failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    )))
}

fn invalid_params(message: impl Into<String>) -> JsonRpcError {
    JsonRpcError {
        code: -32602,
        message: message.into(),
        data: None,
    }
}

fn internal_error(err: impl std::fmt::Display) -> JsonRpcError {
    JsonRpcError {
        code: -32000,
        message: err.to_string(),
        data: None,
    }
}
