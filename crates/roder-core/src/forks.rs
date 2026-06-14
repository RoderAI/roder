//! Runtime fork manager (roadmap phase 81, Task 2): resolves configured
//! `ForkProvider`s from the extension registry and applies path/policy
//! checks before provider calls. Thread attachment lives in
//! `conversation_forks`; these are the provider-facing primitives shared by
//! conversation forks and the `forks/*` app-server surface.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use roder_api::forks::{
    ForkCapabilities, ForkId, ForkProvenance, ForkProvider, ForkProviderDescriptor, ForkRequest,
    ForkStatus, RemoveForkPolicy, RemoveForkResult, WorkspaceFork,
};
use roder_api::remote_runner::{RemoteRunnerProvider, RemoteRunnerSession, RunnerDestination};
use tokio::sync::Mutex;

use crate::Runtime;

/// Default fork provider when a request does not name one.
pub const DEFAULT_FORK_PROVIDER: &str = "git-worktree";

impl Runtime {
    /// Lists registered fork providers.
    pub fn fork_providers(&self) -> Vec<ForkProviderDescriptor> {
        self.registry
            .fork_providers
            .iter()
            .map(|provider| provider.descriptor())
            .collect()
    }

    /// Resolves a fork provider with an actionable error naming the
    /// available ids.
    pub fn fork_provider(&self, provider_id: &str) -> anyhow::Result<Arc<dyn ForkProvider>> {
        self.registry.fork_provider(provider_id).ok_or_else(|| {
            let available = self
                .registry
                .fork_providers
                .iter()
                .map(|provider| provider.descriptor().id)
                .collect::<Vec<_>>()
                .join(", ");
            anyhow::anyhow!(
                "fork provider {provider_id:?} is not installed (available: {})",
                if available.is_empty() {
                    "none"
                } else {
                    &available
                }
            )
        })
    }

    /// Creates a workspace fork after host-side validation.
    pub async fn create_workspace_fork(
        &self,
        provider_id: &str,
        request: ForkRequest,
    ) -> anyhow::Result<WorkspaceFork> {
        anyhow::ensure!(
            request.source_workspace.is_absolute(),
            "fork source workspace must be an absolute path: {}",
            request.source_workspace.display()
        );
        if let Some(name) = &request.name {
            anyhow::ensure!(
                !name.trim().is_empty() && name.len() <= 100,
                "fork names must be 1-100 characters"
            );
        }
        let provider = self.fork_provider(provider_id)?;
        anyhow::ensure!(
            provider.descriptor().capabilities.create,
            "fork provider {provider_id:?} does not support creation"
        );
        provider.create_fork(request).await
    }

    pub async fn list_workspace_forks(
        &self,
        provider_id: &str,
        source_workspace: &std::path::Path,
    ) -> anyhow::Result<Vec<WorkspaceFork>> {
        self.fork_provider(provider_id)?
            .list_forks(source_workspace)
            .await
    }

    pub async fn resume_workspace_fork(
        &self,
        provider_id: &str,
        id: &ForkId,
    ) -> anyhow::Result<WorkspaceFork> {
        self.fork_provider(provider_id)?.resume_fork(id).await
    }

    /// Removes a fork; destructive and always path-confirmed by the
    /// provider contract.
    pub async fn remove_workspace_fork(
        &self,
        provider_id: &str,
        id: &ForkId,
        policy: RemoveForkPolicy,
    ) -> anyhow::Result<RemoveForkResult> {
        self.fork_provider(provider_id)?
            .remove_fork(id, policy)
            .await
    }
}

/// Provider id used by the remote-runner fork adapter (roadmap phase 81,
/// Task 5).
pub const REMOTE_RUNNER_FORK_PROVIDER_ID: &str = "remote-runner";

/**
 * Represents a fresh remote-runner session as a `WorkspaceFork` with
 * `remote_compute = true`. The fork layer owns only lifecycle, provenance,
 * and attachment — file/process operations stay delegated to the
 * `RemoteRunnerSession` contract. `remove_fork` closes the runner session
 * (providers without snapshot deletion simply terminate the session, which
 * is documented as the deterministic cleanup behavior); `resume_fork`
 * re-opens it through the runner provider's `resume_session` path, with
 * snapshot-backed restore when the provider recorded one.
 */
pub struct RemoteRunnerForkAdapter {
    provider: Arc<dyn RemoteRunnerProvider>,
    destination: RunnerDestination,
    /// Absolute workspace path on the runner.
    runner_workspace: PathBuf,
    sessions: Mutex<HashMap<ForkId, Arc<dyn RemoteRunnerSession>>>,
}

