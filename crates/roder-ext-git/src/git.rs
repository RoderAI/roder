use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use roder_api::version_control::{
    VcsBase, VcsCapabilities, VcsCapabilityState, VcsChangeArea, VcsChangedContentPage,
    VcsChangedFile, VcsChangedFileStatus, VcsError, VcsLineKind, VcsLineOfWork, VcsOperation,
    VcsOperationCapability, VcsOperationResult, VcsProviderId, VcsProviderIdentity,
    VcsReadChangedContentRequest, VcsRestoreRequest, VcsSelectionGranularity, VcsSelectionRequest,
    VcsSnapshot, VcsSnapshotCreateRequest, VcsStatus, VcsSyncOperation, VcsSyncRequest,
    VcsWorkspace,
};

#[derive(Clone)]
pub(crate) struct GitRepo {
    root: PathBuf,
    provider_id: VcsProviderId,
}

struct GitBase {
    name: String,
    merge_base: String,
}

const GIT_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

impl GitRepo {
    pub(crate) fn open(workspace: &Path, provider_id: VcsProviderId) -> Result<Self, VcsError> {
        let root = git_at(
            workspace,
            &["rev-parse", "--show-toplevel"],
            &provider_id,
            VcsOperation::Status,
        )?;
        Ok(Self {
            root: PathBuf::from(root.trim()),
            provider_id,
        })
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn status(&self) -> Result<VcsStatus, VcsError> {
        let base = self.base()?;
        let files = self.changed_files(&base.merge_base)?;
        self.status_from_base_and_files(base, &files)
    }

    pub(crate) fn status_with_changes(
        &self,
    ) -> Result<roder_api::version_control::VcsStatusWithChanges, VcsError> {
        let base = self.base()?;
        let files = self.changed_files(&base.merge_base)?;
        let status = self.status_from_base_and_files(base, &files)?;
        Ok(roder_api::version_control::VcsStatusWithChanges { status, files })
    }

    fn status_from_base_and_files(
        &self,
        base: GitBase,
        files: &[VcsChangedFile],
    ) -> Result<VcsStatus, VcsError> {
        Ok(VcsStatus {
            provider: VcsProviderIdentity {
                id: self.provider_id.clone(),
                display_name: "Git".to_string(),
            },
            workspace: VcsWorkspace {
                root: self.root.clone(),
                id: self.rev_parse("HEAD").ok(),
            },
            active_line: self.branch()?.map(|branch| VcsLineOfWork {
                id: branch.clone(),
                name: branch,
                kind: VcsLineKind::Branch,
            }),
            base: Some(VcsBase {
                ref_name: Some(base.name),
                sha: Some(base.merge_base),
            }),
            capabilities: self.capabilities(),
            changed_file_count: files.len() as u32,
        })
    }

    pub(crate) fn capabilities(&self) -> VcsCapabilities {
        git_capabilities(self.has_remote())
    }

    pub(crate) fn changed_files(&self, merge_base: &str) -> Result<Vec<VcsChangedFile>, VcsError> {
        let mut files = self.tracked_files(merge_base)?;
        for path in self.untracked_files()? {
            files.entry(path.clone()).or_insert_with(|| VcsChangedFile {
                path: PathBuf::from(path),
                old_path: None,
                status: VcsChangedFileStatus::Untracked,
                areas: vec![VcsChangeArea::Untracked],
                additions: 0,
                deletions: 0,
                binary: false,
            });
        }
        self.apply_change_areas(merge_base, &mut files)?;
        self.apply_numstat(merge_base, &mut files)?;
        self.apply_untracked_counts(&mut files);

        let mut files = files.into_values().collect::<Vec<_>>();
        files.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(files)
    }

    pub(crate) fn list_changes(&self) -> Result<Vec<VcsChangedFile>, VcsError> {
        let base = self.base()?;
        self.changed_files(&base.merge_base)
    }

    pub(crate) fn list_changes_against_base(
        &self,
        base_sha: &str,
    ) -> Result<Vec<VcsChangedFile>, VcsError> {
        self.changed_files(base_sha)
    }

    pub(crate) fn read_changed_content(
        &self,
        request: VcsReadChangedContentRequest,
    ) -> Result<VcsChangedContentPage, VcsError> {
        let path = normalized_relative_path(&self.provider_id, &request.path)?;
        let base = self.base()?;
        let untracked = self.untracked_files()?.contains(&path);
        let (content, binary) = match request.area {
            Some(VcsChangeArea::Untracked) => {
                if untracked {
                    self.untracked_patch(&path)?
                } else {
                    (String::new(), false)
                }
            }
            area => {
                if area.is_none() && untracked {
                    self.untracked_patch(&path)?
                } else {
                    self.tracked_changed_content(
                        &base.merge_base,
                        &path,
                        area,
                        request.ignore_whitespace,
                    )?
                }
            }
        };
        let lines = content.lines().collect::<Vec<_>>();
        let offset = request.offset.min(lines.len() as u32) as usize;
        let limit = request.limit.max(1) as usize;
        let end = offset.saturating_add(limit).min(lines.len());
        let mut page = lines[offset..end].join("\n");
        if !page.is_empty() {
            page.push('\n');
        }

        Ok(VcsChangedContentPage {
            path: request.path,
            content: Some(page),
            offset: offset as u32,
            total_lines: lines.len() as u32,
            next_offset: (end < lines.len()).then_some(end as u32),
            binary,
        })
    }

    pub(crate) fn select(
        &self,
        request: VcsSelectionRequest,
    ) -> Result<VcsOperationResult, VcsError> {
        if !matches!(request.granularity, VcsSelectionGranularity::Path) {
            return Err(VcsError::UnsupportedOperation {
                provider_id: self.provider_id.clone(),
                operation: VcsOperation::Selection,
                capability: Some(
                    VcsOperationCapability::new(
                        VcsOperation::Selection,
                        VcsCapabilityState::Partial,
                    )
                    .with_granularities(vec![VcsSelectionGranularity::Path]),
                ),
                message: "git provider supports path selection, not hunk selection".to_string(),
            });
        }
        let paths = self.validate_paths(&request.paths)?;
        self.git_with_paths(&["add", "--"], &paths, VcsOperation::Selection)?;
        Ok(VcsOperationResult {
            provider_id: self.provider_id.clone(),
            message: Some(format!("selected {} path(s)", request.paths.len())),
        })
    }

    pub(crate) fn create_snapshot(
        &self,
        request: VcsSnapshotCreateRequest,
    ) -> Result<VcsSnapshot, VcsError> {
        if request.message.trim().is_empty() {
            return Err(VcsError::UnsupportedOperation {
                provider_id: self.provider_id.clone(),
                operation: VcsOperation::SnapshotCreate,
                capability: None,
                message: "snapshot message is required".to_string(),
            });
        }
        let paths = self.validate_paths(&request.paths)?;
        if request.paths.is_empty() {
            self.git(&["add", "--all"], VcsOperation::SnapshotCreate)?;
            if !self.has_staged_changes()? {
                return Err(VcsError::UnsupportedOperation {
                    provider_id: self.provider_id.clone(),
                    operation: VcsOperation::SnapshotCreate,
                    capability: None,
                    message: "nothing to snapshot".to_string(),
                });
            }
            self.git(
                &[
                    "-c",
                    "core.hooksPath=/dev/null",
                    "commit",
                    "--no-verify",
                    "-m",
                    request.message.as_str(),
                ],
                VcsOperation::SnapshotCreate,
            )?;
        } else {
            self.git_with_paths(&["add", "--"], &paths, VcsOperation::SnapshotCreate)?;
            let mut args = vec![
                "--literal-pathspecs",
                "-c",
                "core.hooksPath=/dev/null",
                "commit",
                "--no-verify",
                "-m",
                request.message.as_str(),
                "--",
            ];
            args.extend(paths.iter().map(String::as_str));
            self.git(&args, VcsOperation::SnapshotCreate)?;
        }
        let id = self.rev_parse("HEAD")?;
        Ok(VcsSnapshot {
            provider_id: self.provider_id.clone(),
            id,
            label: Some(request.message),
        })
    }

    pub(crate) fn restore(
        &self,
        request: VcsRestoreRequest,
    ) -> Result<VcsOperationResult, VcsError> {
        let paths = self.validate_paths(&request.paths)?;
        if request.paths.is_empty() {
            return Err(VcsError::PathInvalid {
                provider_id: self.provider_id.clone(),
                path: PathBuf::new(),
                message: "at least one restore path is required".to_string(),
            });
        }
        let untracked = self.untracked_files()?;
        let (untracked_paths, tracked_paths): (Vec<_>, Vec<_>) = request
            .paths
            .iter()
            .zip(paths.iter())
            .partition(|(_, path)| is_untracked_path(&untracked, path));
        let untracked_paths = untracked_paths
            .into_iter()
            .map(|(_, path)| path.clone())
            .collect::<Vec<_>>();
        let tracked_paths = tracked_paths
            .into_iter()
            .map(|(_, path)| path.clone())
            .collect::<Vec<_>>();
        if !tracked_paths.is_empty() {
            self.git_with_paths(
                &["restore", "--staged", "--worktree", "--"],
                &tracked_paths,
                VcsOperation::Restore,
            )?;
        }
        if !untracked_paths.is_empty() {
            self.git_with_paths(
                &["clean", "-fd", "--"],
                &untracked_paths,
                VcsOperation::Restore,
            )?;
        }
        Ok(VcsOperationResult {
            provider_id: self.provider_id.clone(),
            message: Some(format!("restored {} path(s)", request.paths.len())),
        })
    }

    pub(crate) fn list_lines(&self) -> Result<Vec<VcsLineOfWork>, VcsError> {
        let output = self.git(
            &["branch", "--format=%(refname:short)"],
            VcsOperation::LineList,
        )?;
        Ok(output
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| VcsLineOfWork {
                id: line.trim().to_string(),
                name: line.trim().to_string(),
                kind: VcsLineKind::Branch,
            })
            .collect())
    }

