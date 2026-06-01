use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use roder_api::events::{ThreadId, TurnId};
use roder_api::tools::ToolCall;
use roder_api::version_control::{
    RegistryVcsProviderResolver, VcsBase, VcsChangedFile, VcsChangedFileStatus, VcsError,
    VcsListChangesRequest, VcsProvider, VcsProviderResolution, VcsProviderResolver,
    VcsResolveRequest,
};
use roder_api::workspace_changes::{
    WorkspaceChangeConfidence, WorkspaceChangeObservation, WorkspaceChangeSource,
    WorkspaceChangeStatus, WorkspaceObservedFile,
};
use time::OffsetDateTime;

pub(crate) struct WorkspaceChangeBaseline {
    root: PathBuf,
    provider: Arc<dyn VcsProvider>,
    provider_id: String,
    base: Option<VcsBase>,
    files: BTreeMap<String, FileFingerprint>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileFingerprint {
    old_path: Option<String>,
    status: WorkspaceChangeStatus,
    additions: u32,
    deletions: u32,
    binary: bool,
}

impl WorkspaceChangeBaseline {
    pub(crate) async fn capture_for_tool(
        call: &ToolCall,
        workspace: Option<&str>,
        resolver: RegistryVcsProviderResolver,
    ) -> Option<Self> {
        if !should_reconcile_tool(&call.name) {
            return None;
        }
        let workspace = PathBuf::from(workspace?);
        let resolution = resolver
            .resolve_provider(VcsResolveRequest {
                workspace_root: workspace,
                preferred_provider_id: None,
            })
            .await
            .ok()?;
        let VcsProviderResolution::Available { provider, claim } = resolution else {
            return None;
        };
        let change_set = provider
            .status_with_changes(VcsListChangesRequest {
                workspace_root: claim.workspace.root.clone(),
            })
            .await
            .ok()?;
        let base = change_set.status.base.clone();
        let files = fingerprints(change_set.files);
        Some(Self {
            root: claim.workspace.root,
            provider_id: provider.id(),
            provider,
            base,
            files,
        })
    }

    pub(crate) async fn observed_after(
        self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        call: &ToolCall,
    ) -> Option<WorkspaceChangeObservation> {
        let after = current_files(&self.provider, &self.root, self.base)
            .await
            .ok()?;
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
            id: format!("{}-workspace-observed", call.id),
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            tool_call_id: call.id.clone(),
            tool_name: call.name.clone(),
            source: WorkspaceChangeSource::VersionControlReconciled,
            provider_id: Some(self.provider_id),
            confidence: WorkspaceChangeConfidence::ObservedAfterTool,
            files,
            created_at: OffsetDateTime::now_utc(),
        })
    }
}

fn should_reconcile_tool(name: &str) -> bool {
    matches!(name, "shell" | "exec_command")
}

async fn current_files(
    provider: &Arc<dyn VcsProvider>,
    root: &PathBuf,
    base: Option<VcsBase>,
) -> Result<BTreeMap<String, FileFingerprint>, VcsError> {
    let files = provider
        .list_changes_against_base(
            VcsListChangesRequest {
                workspace_root: root.clone(),
            },
            base,
        )
        .await?;
    Ok(fingerprints(files))
}

fn fingerprints(files: Vec<VcsChangedFile>) -> BTreeMap<String, FileFingerprint> {
    files.into_iter().map(fingerprint).collect()
}

