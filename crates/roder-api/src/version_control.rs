use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

pub type VcsProviderId = String;
pub type VcsWorkspaceId = String;
pub type VcsLineId = String;
pub type VcsSnapshotId = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VcsProviderIdentity {
    pub id: VcsProviderId,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VcsWorkspace {
    pub root: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<VcsWorkspaceId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VcsLineOfWork {
    pub id: VcsLineId,
    pub name: String,
    pub kind: VcsLineKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VcsLineKind {
    Branch,
    Bookmark,
    WorkingCopy,
    Revision,
    ProviderNative,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VcsBase {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ref_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VcsStatus {
    pub provider: VcsProviderIdentity,
    pub workspace: VcsWorkspace,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_line: Option<VcsLineOfWork>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<VcsBase>,
    pub capabilities: VcsCapabilities,
    #[serde(default)]
    pub changed_file_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VcsStatusWithChanges {
    pub status: VcsStatus,
    pub files: Vec<VcsChangedFile>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VcsCapabilities {
    #[serde(default)]
    pub operations: Vec<VcsOperationCapability>,
}

impl VcsCapabilities {
    pub fn capability_for(&self, operation: VcsOperation) -> Option<&VcsOperationCapability> {
        self.operations
            .iter()
            .find(|capability| capability.operation == operation)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VcsOperationCapability {
    pub operation: VcsOperation,
    pub state: VcsCapabilityState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_namespace: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub granularities: Vec<VcsSelectionGranularity>,
}

impl VcsOperationCapability {
    pub fn new(operation: VcsOperation, state: VcsCapabilityState) -> Self {
        Self {
            operation,
            state,
            reason: None,
            provider_namespace: None,
            granularities: Vec::new(),
        }
    }

    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    pub fn with_provider_namespace(mut self, namespace: impl Into<String>) -> Self {
        self.provider_namespace = Some(namespace.into());
        self
    }

    pub fn with_granularities(mut self, granularities: Vec<VcsSelectionGranularity>) -> Self {
        self.granularities = granularities;
        self
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VcsCapabilityState {
    Supported,
    Unsupported,
    Partial,
    ProviderNative,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VcsOperation {
    Status,
    ChangesList,
    ChangesRead,
    Selection,
    SnapshotCreate,
    Restore,
    LineList,
    LineSwitch,
    SyncFetch,
    SyncPull,
    SyncPush,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VcsSelectionGranularity {
    None,
    Path,
    Hunk,
    ProviderNative,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VcsChangedFile {
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_path: Option<PathBuf>,
    pub status: VcsChangedFileStatus,
    pub additions: u32,
    pub deletions: u32,
    #[serde(default)]
    pub binary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VcsChangedFileStatus {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    ProviderNative,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VcsChangedContentPage {
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    pub offset: u32,
    pub total_lines: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<u32>,
    #[serde(default)]
    pub binary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VcsDetectionClaim {
    pub workspace: VcsWorkspace,
    pub priority: i32,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VcsResolveRequest {
    pub workspace_root: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_provider_id: Option<VcsProviderId>,
}

#[derive(Clone)]
pub enum VcsProviderResolution {
    Available {
        provider: Arc<dyn VcsProvider>,
        claim: VcsDetectionClaim,
    },
    Unavailable {
        workspace_root: PathBuf,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VcsStatusRequest {
    pub workspace_root: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VcsListChangesRequest {
    pub workspace_root: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VcsReadChangedContentRequest {
    pub workspace_root: PathBuf,
    pub path: PathBuf,
    pub offset: u32,
    pub limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VcsSelectionRequest {
    pub workspace_root: PathBuf,
    pub paths: Vec<PathBuf>,
    pub granularity: VcsSelectionGranularity,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VcsSnapshotCreateRequest {
    pub workspace_root: PathBuf,
    pub message: String,
    #[serde(default)]
    pub paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VcsSnapshot {
    pub provider_id: VcsProviderId,
    pub id: VcsSnapshotId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VcsRestoreRequest {
    pub workspace_root: PathBuf,
    pub paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VcsLineSwitchRequest {
    pub workspace_root: PathBuf,
    pub line_id: VcsLineId,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VcsSyncOperation {
    Fetch,
    Pull,
    Push,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VcsSyncRequest {
    pub workspace_root: PathBuf,
    pub operation: VcsSyncOperation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VcsOperationResult {
    pub provider_id: VcsProviderId,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum VcsError {
    Unavailable {
        operation: VcsOperation,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_id: Option<VcsProviderId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<PathBuf>,
        message: String,
    },
    UnsupportedOperation {
        provider_id: VcsProviderId,
        operation: VcsOperation,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        capability: Option<VcsOperationCapability>,
        message: String,
    },
    PathInvalid {
        provider_id: VcsProviderId,
        path: PathBuf,
        message: String,
    },
    DirtyWorkspace {
        provider_id: VcsProviderId,
        operation: VcsOperation,
        message: String,
    },
    CommandFailed {
        provider_id: VcsProviderId,
        operation: VcsOperation,
        command: String,
        exit_code: Option<i32>,
        stderr: String,
    },
    ProviderNativeRequired {
        provider_id: VcsProviderId,
        operation: VcsOperation,
        namespace: String,
        message: String,
    },
}

impl std::fmt::Display for VcsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable { message, .. }
            | Self::UnsupportedOperation { message, .. }
            | Self::PathInvalid { message, .. }
            | Self::DirtyWorkspace { message, .. }
            | Self::ProviderNativeRequired { message, .. } => f.write_str(message),
            Self::CommandFailed {
                command, stderr, ..
            } => write!(f, "{command} failed: {stderr}"),
        }
    }
}

impl std::error::Error for VcsError {}

#[async_trait::async_trait]
pub trait VcsProvider: Send + Sync + 'static {
    fn id(&self) -> VcsProviderId;
    fn display_name(&self) -> String;

    async fn detect(&self, workspace_root: &Path) -> Result<Option<VcsDetectionClaim>, VcsError>;
    async fn status(&self, request: VcsStatusRequest) -> Result<VcsStatus, VcsError>;
    async fn list_changes(
        &self,
        request: VcsListChangesRequest,
    ) -> Result<Vec<VcsChangedFile>, VcsError>;
    async fn status_with_changes(
        &self,
        request: VcsListChangesRequest,
    ) -> Result<VcsStatusWithChanges, VcsError> {
        let status = self
            .status(VcsStatusRequest {
                workspace_root: request.workspace_root.clone(),
            })
            .await?;
        let files = self.list_changes(request).await?;
        Ok(VcsStatusWithChanges { status, files })
    }
    async fn list_changes_against_base(
        &self,
        request: VcsListChangesRequest,
        _base: Option<VcsBase>,
    ) -> Result<Vec<VcsChangedFile>, VcsError> {
        self.list_changes(request).await
    }
    async fn read_changed_content(
        &self,
        request: VcsReadChangedContentRequest,
    ) -> Result<VcsChangedContentPage, VcsError>;

    async fn select(&self, request: VcsSelectionRequest) -> Result<VcsOperationResult, VcsError> {
        Err(unsupported(
            self.id(),
            VcsOperation::Selection,
            "selection is not supported by this provider",
            request.granularity,
        ))
    }

    async fn create_snapshot(
        &self,
        _request: VcsSnapshotCreateRequest,
    ) -> Result<VcsSnapshot, VcsError> {
        Err(unsupported(
            self.id(),
            VcsOperation::SnapshotCreate,
            "snapshot creation is not supported by this provider",
            VcsSelectionGranularity::None,
        ))
    }

    async fn restore(&self, _request: VcsRestoreRequest) -> Result<VcsOperationResult, VcsError> {
        Err(unsupported(
            self.id(),
            VcsOperation::Restore,
            "restore is not supported by this provider",
            VcsSelectionGranularity::None,
        ))
    }

    async fn list_lines(&self, workspace_root: PathBuf) -> Result<Vec<VcsLineOfWork>, VcsError> {
        Err(VcsError::UnsupportedOperation {
            provider_id: self.id(),
            operation: VcsOperation::LineList,
            capability: None,
            message: format!(
                "line listing is not supported for {}",
                workspace_root.display()
            ),
        })
    }

    async fn switch_line(
        &self,
        _request: VcsLineSwitchRequest,
    ) -> Result<VcsOperationResult, VcsError> {
        Err(VcsError::UnsupportedOperation {
            provider_id: self.id(),
            operation: VcsOperation::LineSwitch,
            capability: None,
            message: "line switching is not supported by this provider".to_string(),
        })
    }

    async fn sync(&self, request: VcsSyncRequest) -> Result<VcsOperationResult, VcsError> {
        let operation = match request.operation {
            VcsSyncOperation::Fetch => VcsOperation::SyncFetch,
            VcsSyncOperation::Pull => VcsOperation::SyncPull,
            VcsSyncOperation::Push => VcsOperation::SyncPush,
        };
        Err(VcsError::UnsupportedOperation {
            provider_id: self.id(),
            operation,
            capability: None,
            message: "sync is not supported by this provider".to_string(),
        })
    }
}

#[async_trait::async_trait]
pub trait VcsProviderResolver: Send + Sync + 'static {
    async fn resolve_provider(
        &self,
        request: VcsResolveRequest,
    ) -> Result<VcsProviderResolution, VcsError>;
}

#[derive(Clone, Default)]
pub struct RegistryVcsProviderResolver {
    providers: Vec<Arc<dyn VcsProvider>>,
}

impl RegistryVcsProviderResolver {
    pub fn new(providers: Vec<Arc<dyn VcsProvider>>) -> Self {
        Self { providers }
    }
}

#[async_trait::async_trait]
impl VcsProviderResolver for RegistryVcsProviderResolver {
    async fn resolve_provider(
        &self,
        request: VcsResolveRequest,
    ) -> Result<VcsProviderResolution, VcsError> {
        if let Some(preferred) = request.preferred_provider_id {
            for provider in &self.providers {
                if provider.id() == preferred {
                    return match provider.detect(&request.workspace_root).await? {
                        Some(claim) => Ok(VcsProviderResolution::Available {
                            provider: Arc::clone(provider),
                            claim,
                        }),
                        None => Ok(VcsProviderResolution::Unavailable {
                            workspace_root: request.workspace_root,
                        }),
                    };
                }
            }
            return Ok(VcsProviderResolution::Unavailable {
                workspace_root: request.workspace_root,
            });
        }

        let mut claims = Vec::new();
        for provider in &self.providers {
            if let Some(claim) = provider.detect(&request.workspace_root).await? {
                claims.push((Arc::clone(provider), claim));
            }
        }
        claims.sort_by(
            |(left_provider, left_claim), (right_provider, right_claim)| {
                right_claim
                    .priority
                    .cmp(&left_claim.priority)
                    .then_with(|| {
                        right_claim
                            .workspace
                            .root
                            .components()
                            .count()
                            .cmp(&left_claim.workspace.root.components().count())
                    })
                    .then_with(|| left_provider.id().cmp(&right_provider.id()))
            },
        );
        if let Some((provider, claim)) = claims.into_iter().next() {
            Ok(VcsProviderResolution::Available { provider, claim })
        } else {
            Ok(VcsProviderResolution::Unavailable {
                workspace_root: request.workspace_root,
            })
        }
    }
}

fn unsupported(
    provider_id: VcsProviderId,
    operation: VcsOperation,
    message: impl Into<String>,
    granularity: VcsSelectionGranularity,
) -> VcsError {
    VcsError::UnsupportedOperation {
        provider_id,
        operation,
        capability: Some(
            VcsOperationCapability::new(operation, VcsCapabilityState::Unsupported)
                .with_granularities(vec![granularity]),
        ),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vcs_capability_states_round_trip_json() {
        let capabilities = VcsCapabilities {
            operations: vec![
                VcsOperationCapability::new(VcsOperation::Status, VcsCapabilityState::Supported),
                VcsOperationCapability::new(
                    VcsOperation::Selection,
                    VcsCapabilityState::Unsupported,
                )
                .with_reason("hunk selection unavailable")
                .with_granularities(vec![VcsSelectionGranularity::Path]),
                VcsOperationCapability::new(VcsOperation::Restore, VcsCapabilityState::Partial),
                VcsOperationCapability::new(
                    VcsOperation::SyncPush,
                    VcsCapabilityState::ProviderNative,
                )
                .with_provider_namespace("jj"),
            ],
        };

        let encoded = serde_json::to_value(&capabilities).expect("serialize capabilities");
        let decoded =
            serde_json::from_value::<VcsCapabilities>(encoded).expect("deserialize capabilities");

        assert_eq!(decoded, capabilities);
        assert_eq!(
            decoded
                .capability_for(VcsOperation::SyncPush)
                .unwrap()
                .state,
            VcsCapabilityState::ProviderNative
        );
    }

    #[test]
    fn vcs_error_serializes_provider_operation_path_and_command_context() {
        let error = VcsError::CommandFailed {
            provider_id: "git".to_string(),
            operation: VcsOperation::ChangesRead,
            command: "git diff -- path".to_string(),
            exit_code: Some(128),
            stderr: "bad path".to_string(),
        };

        let encoded = serde_json::to_value(&error).expect("serialize vcs error");

        assert_eq!(encoded["providerId"], "git");
        assert_eq!(encoded["operation"], "changes_read");
        assert_eq!(encoded["command"], "git diff -- path");
        assert_eq!(encoded["exitCode"], 128);
        assert_eq!(encoded["stderr"], "bad path");
    }

    #[tokio::test]
    async fn overlapping_detection_claims_resolve_by_priority_then_provider_id() {
        let workspace = PathBuf::from("/workspace");
        let resolver = RegistryVcsProviderResolver::new(vec![
            Arc::new(FakeProvider::new("zz-low", 1, &workspace)),
            Arc::new(FakeProvider::new("bb-high", 9, &workspace)),
            Arc::new(FakeProvider::new("aa-high", 9, &workspace)),
        ]);

        let resolution = resolver
            .resolve_provider(VcsResolveRequest {
                workspace_root: workspace,
                preferred_provider_id: None,
            })
            .await
            .expect("resolve provider");

        let VcsProviderResolution::Available { provider, claim } = resolution else {
            panic!("expected active provider");
        };
        assert_eq!(provider.id(), "aa-high");
        assert_eq!(claim.priority, 9);
    }

    #[tokio::test]
    async fn preferred_provider_takes_precedence_over_claim_priority() {
        let workspace = PathBuf::from("/workspace");
        let resolver = RegistryVcsProviderResolver::new(vec![
            Arc::new(FakeProvider::new("git", 1, &workspace)),
            Arc::new(FakeProvider::new("jj", 9, &workspace)),
        ]);

        let resolution = resolver
            .resolve_provider(VcsResolveRequest {
                workspace_root: workspace,
                preferred_provider_id: Some("git".to_string()),
            })
            .await
            .expect("resolve provider");

        let VcsProviderResolution::Available { provider, .. } = resolution else {
            panic!("expected active provider");
        };
        assert_eq!(provider.id(), "git");
    }

    #[tokio::test]
    async fn async_provider_trait_can_wrap_blocking_work_in_provider() {
        let provider = FakeProvider::new("blocking", 1, Path::new("/workspace"));

        let status = provider
            .status(VcsStatusRequest {
                workspace_root: PathBuf::from("/workspace"),
            })
            .await
            .expect("status");

        assert_eq!(status.provider.id, "blocking");
        assert_eq!(status.changed_file_count, 1);
    }

    #[tokio::test]
    async fn unsupported_hunk_selection_reports_capability_error() {
        let provider = FakeProvider::new("fake", 1, Path::new("/workspace"));

        let error = provider
            .select(VcsSelectionRequest {
                workspace_root: PathBuf::from("/workspace"),
                paths: vec![PathBuf::from("src/lib.rs")],
                granularity: VcsSelectionGranularity::Hunk,
            })
            .await
            .expect_err("hunk selection should be unsupported");

        let VcsError::UnsupportedOperation {
            operation,
            capability,
            ..
        } = error
        else {
            panic!("expected unsupported operation");
        };
        assert_eq!(operation, VcsOperation::Selection);
        assert_eq!(
            capability.unwrap().granularities,
            vec![VcsSelectionGranularity::Hunk]
        );
    }

    struct FakeProvider {
        id: String,
        priority: i32,
        root: PathBuf,
    }

    impl FakeProvider {
        fn new(id: impl Into<String>, priority: i32, root: &Path) -> Self {
            Self {
                id: id.into(),
                priority,
                root: root.to_path_buf(),
            }
        }
    }

    #[async_trait::async_trait]
    impl VcsProvider for FakeProvider {
        fn id(&self) -> VcsProviderId {
            self.id.clone()
        }

        fn display_name(&self) -> String {
            self.id.clone()
        }

        async fn detect(
            &self,
            _workspace_root: &Path,
        ) -> Result<Option<VcsDetectionClaim>, VcsError> {
            Ok(Some(VcsDetectionClaim {
                workspace: VcsWorkspace {
                    root: self.root.clone(),
                    id: None,
                },
                priority: self.priority,
                metadata: serde_json::Value::Null,
            }))
        }

        async fn status(&self, request: VcsStatusRequest) -> Result<VcsStatus, VcsError> {
            let id = self.id.clone();
            tokio::task::spawn_blocking(move || VcsStatus {
                provider: VcsProviderIdentity {
                    id: id.clone(),
                    display_name: id,
                },
                workspace: VcsWorkspace {
                    root: request.workspace_root,
                    id: None,
                },
                active_line: None,
                base: None,
                capabilities: VcsCapabilities {
                    operations: vec![VcsOperationCapability::new(
                        VcsOperation::Status,
                        VcsCapabilityState::Supported,
                    )],
                },
                changed_file_count: 1,
            })
            .await
            .map_err(|err| VcsError::Unavailable {
                operation: VcsOperation::Status,
                provider_id: Some(self.id.clone()),
                path: None,
                message: err.to_string(),
            })
        }

        async fn list_changes(
            &self,
            _request: VcsListChangesRequest,
        ) -> Result<Vec<VcsChangedFile>, VcsError> {
            Ok(Vec::new())
        }

        async fn read_changed_content(
            &self,
            request: VcsReadChangedContentRequest,
        ) -> Result<VcsChangedContentPage, VcsError> {
            Ok(VcsChangedContentPage {
                path: request.path,
                content: Some(String::new()),
                offset: request.offset,
                total_lines: 0,
                next_offset: None,
                binary: false,
            })
        }
    }
}
