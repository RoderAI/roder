//! Provider-neutral workspace forks (roadmap phase 81).
//!
//! A **fork** is a writable workspace copy or session derived from a source
//! workspace — backing a thread, subagent lane, task branch, or experiment.
//! Providers implement concrete storage/compute behavior (Git worktrees,
//! Rift copy-on-write snapshots, remote sandboxes); core code only sees
//! these types. Forks are not inference providers and are not GitHub
//! repository forks.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

pub type ForkProviderId = String;
/// Provider-scoped fork identifier. Stable and resolvable by the provider
/// alone (e.g. the absolute worktree path for `git-worktree`).
pub type ForkId = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ForkProviderDescriptor {
    pub id: ForkProviderId,
    pub display_name: String,
    pub capabilities: ForkCapabilities,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ForkCapabilities {
    pub create: bool,
    pub list: bool,
    pub remove: bool,
    pub resume: bool,
    pub diff_summary: bool,
    pub merge_back: bool,
    pub copy_on_write: bool,
    pub remote_compute: bool,
}

/// Why a fork is being created; recorded for provenance/audit only.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ForkReason {
    ConversationFork,
    SubagentLane,
    TaskLane,
    Experiment,
    Other,
}

/// Source-state policy applied before fork creation.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ForkPolicy {
    /// Allow forking from a source with uncommitted/dirty state. Providers
    /// that cannot honor `true` must fail with a clear error rather than
    /// silently copying dirty state. Default: fail closed on dirty sources
    /// (Roder-owned `.roder/` state is always exempt).
    #[serde(default)]
    pub allow_dirty_source: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ForkStatus {
    Active,
    Removed,
    /// Recorded as existing but its workspace is missing on disk.
    Missing,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ForkCleanupPolicy {
    /// Removal only via an explicit, path-confirmed request (default).
    #[default]
    Explicit,
    /// Eligible for policy-driven auto-clean when its task lane exits.
    AutoOnTaskExit,
}

/// Provider-recorded provenance. All fields optional so local and remote
/// providers can populate what they actually know; never secret-bearing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ForkProvenance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

impl ForkProvenance {
    pub fn at(created_at: OffsetDateTime) -> Self {
        Self {
            branch: None,
            source_branch: None,
            source_commit: None,
            snapshot_id: None,
            session_id: None,
            created_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ForkRequest {
    pub source_workspace: PathBuf,
    /// User-facing fork name; providers sanitize into their naming scheme.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub reason: ForkReason,
    #[serde(default)]
    pub policy: ForkPolicy,
    /// Provider-specific options; must never carry secrets.
    #[serde(default)]
    pub provider_config: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFork {
    pub id: ForkId,
    pub provider_id: ForkProviderId,
    pub source_workspace: PathBuf,
    /// The writable workspace this fork provides.
    pub workspace: PathBuf,
    pub status: ForkStatus,
    pub provenance: ForkProvenance,
    #[serde(default)]
    pub cleanup: ForkCleanupPolicy,
    /// Non-secret provider metadata for display/debugging.
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// Removal is destructive and always path-confirmed: the caller must name
/// the exact fork workspace being removed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RemoveForkPolicy {
    pub confirm_workspace: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RemoveForkResult {
    pub id: ForkId,
    pub removed: bool,
    pub workspace: PathBuf,
}

/// Provider contract for workspace forks. Registered through the extension
/// registry (`ProvidedService::ForkProvider`); providers declare their
/// capabilities and must not assume ambient authority beyond them.
#[async_trait::async_trait]
pub trait ForkProvider: Send + Sync + 'static {
    fn descriptor(&self) -> ForkProviderDescriptor;

    async fn create_fork(&self, request: ForkRequest) -> anyhow::Result<WorkspaceFork>;

    /// Lists forks of `source_workspace` known to this provider.
    async fn list_forks(&self, source_workspace: &Path) -> anyhow::Result<Vec<WorkspaceFork>>;

    /// Re-resolves a fork by id (e.g. after restart), reporting `Missing`
    /// status when its workspace disappeared out-of-band.
    async fn resume_fork(&self, id: &ForkId) -> anyhow::Result<WorkspaceFork>;

    async fn remove_fork(
        &self,
        id: &ForkId,
        policy: RemoveForkPolicy,
    ) -> anyhow::Result<RemoveForkResult>;
}