fn fingerprint(file: VcsChangedFile) -> (String, FileFingerprint) {
    (
        file.path.display().to_string(),
        FileFingerprint {
            old_path: file.old_path.map(|path| path.display().to_string()),
            status: match file.status {
                VcsChangedFileStatus::Modified | VcsChangedFileStatus::ProviderNative => {
                    WorkspaceChangeStatus::Modified
                }
                VcsChangedFileStatus::Added => WorkspaceChangeStatus::Added,
                VcsChangedFileStatus::Deleted => WorkspaceChangeStatus::Deleted,
                VcsChangedFileStatus::Renamed => WorkspaceChangeStatus::Renamed,
                VcsChangedFileStatus::Untracked => WorkspaceChangeStatus::Untracked,
            },
            additions: file.additions,
            deletions: file.deletions,
            binary: file.binary,
        },
    )
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

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use roder_api::tools::ToolCall;
    use roder_api::version_control::{
        VcsCapabilities, VcsChangedContentPage, VcsDetectionClaim, VcsProviderId,
        VcsReadChangedContentRequest, VcsStatus, VcsStatusRequest, VcsWorkspace,
    };
    use roder_api::workspace_changes::{WorkspaceChangeConfidence, WorkspaceChangeStatus};

    use super::*;

    #[tokio::test]
    async fn shell_reconciliation_reports_provider_neutral_changes_after_baseline() {
        let workspace = PathBuf::from("/workspace");
        let provider = Arc::new(FakeVcsProvider::new(workspace.clone()));
        provider.set_files(vec![
            changed_file("preexisting.txt", VcsChangedFileStatus::Untracked, 1),
            changed_file("tracked.txt", VcsChangedFileStatus::Modified, 0),
        ]);
        let resolver = RegistryVcsProviderResolver::new(vec![provider.clone()]);

        let baseline = WorkspaceChangeBaseline::capture_for_tool(
            &tool_call(),
            Some("/workspace"),
            resolver.clone(),
        )
        .await
        .unwrap();

        provider.set_files(vec![
            changed_file("new.txt", VcsChangedFileStatus::Untracked, 2),
            changed_file("preexisting.txt", VcsChangedFileStatus::Untracked, 1),
            changed_file("tracked.txt", VcsChangedFileStatus::Modified, 1),
        ]);

        let change = baseline
            .observed_after(&"thread-1".to_string(), &"turn-1".to_string(), &tool_call())
            .await
            .unwrap();

        assert_eq!(
            change.confidence,
            WorkspaceChangeConfidence::ObservedAfterTool
        );
        assert_eq!(
            change.source,
            WorkspaceChangeSource::VersionControlReconciled
        );
        assert_eq!(change.provider_id.as_deref(), Some("fake-vcs"));
        assert_eq!(change.files.len(), 2);
        assert_eq!(change.files[0].path, "new.txt");
        assert_eq!(change.files[0].status, WorkspaceChangeStatus::Untracked);
        assert_eq!(change.files[0].additions, 2);
        assert_eq!(change.files[1].path, "tracked.txt");
        assert_eq!(change.files[1].status, WorkspaceChangeStatus::Modified);
        assert_eq!(change.files[1].additions, 1);
    }

    #[tokio::test]
    async fn no_provider_workspace_skips_reconciliation() {
        let resolver = RegistryVcsProviderResolver::new(Vec::new());

        let baseline =
            WorkspaceChangeBaseline::capture_for_tool(&tool_call(), Some("/workspace"), resolver)
                .await;

        assert!(baseline.is_none());
    }

    fn tool_call() -> ToolCall {
        ToolCall {
            id: "tool-1".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({}),
            raw_arguments: "{}".to_string(),
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
        }
    }

    fn changed_file(path: &str, status: VcsChangedFileStatus, additions: u32) -> VcsChangedFile {
        VcsChangedFile {
            path: PathBuf::from(path),
            old_path: None,
            status,
            additions,
            deletions: 0,
            binary: false,
        }
    }

    struct FakeVcsProvider {
        root: PathBuf,
        files: Mutex<Vec<VcsChangedFile>>,
    }

    impl FakeVcsProvider {
        fn new(root: PathBuf) -> Self {
            Self {
                root,
                files: Mutex::new(Vec::new()),
            }
        }

        fn set_files(&self, files: Vec<VcsChangedFile>) {
            *self.files.lock().unwrap() = files;
        }
    }

    #[async_trait::async_trait]
    impl VcsProvider for FakeVcsProvider {
        fn id(&self) -> VcsProviderId {
            "fake-vcs".to_string()
        }

        fn display_name(&self) -> String {
            "Fake VCS".to_string()
        }

        async fn detect(
            &self,
            _workspace_root: &std::path::Path,
        ) -> Result<Option<VcsDetectionClaim>, VcsError> {
            Ok(Some(VcsDetectionClaim {
                workspace: VcsWorkspace {
                    root: self.root.clone(),
                    id: None,
                },
                priority: 1,
                metadata: serde_json::Value::Null,
            }))
        }

        async fn status(&self, request: VcsStatusRequest) -> Result<VcsStatus, VcsError> {
            Ok(VcsStatus {
                provider: roder_api::version_control::VcsProviderIdentity {
                    id: self.id(),
                    display_name: self.display_name(),
                },
                workspace: VcsWorkspace {
                    root: request.workspace_root,
                    id: None,
                },
                active_line: None,
                base: None,
                capabilities: VcsCapabilities::default(),
                changed_file_count: self.files.lock().unwrap().len() as u32,
            })
        }

        async fn list_changes(
            &self,
            _request: VcsListChangesRequest,
        ) -> Result<Vec<VcsChangedFile>, VcsError> {
            Ok(self.files.lock().unwrap().clone())
        }

        async fn read_changed_content(
            &self,
            request: VcsReadChangedContentRequest,
        ) -> Result<VcsChangedContentPage, VcsError> {
            Ok(VcsChangedContentPage {
                path: request.path,
                content: None,
                offset: request.offset,
                total_lines: 0,
                next_offset: None,
                binary: false,
            })
        }
    }
}