    pub(crate) fn switch_line(&self, line_id: &str) -> Result<VcsOperationResult, VcsError> {
        if !self
            .git(&["status", "--porcelain"], VcsOperation::LineSwitch)?
            .trim()
            .is_empty()
        {
            return Err(VcsError::DirtyWorkspace {
                provider_id: self.provider_id.clone(),
                operation: VcsOperation::LineSwitch,
                message: "cannot switch git branches with a dirty workspace".to_string(),
            });
        }
        self.git(
            &["-c", "core.hooksPath=/dev/null", "switch", line_id],
            VcsOperation::LineSwitch,
        )?;
        Ok(VcsOperationResult {
            provider_id: self.provider_id.clone(),
            message: Some(format!("switched to {line_id}")),
        })
    }

    pub(crate) fn sync(&self, request: VcsSyncRequest) -> Result<VcsOperationResult, VcsError> {
        if !self.has_remote() {
            let operation = match request.operation {
                VcsSyncOperation::Fetch => VcsOperation::SyncFetch,
                VcsSyncOperation::Pull => VcsOperation::SyncPull,
                VcsSyncOperation::Push => VcsOperation::SyncPush,
            };
            return Err(VcsError::UnsupportedOperation {
                provider_id: self.provider_id.clone(),
                operation,
                capability: Some(
                    VcsOperationCapability::new(operation, VcsCapabilityState::Unsupported)
                        .with_reason("git repository has no remotes configured"),
                ),
                message: "git repository has no remotes configured".to_string(),
            });
        }
        let args = match request.operation {
            VcsSyncOperation::Fetch => vec!["fetch"],
            VcsSyncOperation::Pull => vec!["pull", "--ff-only"],
            VcsSyncOperation::Push => vec!["push"],
        };
        self.git(&args, sync_operation(request.operation))?;
        Ok(VcsOperationResult {
            provider_id: self.provider_id.clone(),
            message: Some("sync completed".to_string()),
        })
    }

