use std::path::Path;
use std::sync::Arc;

use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use roder_api::version_control::{
    VcsChangedContentPage, VcsChangedFile, VcsDetectionClaim, VcsError, VcsLineOfWork,
    VcsLineSwitchRequest, VcsListChangesRequest, VcsOperationResult, VcsProvider, VcsProviderId,
    VcsReadChangedContentRequest, VcsRestoreRequest, VcsSelectionRequest, VcsSnapshot,
    VcsSnapshotCreateRequest, VcsStatus, VcsStatusRequest, VcsStatusWithChanges, VcsSyncRequest,
    VcsWorkspace,
};
use semver::Version;

use crate::git::GitRepo;

pub const GIT_VCS_PROVIDER_ID: &str = "git";

#[derive(Debug, Default)]
pub struct GitExtension;

impl RoderExtension for GitExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-git".to_string(),
            name: "Git VCS Provider".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("Provides git-backed version-control workflows.".to_string()),
            provides: vec![ProvidedService::VersionControlProvider(
                GIT_VCS_PROVIDER_ID.to_string(),
            )],
            required_capabilities: Vec::new(),
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.version_control_provider(Arc::new(GitProvider));
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct GitProvider;

#[async_trait::async_trait]
impl VcsProvider for GitProvider {
    fn id(&self) -> VcsProviderId {
        GIT_VCS_PROVIDER_ID.to_string()
    }

    fn display_name(&self) -> String {
        "Git".to_string()
    }

    async fn detect(&self, workspace_root: &Path) -> Result<Option<VcsDetectionClaim>, VcsError> {
        let workspace_root = workspace_root.to_path_buf();
        tokio::task::spawn_blocking(move || {
            match GitRepo::open(&workspace_root, GIT_VCS_PROVIDER_ID.to_string()) {
                Ok(repo) => Ok(Some(VcsDetectionClaim {
                    workspace: VcsWorkspace {
                        root: repo.root().to_path_buf(),
                        id: None,
                    },
                    priority: 100,
                    metadata: serde_json::json!({ "kind": "git" }),
                })),
                Err(VcsError::CommandFailed { .. }) => Ok(None),
                Err(error) => Err(error),
            }
        })
        .await
        .map_err(join_error)?
    }

    async fn status(&self, request: VcsStatusRequest) -> Result<VcsStatus, VcsError> {
        tokio::task::spawn_blocking(move || {
            GitRepo::open(&request.workspace_root, GIT_VCS_PROVIDER_ID.to_string())?.status()
        })
        .await
        .map_err(join_error)?
    }

    async fn list_changes(
        &self,
        request: VcsListChangesRequest,
    ) -> Result<Vec<VcsChangedFile>, VcsError> {
        tokio::task::spawn_blocking(move || {
            GitRepo::open(&request.workspace_root, GIT_VCS_PROVIDER_ID.to_string())?.list_changes()
        })
        .await
        .map_err(join_error)?
    }

    async fn status_with_changes(
        &self,
        request: VcsListChangesRequest,
    ) -> Result<VcsStatusWithChanges, VcsError> {
        tokio::task::spawn_blocking(move || {
            GitRepo::open(&request.workspace_root, GIT_VCS_PROVIDER_ID.to_string())?
                .status_with_changes()
        })
        .await
        .map_err(join_error)?
    }

    async fn list_changes_against_base(
        &self,
        request: VcsListChangesRequest,
        base: Option<roder_api::version_control::VcsBase>,
    ) -> Result<Vec<VcsChangedFile>, VcsError> {
        tokio::task::spawn_blocking(move || {
            let repo = GitRepo::open(&request.workspace_root, GIT_VCS_PROVIDER_ID.to_string())?;
            if let Some(base_sha) = base.and_then(|base| base.sha) {
                return repo.list_changes_against_base(&base_sha);
            }
            repo.list_changes()
        })
        .await
        .map_err(join_error)?
    }

    async fn read_changed_content(
        &self,
        request: VcsReadChangedContentRequest,
    ) -> Result<VcsChangedContentPage, VcsError> {
        tokio::task::spawn_blocking(move || {
            GitRepo::open(&request.workspace_root, GIT_VCS_PROVIDER_ID.to_string())?
                .read_changed_content(request)
        })
        .await
        .map_err(join_error)?
    }

    async fn select(&self, request: VcsSelectionRequest) -> Result<VcsOperationResult, VcsError> {
        tokio::task::spawn_blocking(move || {
            GitRepo::open(&request.workspace_root, GIT_VCS_PROVIDER_ID.to_string())?.select(request)
        })
        .await
        .map_err(join_error)?
    }

    async fn create_snapshot(
        &self,
        request: VcsSnapshotCreateRequest,
    ) -> Result<VcsSnapshot, VcsError> {
        tokio::task::spawn_blocking(move || {
            GitRepo::open(&request.workspace_root, GIT_VCS_PROVIDER_ID.to_string())?
                .create_snapshot(request)
        })
        .await
        .map_err(join_error)?
    }

    async fn restore(&self, request: VcsRestoreRequest) -> Result<VcsOperationResult, VcsError> {
        tokio::task::spawn_blocking(move || {
            GitRepo::open(&request.workspace_root, GIT_VCS_PROVIDER_ID.to_string())?
                .restore(request)
        })
        .await
        .map_err(join_error)?
    }

    async fn list_lines(
        &self,
        workspace_root: std::path::PathBuf,
    ) -> Result<Vec<VcsLineOfWork>, VcsError> {
        tokio::task::spawn_blocking(move || {
            GitRepo::open(&workspace_root, GIT_VCS_PROVIDER_ID.to_string())?.list_lines()
        })
        .await
        .map_err(join_error)?
    }

    async fn switch_line(
        &self,
        request: VcsLineSwitchRequest,
    ) -> Result<VcsOperationResult, VcsError> {
        tokio::task::spawn_blocking(move || {
            let line_id = request.line_id.clone();
            GitRepo::open(&request.workspace_root, GIT_VCS_PROVIDER_ID.to_string())?
                .switch_line(&line_id)
        })
        .await
        .map_err(join_error)?
    }

    async fn sync(&self, request: VcsSyncRequest) -> Result<VcsOperationResult, VcsError> {
        tokio::task::spawn_blocking(move || {
            GitRepo::open(&request.workspace_root, GIT_VCS_PROVIDER_ID.to_string())?.sync(request)
        })
        .await
        .map_err(join_error)?
    }
}

fn join_error(err: tokio::task::JoinError) -> VcsError {
    VcsError::Unavailable {
        operation: roder_api::version_control::VcsOperation::Status,
        provider_id: Some(GIT_VCS_PROVIDER_ID.to_string()),
        path: None,
        message: err.to_string(),
    }
}
