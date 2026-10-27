//! Workspace isolation is independent of conversation-history selection.
//! Persist the child binding before it can execute any tools.

use roder_api::forks::{
    ForkOwnership, ForkPolicy, ForkReason, ForkRequest, ForkStatus, RemoveForkPolicy,
};
use roder_api::thread::ThreadMetadata;

use super::*;

impl Runtime {
    pub(super) async fn validate_agent_worktree_parent(
        &self,
        parent: Option<&ThreadMetadata>,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.thread_store.is_some(),
            "worktree isolation requires a durable thread store"
        );
        anyhow::ensure!(
            parent.is_none_or(|metadata| metadata.runner_binding.is_none()
                && metadata.runner_destination.is_none())
                && self.config.read().await.remote_runner_destination.is_none(),
            "worktree isolation requires a local workspace; remote runner inheritance cannot be silently changed"
        );
        self.fork_provider(crate::forks::DEFAULT_FORK_PROVIDER)?;
        Ok(())
    }

    pub(super) async fn isolate_agent_workspace(
        &self,
        parent_thread_id: &ThreadId,
        mut thread: ThreadMetadata,
    ) -> anyhow::Result<ThreadMetadata> {
        let fork = self
            .create_workspace_fork(
                crate::forks::DEFAULT_FORK_PROVIDER,
                ForkRequest {
                    source_workspace: std::path::PathBuf::from(&thread.workspace),
                    ownership: Some(ForkOwnership {
                        creator_thread_id: Some(parent_thread_id.clone()),
                        owner_thread_id: Some(thread.thread_id.clone()),
                    }),
                    // Use an opaque stable identity, not user text as a directory.
                    name: Some(format!("agent-{}", thread.thread_id)),
                    reason: ForkReason::SubagentLane,
                    policy: ForkPolicy::default(),
                    provider_config: serde_json::json!({}),
                },
            )
            .await?;
        thread.workspace = fork.workspace.display().to_string();
        thread.workspace_id = None;
        thread.root_id = None;
        thread.runner_binding = None;
        thread.runner_destination = None;
        thread.runner_state = None;
        let note = format!(
            "\n\nThis agent uses an isolated, Roder-owned Git worktree at {}. \
             Execute coding tools in this workspace, not the parent's checkout. \
             It starts at committed HEAD; uncommitted parent changes were not copied. \
             Changes are not automatically merged or deleted when the agent exits.",
            thread.workspace
        );
        thread
            .developer_instructions
            .get_or_insert_default()
            .push_str(&note);
        thread.workspace_fork = Some(fork.clone());
        let store = self
            .thread_store
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("worktree isolation requires a durable thread store"))?;
        match store.update_thread_metadata(thread).await {
            Ok(thread) => Ok(thread),
            Err(error) => {
                // Only a newly created clean worktree is eligible. No force.
                let _ = self
                    .remove_workspace_fork(
                        &fork.provider_id,
                        &fork.id,
                        RemoveForkPolicy {
                            confirm_workspace: fork.workspace.clone(),
                        },
                    )
                    .await;
                Err(error)
            }
        }
    }

    /// Failed starts may reclaim a clean checkout; dirty work remains
    /// available for explicit review and cleanup through the fork API.
    pub(super) async fn discard_failed_agent_workspace(&self, thread_id: &ThreadId) {
        let Ok(Some(mut metadata)) = self.load_thread_metadata(thread_id).await else {
            return;
        };
        let Some(fork) = metadata.workspace_fork.as_mut() else {
            return;
        };
        if fork
            .ownership
            .as_ref()
            .and_then(|owner| owner.owner_thread_id.as_ref())
            != Some(thread_id)
        {
            return;
        }
        if self
            .remove_workspace_fork(
                &fork.provider_id,
                &fork.id,
                RemoveForkPolicy {
                    confirm_workspace: fork.workspace.clone(),
                },
            )
            .await
            .is_ok()
        {
            fork.status = ForkStatus::Removed;
            if let Some(store) = &self.thread_store {
                let _ = store.update_thread_metadata(metadata).await;
            }
        }
    }
}