    fn tracked_files(
        &self,
        merge_base: &str,
    ) -> Result<BTreeMap<String, VcsChangedFile>, VcsError> {
        let output = self.git(
            &["diff", "--name-status", merge_base, "--"],
            VcsOperation::ChangesList,
        )?;
        let mut files = BTreeMap::new();
        for line in output.lines().filter(|line| !line.trim().is_empty()) {
            let columns = line.split('\t').collect::<Vec<_>>();
            let status_text = columns.first().copied().unwrap_or_default();
            let status = match status_text.chars().next().unwrap_or('M') {
                'A' => VcsChangedFileStatus::Added,
                'D' => VcsChangedFileStatus::Deleted,
                'R' => VcsChangedFileStatus::Renamed,
                _ => VcsChangedFileStatus::Modified,
            };
            let (old_path, path) =
                if matches!(status, VcsChangedFileStatus::Renamed) && columns.len() >= 3 {
                    (Some(PathBuf::from(columns[1])), columns[2].to_string())
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
                VcsChangedFile {
                    path: PathBuf::from(path),
                    old_path,
                    status,
                    areas: Vec::new(),
                    additions: 0,
                    deletions: 0,
                    binary: false,
                },
            );
        }
        Ok(files)
    }

    fn apply_change_areas(
        &self,
        merge_base: &str,
        files: &mut BTreeMap<String, VcsChangedFile>,
    ) -> Result<(), VcsError> {
        for path in self.changed_paths(&["diff", "--name-only", merge_base, "HEAD", "--"])? {
            add_change_area(files, &path, VcsChangeArea::Committed);
        }
        for path in self.changed_paths(&["diff", "--cached", "--name-only", "HEAD", "--"])? {
            add_change_area(files, &path, VcsChangeArea::Staged);
        }
        for path in self.changed_paths(&["diff", "--name-only", "--"])? {
            add_change_area(files, &path, VcsChangeArea::Unstaged);
        }
        for file in files.values_mut() {
            file.areas.sort();
            file.areas.dedup();
        }
        Ok(())
    }