impl RemoteRunnerForkAdapter {
    pub fn new(
        provider: Arc<dyn RemoteRunnerProvider>,
        destination: RunnerDestination,
        runner_workspace: PathBuf,
    ) -> Self {
        Self {
            provider,
            destination,
            runner_workspace,
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// The live runner session backing a fork, for tool execution wiring.
    pub async fn session(&self, id: &ForkId) -> Option<Arc<dyn RemoteRunnerSession>> {
        self.sessions.lock().await.get(id).cloned()
    }

    fn fork_for(&self, session: &Arc<dyn RemoteRunnerSession>) -> WorkspaceFork {
        let state = session.state();
        WorkspaceFork {
            id: state.session_id.clone(),
            provider_id: REMOTE_RUNNER_FORK_PROVIDER_ID.to_string(),
            source_workspace: self.runner_workspace.clone(),
            workspace: self.runner_workspace.clone(),
            status: ForkStatus::Active,
            provenance: ForkProvenance {
                branch: None,
                source_branch: None,
                source_commit: None,
                snapshot_id: state
                    .snapshot
                    .as_ref()
                    .map(|snapshot| snapshot.snapshot_id.clone()),
                session_id: Some(state.session_id),
                created_at: time::OffsetDateTime::now_utc(),
            },
            cleanup: Default::default(),
            metadata: serde_json::json!({
                "runnerProviderId": state.provider_id,
                "destinationId": state.destination_id,
            }),
        }
    }
}

#[async_trait::async_trait]
impl ForkProvider for RemoteRunnerForkAdapter {
    fn descriptor(&self) -> ForkProviderDescriptor {
        ForkProviderDescriptor {
            id: REMOTE_RUNNER_FORK_PROVIDER_ID.to_string(),
            display_name: format!("Remote runner ({})", self.destination.provider_id),
            capabilities: ForkCapabilities {
                create: true,
                list: false,
                remove: true,
                resume: true,
                diff_summary: false,
                merge_back: false,
                copy_on_write: false,
                remote_compute: true,
            },
        }
    }

    async fn create_fork(&self, request: ForkRequest) -> anyhow::Result<WorkspaceFork> {
        anyhow::ensure!(
            !request.policy.allow_dirty_source,
            "remote-runner forks always start from the destination's own state; \
             allow_dirty_source has no meaning here and must stay false"
        );
        let session = self
            .provider
            .create_session(self.destination.clone())
            .await?;
        let fork = self.fork_for(&session);
        self.sessions.lock().await.insert(fork.id.clone(), session);
        Ok(fork)
    }

    async fn list_forks(&self, _source: &std::path::Path) -> anyhow::Result<Vec<WorkspaceFork>> {
        // Runner providers own session listing; the adapter only tracks the
        // sessions it created in this process.
        let sessions = self.sessions.lock().await;
        Ok(sessions
            .values()
            .map(|session| self.fork_for(session))
            .collect())
    }

    async fn resume_fork(&self, id: &ForkId) -> anyhow::Result<WorkspaceFork> {
        if let Some(session) = self.session(id).await {
            return Ok(self.fork_for(&session));
        }
        // Snapshot-backed resume through the runner provider.
        let state = roder_api::remote_runner::RunnerSessionState {
            provider_id: self.destination.provider_id.clone(),
            session_id: id.clone(),
            destination_id: self.destination.id.clone(),
            snapshot: None,
            metadata: self.destination.config.clone(),
        };
        let session = self.provider.resume_session(state).await?;
        let fork = self.fork_for(&session);
        self.sessions.lock().await.insert(fork.id.clone(), session);
        Ok(fork)
    }

    async fn remove_fork(
        &self,
        id: &ForkId,
        policy: RemoveForkPolicy,
    ) -> anyhow::Result<RemoveForkResult> {
        anyhow::ensure!(
            policy.confirm_workspace == self.runner_workspace,
            "removal is path-confirmed: confirm the runner workspace {}",
            self.runner_workspace.display()
        );
        let session = self
            .sessions
            .lock()
            .await
            .remove(id)
            .ok_or_else(|| anyhow::anyhow!("remote fork {id} is not active in this process"))?;
        session.close().await?;
        Ok(RemoveForkResult {
            id: id.clone(),
            removed: true,
            workspace: self.runner_workspace.clone(),
        })
    }
}
