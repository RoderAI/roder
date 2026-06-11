//! `ForkProvider` implementation over the Rift CLI adapter.

use std::path::{Path, PathBuf};

use roder_api::forks::{
    ForkCapabilities, ForkId, ForkProvider, ForkProviderDescriptor, ForkProvenance, ForkRequest,
    ForkStatus, RemoveForkPolicy, RemoveForkResult, WorkspaceFork,
};
use time::OffsetDateTime;

use crate::cli::{parse_created_path, parse_list, run_rift};
use crate::config::RiftConfig;
use crate::errors::RiftError;

pub const RIFT_FORK_PROVIDER_ID: &str = "rift";

#[derive(Debug, Default)]
pub struct RiftForkProvider {
    config: RiftConfig,
}

impl RiftForkProvider {
    pub fn new(config: RiftConfig) -> Self {
        Self { config }
    }

    fn fork(&self, source: &Path, name: &str, path: String, status: ForkStatus) -> WorkspaceFork {
        WorkspaceFork {
            id: path.clone(),
            provider_id: RIFT_FORK_PROVIDER_ID.to_string(),
            source_workspace: source.to_path_buf(),
            workspace: PathBuf::from(path),
            status,
            provenance: ForkProvenance {
                branch: None,
                source_branch: None,
                source_commit: None,
                snapshot_id: Some(name.to_string()),
                session_id: None,
                created_at: OffsetDateTime::now_utc(),
            },
            cleanup: Default::default(),
            metadata: serde_json::json!({ "snapshotName": name, "copyOnWrite": true }),
        }
    }
}

#[async_trait::async_trait]
impl ForkProvider for RiftForkProvider {
    fn descriptor(&self) -> ForkProviderDescriptor {
        ForkProviderDescriptor {
            id: RIFT_FORK_PROVIDER_ID.to_string(),
            display_name: "Rift snapshot".to_string(),
            capabilities: ForkCapabilities {
                create: true,
                list: true,
                remove: true,
                resume: true,
                diff_summary: false,
                merge_back: false,
                copy_on_write: true,
                remote_compute: false,
            },
        }
    }

    async fn create_fork(&self, request: ForkRequest) -> anyhow::Result<WorkspaceFork> {
        anyhow::ensure!(
            !request.policy.allow_dirty_source,
            "the rift provider snapshots the source as-is; allow_dirty_source has no meaning \
             here and must stay false"
        );
        let source = request.source_workspace.clone();
        let name = request.name.clone().unwrap_or_else(|| "fork".to_string());
        anyhow::ensure!(
            name.chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.')),
            "rift fork names must be alphanumeric/dash/underscore, got {name:?}"
        );
        let dest_base = self
            .config
            .base_dir
            .clone()
            .unwrap_or_else(|| source.join(".roder/rift-forks"));
        std::fs::create_dir_all(&dest_base)?;

        run_rift(&self.config, &source, &["init"]).await?;
        let dest = dest_base.display().to_string();
        let output = run_rift(
            &self.config,
            &source,
            &["create", &name, "--dest", &dest],
        )
        .await?;
        let created = parse_created_path(&output)?;
        Ok(self.fork(&source, &name, created, ForkStatus::Active))
    }

    async fn list_forks(&self, source_workspace: &Path) -> anyhow::Result<Vec<WorkspaceFork>> {
        let output = run_rift(&self.config, source_workspace, &["list"]).await?;
        Ok(parse_list(&output)
            .into_iter()
            .map(|(name, path)| {
                let status = if Path::new(&path).is_dir() {
                    ForkStatus::Active
                } else {
                    ForkStatus::Missing
                };
                self.fork(source_workspace, &name, path, status)
            })
            .collect())
    }

    async fn resume_fork(&self, id: &ForkId) -> anyhow::Result<WorkspaceFork> {
        let path = PathBuf::from(id);
        let status = if path.is_dir() {
            ForkStatus::Active
        } else {
            ForkStatus::Missing
        };
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "fork".to_string());
        // Provenance beyond the path requires the source workspace; resume
        // reports liveness, and listing from the source recovers names.
        Ok(self.fork(Path::new(""), &name, id.clone(), status))
    }

    async fn remove_fork(
        &self,
        id: &ForkId,
        policy: RemoveForkPolicy,
    ) -> anyhow::Result<RemoveForkResult> {
        let path = PathBuf::from(id);
        if policy.confirm_workspace != path {
            return Err(RiftError::new(
                "confirmation_mismatch",
                Some(path.display().to_string()),
                "removal is path-confirmed: confirm the exact fork workspace",
            )
            .into());
        }
        let cwd = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        run_rift(&self.config, &cwd, &["remove", id]).await?;
        Ok(RemoveForkResult {
            id: id.clone(),
            removed: true,
            workspace: path,
        })
    }
}