    fn changed_paths(&self, args: &[&str]) -> Result<Vec<String>, VcsError> {
        let output = self.git(args, VcsOperation::ChangesList)?;
        Ok(output
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(ToOwned::to_owned)
            .collect())
    }

    fn apply_numstat(
        &self,
        merge_base: &str,
        files: &mut BTreeMap<String, VcsChangedFile>,
    ) -> Result<(), VcsError> {
        let output = self.git(
            &["diff", "--numstat", merge_base, "--"],
            VcsOperation::ChangesList,
        )?;
        for line in output.lines().filter(|line| !line.trim().is_empty()) {
            let columns = line.split('\t').collect::<Vec<_>>();
            if columns.len() < 3 {
                continue;
            }
            let Some(file) = files.get_mut(columns[2]) else {
                continue;
            };
            file.binary = columns[0] == "-" || columns[1] == "-";
            file.additions = columns[0].parse().unwrap_or(0);
            file.deletions = columns[1].parse().unwrap_or(0);
        }
        Ok(())
    }

    fn untracked_files(&self) -> Result<BTreeSet<String>, VcsError> {
        let output = self.git(
            &["ls-files", "--others", "--exclude-standard"],
            VcsOperation::ChangesList,
        )?;
        Ok(output
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(ToOwned::to_owned)
            .collect())
    }

