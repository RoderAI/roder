//! Minimal in-memory [`VcsProvider`] so review tests can exercise target
//! resolution without a real repository on disk.

use std::path::Path;

use roder_api::version_control::{
    VcsBase, VcsCapabilities, VcsChangedContentPage, VcsChangedFile, VcsDetectionClaim, VcsError,
    VcsListChangesRequest, VcsProvider, VcsProviderId, VcsProviderIdentity,
    VcsReadChangedContentRequest, VcsStatus, VcsStatusRequest, VcsWorkspace,
};

pub(crate) struct StubVcs {
    base: Option<VcsBase>,
    head: Option<String>,
}

impl StubVcs {
    pub(crate) fn new(ref_name: Option<&str>, sha: Option<&str>) -> Self {
        Self {
            base: Some(VcsBase {
                ref_name: ref_name.map(str::to_string),
                sha: sha.map(str::to_string),
            }),
            head: Some("headsha".to_string()),
        }
    }
}

#[async_trait::async_trait]
impl VcsProvider for StubVcs {
    fn id(&self) -> VcsProviderId {
        "stub".to_string()
    }

    fn display_name(&self) -> String {
        "Stub".to_string()
    }

    /// Always claims the workspace; `RegistryVcsProviderResolver` picks the
    /// provider from this.
    async fn detect(&self, workspace_root: &Path) -> Result<Option<VcsDetectionClaim>, VcsError> {
        Ok(Some(VcsDetectionClaim {
            workspace: VcsWorkspace {
                root: workspace_root.to_path_buf(),
                id: None,
            },
            priority: 1,
            metadata: serde_json::Value::Null,
        }))
    }

    async fn status(&self, request: VcsStatusRequest) -> Result<VcsStatus, VcsError> {
        Ok(VcsStatus {
            provider: VcsProviderIdentity {
                id: self.id(),
                display_name: self.display_name(),
            },
            workspace: VcsWorkspace {
                root: request.workspace_root,
                id: self.head.clone(),
            },
            active_line: None,
            base: self.base.clone(),
            capabilities: VcsCapabilities::default(),
            changed_file_count: 0,
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
            content: None,
            offset: 0,
            total_lines: 0,
            next_offset: None,
            binary: false,
        })
    }
}
