//! App-server handlers for native worktree conversation forks (roadmap
//! phase 90): `thread/fork_worktree`, `thread/fork_status`, and
//! `thread/remove_worktree_fork`.

use roder_api::thread::WorktreeForkStatus;
use roder_core::conversation_forks::ForkThreadRequest;
use roder_protocol::{
    JsonRpcError, ThreadForkStatusParams, ThreadForkStatusResult, ThreadForkWorktreeParams,
    ThreadForkWorktreeResult, ThreadRemoveWorktreeForkParams, ThreadRemoveWorktreeForkResult,
};

use crate::protocol_contract::{idle_thread_status, protocol_thread_from_metadata};
use crate::server::{AppServer, internal_error};

impl AppServer {
    pub(crate) async fn handle_thread_fork_worktree(
        &self,
        params: ThreadForkWorktreeParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let outcome = self
            .runtime
            .fork_thread_worktree(ForkThreadRequest {
                parent_thread_id: params.thread_id,
                name: params.name,
                from_turn_id: params.from_turn_id,
            })
            .await
            .map_err(internal_error)?;
        let fork = outcome.child.worktree_fork.clone().ok_or_else(|| {
            internal_error("forked thread is missing its worktree provenance")
        })?;
        let thread = protocol_thread_from_metadata(outcome.child, None, idle_thread_status());
        serde_json::to_value(ThreadForkWorktreeResult {
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
        let worktree_missing = metadata.worktree_fork.as_ref().is_some_and(|fork| {
            fork.status == WorktreeForkStatus::Active
                && !std::path::Path::new(&metadata.workspace).is_dir()
        });
        serde_json::to_value(ThreadForkStatusResult {
            thread_id: metadata.thread_id,
            parent_thread_id: metadata.parent_thread_id,
            forked_from_turn_id: metadata.forked_from_turn_id,
            fork: metadata.worktree_fork,
            worktree_missing,
        })
        .map_err(internal_error)
    }

    pub(crate) async fn handle_thread_remove_worktree_fork(
        &self,
        params: ThreadRemoveWorktreeForkParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let fork = self
            .runtime
            .remove_thread_worktree_fork(&params.thread_id, &params.confirm_path)
            .await
            .map_err(internal_error)?;
        serde_json::to_value(ThreadRemoveWorktreeForkResult {
            thread_id: params.thread_id,
            fork,
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