    fn apply_untracked_counts(&self, files: &mut BTreeMap<String, VcsChangedFile>) {
        for file in files
            .values_mut()
            .filter(|file| matches!(file.status, VcsChangedFileStatus::Untracked))
        {
            match untracked_file_text(&self.root.join(&file.path)) {
                Ok(Some(text)) => {
                    file.binary = false;
                    file.additions = text.lines().count() as u32;
                }
                Ok(None) => {
                    file.binary = true;
                    file.additions = 0;
                }
                Err(_) => file.additions = 0,
            }
        }
    }

    fn untracked_patch(&self, path: &str) -> Result<(String, bool), VcsError> {
        let full_path = self.root.join(path);
        let Some(text) =
            untracked_file_text(&full_path).map_err(|err| VcsError::CommandFailed {
                provider_id: self.provider_id.clone(),
                operation: VcsOperation::ChangesRead,
                command: format!("read {}", full_path.display()),
                exit_code: None,
                stderr: err.to_string(),
            })?
        else {
            return Ok((
                format!(
                    "diff --git a/{path} b/{path}\nnew file mode 100644\nBinary files /dev/null and b/{path} differ\n"
                ),
                true,
            ));
        };
        let mut patch = format!(
            "diff --git a/{path} b/{path}\nnew file mode 100644\n--- /dev/null\n+++ b/{path}\n"
        );
        patch.push_str(&format!("@@ -0,0 +1,{} @@\n", text.lines().count()));
        for line in text.lines() {
            patch.push('+');
            patch.push_str(line);
            patch.push('\n');
        }
        Ok((patch, false))
    }

