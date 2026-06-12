//! App-server handlers for workspace forks (roadmap phases 90 + 81):
//! `thread/fork`, `thread/fork_status`, `thread/remove_fork`, plus the
//! provider-facing `forks/providers/list`, `forks/list`, `forks/create`,
//! and `forks/remove`.

use std::path::PathBuf;

use roder_api::forks::{ForkPolicy, ForkReason, ForkRequest, ForkStatus, RemoveForkPolicy};
use roder_core::conversation_forks::ForkThreadRequest;
use roder_core::forks::DEFAULT_FORK_PROVIDER;
use roder_protocol::{
    ForksCreateParams, ForksCreateResult, ForksListParams, ForksListResult,
    ForksProvidersListResult, ForksRemoveParams, ForksRemoveResult, JsonRpcError, ThreadForkParams,
    ThreadForkResult, ThreadForkStatusParams, ThreadForkStatusResult, ThreadRemoveForkParams,
    ThreadRemoveForkResult,
};

use crate::protocol_contract::{idle_thread_status, protocol_thread_from_metadata};
use crate::server::{AppServer, internal_error};

/// Explicit param wins; then `RODER_FORK_PROVIDER`/`[forks].default_provider`
/// config; then the built-in `git-worktree` default.
fn provider_or_default(provider: Option<String>) -> String {
    provider.unwrap_or_else(|| {
        roder_config::load_config()
            .map(|config| roder_config::default_fork_provider(&config))
            .unwrap_or_else(|_| DEFAULT_FORK_PROVIDER.to_string())
    })
}

impl AppServer {
    pub(crate) async fn handle_thread_fork(
        &self,
        params: ThreadForkParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let outcome = self
            .runtime
            .fork_thread(ForkThreadRequest {
                parent_thread_id: params.thread_id,
                name: params.name,
                from_turn_id: params.from_turn_id,
                provider_id: Some(provider_or_default(params.provider)),
                provider_config: params.provider_config.unwrap_or(serde_json::json!({})),
            })
            .await
            .map_err(internal_error)?;
        let fork = outcome
            .child
            .workspace_fork
            .clone()
            .ok_or_else(|| internal_error("forked thread is missing its fork provenance"))?;
        let thread = protocol_thread_from_metadata(outcome.child, None, idle_thread_status());
        serde_json::to_value(ThreadForkResult {
            thread,
            fork,
            warnings: outcome.warnings,
        })
        .map_err(internal_error)
    }

    pub(crate) async fn handle_thread_fork_status(
        &self,
        params: ThreadForkStatusParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let metadata = self
            .runtime
            .load_thread_metadata(&params.thread_id)
            .await
            .map_err(internal_error)?
            .ok_or_else(|| fork_not_found(format!("thread {} was not found", params.thread_id)))?;
        let workspace_missing = metadata.workspace_fork.as_ref().is_some_and(|fork| {
            fork.status == ForkStatus::Active && !std::path::Path::new(&metadata.workspace).is_dir()
        });
        serde_json::to_value(ThreadForkStatusResult {
            thread_id: metadata.thread_id,
            parent_thread_id: metadata.parent_thread_id,
            forked_from_turn_id: metadata.forked_from_turn_id,
            fork: metadata.workspace_fork,
            workspace_missing,
        })
        .map_err(internal_error)
    }

    pub(crate) async fn handle_thread_remove_fork(
        &self,
        params: ThreadRemoveForkParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let fork = self
            .runtime
            .remove_thread_workspace_fork(&params.thread_id, &params.confirm_path)
            .await
            .map_err(internal_error)?;
        serde_json::to_value(ThreadRemoveForkResult {
            thread_id: params.thread_id,
            fork,
        })
        .map_err(internal_error)
    }

    pub(crate) async fn handle_forks_providers_list(
        &self,
    ) -> Result<serde_json::Value, JsonRpcError> {
        serde_json::to_value(ForksProvidersListResult {
            providers: self.runtime.fork_providers(),
        })
        .map_err(internal_error)
    }

    pub(crate) async fn handle_forks_list(
        &self,
        params: ForksListParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let forks = self
            .runtime
            .list_workspace_forks(
                &provider_or_default(params.provider),
                std::path::Path::new(&params.source_workspace),
            )
            .await
            .map_err(internal_error)?;
        serde_json::to_value(ForksListResult { forks }).map_err(internal_error)
    }

    pub(crate) async fn handle_forks_create(
        &self,
        params: ForksCreateParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let fork = self
            .runtime
            .create_workspace_fork(
                &provider_or_default(params.provider),
                ForkRequest {
                    source_workspace: PathBuf::from(&params.source_workspace),
                    name: params.name,
                    reason: ForkReason::Other,
                    policy: ForkPolicy::default(),
                    provider_config: params.provider_config.unwrap_or(serde_json::json!({})),
                },
            )
            .await
            .map_err(internal_error)?;
        serde_json::to_value(ForksCreateResult { fork }).map_err(internal_error)
    }

    pub(crate) async fn handle_forks_remove(
        &self,
        params: ForksRemoveParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let result = self
            .runtime
            .remove_workspace_fork(
                &provider_or_default(params.provider),
                &params.fork_id,
                RemoveForkPolicy {
                    confirm_workspace: PathBuf::from(&params.confirm_workspace),
                },
            )
            .await
            .map_err(internal_error)?;
        serde_json::to_value(ForksRemoveResult {
            fork_id: result.id,
            removed: result.removed,
        })
        .map_err(internal_error)
    }
}

fn fork_not_found(message: impl Into<String>) -> JsonRpcError {
    JsonRpcError {
        code: -32602,
        message: message.into(),
        data: None,
    }
}
