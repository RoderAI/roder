//! `/fork` slash command: conversation forks backed by workspace fork
//! providers (roadmap phases 90 + 81; `git-worktree` by default) — these
//! are Roder conversation/workspace forks, not GitHub repository forks.

use roder_app_server::AppClient;
use roder_protocol::{
    JsonRpcRequest, ThreadForkParams, ThreadForkResult, ThreadForkStatusParams,
    ThreadForkStatusResult, ThreadRemoveForkParams, ThreadRemoveForkResult,
};

use super::{TuiApp, decode_response, truncate};

impl<C> TuiApp<C>
where
    C: AppClient,
{
    pub(super) async fn run_fork_slash_command(&mut self, args: &str) {
        let args = args.trim();
        if args == "status" || args.is_empty() {
            self.show_fork_status().await;
            self.push_event("slash command: /fork status".to_string());
            return;
        }
        if let Some(rest) = args.strip_prefix("remove") {
            self.remove_fork_workspace(rest.trim()).await;
            self.push_event("slash command: /fork remove".to_string());
            return;
        }
        self.fork_current_thread(args).await;
        self.push_event(format!("slash command: /fork {args}"));
    }

    async fn fork_current_thread(&mut self, name: &str) {
        let params = ThreadForkParams {
            thread_id: self.thread_id.clone(),
            name: name.to_string(),
            from_turn_id: None,
            provider: None,
            provider_config: None,
        };
        let result = fork_thread(&self.client, params).await;
        match result {
            Ok(forked) => {
                let child_id = forked.thread.id.clone();
                // Switch first: load_thread resets the timeline, so the fork
                // summary is pushed afterwards to stay visible.
                self.load_thread(child_id.clone()).await;
                let mut lines = vec![format!(
                    "Forked conversation into {} ({} fork at {}).",
                    truncate(&child_id, 12),
                    forked.fork.provider_id,
                    forked.fork.workspace.display()
                )];
                lines.extend(
                    forked
                        .warnings
                        .iter()
                        .map(|warning| format!("warning: {warning}")),
                );
                lines.push(
                    "Tool writes now happen in the fork workspace; /fork remove <workspace-path> \
                     cleans it up."
                        .to_string(),
                );
                self.timeline.push_system(lines.join("\n"));
            }
            Err(err) => self.record_error(format!("thread/fork failed: {err}")),
        }
    }

    async fn show_fork_status(&mut self) {
        let params = ThreadForkStatusParams {
            thread_id: self.thread_id.clone(),
        };
        match fork_status(&self.client, params).await {
            Ok(status) => {
                let text = match (&status.fork, &status.parent_thread_id) {
                    (Some(fork), parent) => {
                        let mut lines = vec![format!(
                            "{} fork ({:?}) at {}",
                            fork.provider_id,
                            fork.status,
                            fork.workspace.display()
                        )];
                        if let Some(parent) = parent {
                            lines.push(format!("Forked from thread {}", truncate(parent, 12)));
                        }
                        if status.workspace_missing {
                            lines.push(
                                "warning: the fork workspace is missing; restore it or run \
                                 /fork remove <workspace-path>"
                                    .to_string(),
                            );
                        }
                        lines.join("\n")
                    }
                    (None, Some(parent)) => {
                        format!(
                            "This thread was forked from {} but has no workspace fork.",
                            truncate(parent, 12)
                        )
                    }
                    (None, None) => {
                        "This thread is not a fork. Use /fork <name> to fork the conversation \
                         into an isolated workspace fork (not a GitHub repository fork)."
                            .to_string()
                    }
                };
                self.timeline.push_system(text);
            }
            Err(err) => self.record_error(format!("thread/fork_status failed: {err}")),
        }
    }

    async fn remove_fork_workspace(&mut self, confirm_path: &str) {
        if confirm_path.is_empty() {
            self.timeline.push_system(
                "Removal is destructive and path-confirmed: run /fork status to see the exact \
                 workspace path, then /fork remove <workspace-path>."
                    .to_string(),
            );
            return;
        }
        let params = ThreadRemoveForkParams {
            thread_id: self.thread_id.clone(),
            confirm_path: confirm_path.to_string(),
        };
        match remove_thread_fork(&self.client, params).await {
            Ok(removed) => {
                self.timeline.push_system(format!(
                    "Removed fork workspace {} ({} provenance kept).",
                    removed.fork.workspace.display(),
                    removed.fork.provenance.branch.as_deref().unwrap_or("-")
                ));
            }
            Err(err) => self.record_error(format!("thread/remove_fork failed: {err}")),
        }
    }
}

async fn fork_thread<C: AppClient>(
    client: &C,
    params: ThreadForkParams,
) -> anyhow::Result<ThreadForkResult> {
    let response = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("thread/fork")),
            method: "thread/fork".to_string(),
            params: Some(serde_json::to_value(params)?),
        })
        .await;
    decode_response(response)
}

async fn fork_status<C: AppClient>(
    client: &C,
    params: ThreadForkStatusParams,
) -> anyhow::Result<ThreadForkStatusResult> {
    let response = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("thread/fork_status")),
            method: "thread/fork_status".to_string(),
            params: Some(serde_json::to_value(params)?),
        })
        .await;
    decode_response(response)
}

async fn remove_thread_fork<C: AppClient>(
    client: &C,
    params: ThreadRemoveForkParams,
) -> anyhow::Result<ThreadRemoveForkResult> {
    let response = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("thread/remove_fork")),
            method: "thread/remove_fork".to_string(),
            params: Some(serde_json::to_value(params)?),
        })
        .await;
    decode_response(response)
}