    fn base(&self) -> Result<GitBase, VcsError> {
        for candidate in self.base_candidates() {
            if self.rev_parse(&candidate).is_err() {
                continue;
            }
            if let Ok(merge_base) =
                self.git(&["merge-base", "HEAD", &candidate], VcsOperation::Status)
            {
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

    fn base_candidates(&self) -> Vec<String> {
        let mut candidates = Vec::new();
        if let Ok(upstream) = self.git(
            &[
                "rev-parse",
                "--abbrev-ref",
                "--symbolic-full-name",
                "@{upstream}",
            ],
            VcsOperation::Status,
        ) {
            let upstream = upstream.trim();
            if !upstream.is_empty() {
                candidates.push(upstream.to_string());
            }
        }
        if let Ok(default_remote) = self.git(
            &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
            VcsOperation::Status,
        ) {
            let default_remote = default_remote.trim();
            if !default_remote.is_empty() {
                candidates.push(default_remote.to_string());
            }
        }
        candidates.extend(["origin/master", "origin/main", "master", "main"].map(str::to_string));
        candidates.dedup();
        candidates
    }

    fn branch(&self) -> Result<Option<String>, VcsError> {
        let branch = self.git(&["rev-parse", "--abbrev-ref", "HEAD"], VcsOperation::Status)?;
        let branch = branch.trim();
        Ok((!branch.is_empty() && branch != "HEAD").then(|| branch.to_string()))
    }

    fn rev_parse(&self, rev: &str) -> Result<String, VcsError> {
        Ok(self
            .git(&["rev-parse", rev], VcsOperation::Status)?
            .trim()
            .to_string())
    }

    fn git(&self, args: &[&str], operation: VcsOperation) -> Result<String, VcsError> {
        git_at(&self.root, args, &self.provider_id, operation)
    }

    fn git_with_paths(
        &self,
        base_args: &[&str],
        paths: &[String],
        operation: VcsOperation,
    ) -> Result<String, VcsError> {
        let mut args = vec!["--literal-pathspecs"];
        args.extend_from_slice(base_args);
        args.extend(paths.iter().map(String::as_str));
        self.git(&args, operation)
    }

    fn validate_paths(&self, paths: &[PathBuf]) -> Result<Vec<String>, VcsError> {
        paths
            .iter()
            .map(|path| normalized_relative_path(&self.provider_id, path))
            .collect()
    }

    fn has_remote(&self) -> bool {
        self.git(&["remote"], VcsOperation::Status)
            .map(|output| output.lines().any(|line| !line.trim().is_empty()))
            .unwrap_or(false)
    }

    fn tracked_changed_content(
        &self,
        merge_base: &str,
        path: &str,
        area: Option<VcsChangeArea>,
        ignore_whitespace: bool,
    ) -> Result<(String, bool), VcsError> {
        let mut args = vec![
            "--literal-pathspecs",
            "diff",
            "--no-ext-diff",
            "--unified=80",
        ];
        if ignore_whitespace {
            args.push("-w");
        }
        append_diff_area_args(&mut args, merge_base, area);
        args.extend(["--", path]);

        Ok((
            self.git(&args, VcsOperation::ChangesRead)?,
            self.tracked_path_is_binary_for_area(merge_base, path, area)?,
        ))
    }

    fn tracked_path_is_binary_for_area(
        &self,
        merge_base: &str,
        path: &str,
        area: Option<VcsChangeArea>,
    ) -> Result<bool, VcsError> {
        let mut args = vec!["--literal-pathspecs", "diff", "--numstat"];
        append_diff_area_args(&mut args, merge_base, area);
        args.extend(["--", path]);
        let output = self.git(&args, VcsOperation::ChangesRead)?;
        Ok(output.lines().any(|line| {
            let columns = line.split('\t').collect::<Vec<_>>();
            columns.len() >= 2 && columns[0] == "-" && columns[1] == "-"
        }))
    }

    fn has_staged_changes(&self) -> Result<bool, VcsError> {
        let output = self.git(
            &["diff", "--cached", "--quiet", "--"],
            VcsOperation::SnapshotCreate,
        );
        match output {
            Ok(_) => Ok(false),
            Err(VcsError::CommandFailed {
                exit_code: Some(1), ..
            }) => Ok(true),
            Err(error) => Err(error),
        }
    }
}

fn append_diff_area_args<'a>(
    args: &mut Vec<&'a str>,
    merge_base: &'a str,
    area: Option<VcsChangeArea>,
) {
    match area {
        Some(VcsChangeArea::Committed) => args.extend([merge_base, "HEAD"]),
        Some(VcsChangeArea::Staged) => args.extend(["--cached", "HEAD"]),
        Some(VcsChangeArea::Unstaged) => {}
        Some(VcsChangeArea::Untracked) => {}
        None => args.push(merge_base),
    }
}

fn git_capabilities(has_remote: bool) -> VcsCapabilities {
    let sync_state = if has_remote {
        VcsCapabilityState::Supported
    } else {
        VcsCapabilityState::Unsupported
    };
    let sync_reason = (!has_remote).then(|| "git repository has no remotes configured".to_string());
    VcsCapabilities {
        operations: vec![
            VcsOperationCapability::new(VcsOperation::Status, VcsCapabilityState::Supported),
            VcsOperationCapability::new(VcsOperation::ChangesList, VcsCapabilityState::Supported),
            VcsOperationCapability::new(VcsOperation::ChangesRead, VcsCapabilityState::Supported),
            VcsOperationCapability::new(VcsOperation::Selection, VcsCapabilityState::Partial)
                .with_granularities(vec![VcsSelectionGranularity::Path]),
            VcsOperationCapability::new(
                VcsOperation::SnapshotCreate,
                VcsCapabilityState::Supported,
            )
            .with_granularities(vec![VcsSelectionGranularity::Path]),
            VcsOperationCapability::new(VcsOperation::Restore, VcsCapabilityState::Supported)
                .with_granularities(vec![VcsSelectionGranularity::Path]),
            VcsOperationCapability::new(VcsOperation::LineList, VcsCapabilityState::Supported),
            VcsOperationCapability::new(VcsOperation::LineSwitch, VcsCapabilityState::Supported),
            sync_capability(VcsOperation::SyncFetch, sync_state, sync_reason.clone()),
            sync_capability(VcsOperation::SyncPull, sync_state, sync_reason.clone()),
            sync_capability(VcsOperation::SyncPush, sync_state, sync_reason),
        ],
    }
}

fn sync_capability(
    operation: VcsOperation,
    state: VcsCapabilityState,
    reason: Option<String>,
) -> VcsOperationCapability {
    let mut capability = VcsOperationCapability::new(operation, state);
    capability.reason = reason;
    capability
}

fn sync_operation(operation: VcsSyncOperation) -> VcsOperation {
    match operation {
        VcsSyncOperation::Fetch => VcsOperation::SyncFetch,
        VcsSyncOperation::Pull => VcsOperation::SyncPull,
        VcsSyncOperation::Push => VcsOperation::SyncPush,
    }
}

fn normalized_relative_path(provider_id: &str, path: &Path) -> Result<String, VcsError> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(VcsError::PathInvalid {
            provider_id: provider_id.to_string(),
            path: path.to_path_buf(),
            message: "path must be a relative repository path".to_string(),
        });
    }
    let normalized = path.to_string_lossy().replace('\\', "/");
    if normalized.starts_with(':') {
        return Err(VcsError::PathInvalid {
            provider_id: provider_id.to_string(),
            path: path.to_path_buf(),
            message: "git pathspec magic is not allowed".to_string(),
        });
    }
    if normalized
        .split('/')
        .any(|component| component.is_empty() || component == "." || component == "..")
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(VcsError::PathInvalid {
            provider_id: provider_id.to_string(),
            path: path.to_path_buf(),
            message: "path must stay inside the repository".to_string(),
        });
    }
    Ok(normalized)
}

fn is_untracked_path(untracked: &BTreeSet<String>, path: &str) -> bool {
    untracked.contains(path)
        || untracked
            .iter()
            .any(|candidate| candidate.starts_with(&format!("{path}/")))
}

fn add_change_area(files: &mut BTreeMap<String, VcsChangedFile>, path: &str, area: VcsChangeArea) {
    if let Some(file) = files.get_mut(path) {
        file.areas.push(area);
    }
}

fn git_at(
    cwd: &Path,
    args: &[&str],
    provider_id: &str,
    operation: VcsOperation,
) -> Result<String, VcsError> {
    let mut child = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|err| VcsError::CommandFailed {
            provider_id: provider_id.to_string(),
            operation,
            command: format!("git {}", args.join(" ")),
            exit_code: None,
            stderr: err.to_string(),
        })?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < GIT_COMMAND_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(VcsError::CommandFailed {
                    provider_id: provider_id.to_string(),
                    operation,
                    command: format!("git {}", args.join(" ")),
                    exit_code: None,
                    stderr: format!("timed out after {}s", GIT_COMMAND_TIMEOUT.as_secs()),
                });
            }
            Err(err) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(VcsError::CommandFailed {
                    provider_id: provider_id.to_string(),
                    operation,
                    command: format!("git {}", args.join(" ")),
                    exit_code: None,
                    stderr: err.to_string(),
                });
            }
        }
    }
    let output = child
        .wait_with_output()
        .map_err(|err| VcsError::CommandFailed {
            provider_id: provider_id.to_string(),
            operation,
            command: format!("git {}", args.join(" ")),
            exit_code: None,
            stderr: err.to_string(),
        })?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    Err(VcsError::CommandFailed {
        provider_id: provider_id.to_string(),
        operation,
        command: format!("git {}", args.join(" ")),
        exit_code: output.status.code(),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
    })
}

fn untracked_file_text(path: &Path) -> std::io::Result<Option<String>> {
    let bytes = std::fs::read(path)?;
    Ok(String::from_utf8(bytes).ok())
}
