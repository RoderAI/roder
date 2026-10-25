use super::*;
use roder_protocol::{ThreadReadParams, ThreadReadResult};

impl<C: AppClient> AcpAdapter<C> {
    pub(super) async fn handle_session_reopen<P: AcpClientPeer>(
        &self,
        params: Option<serde_json::Value>,
        peer: &P,
        replay: bool,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let _admission = self.admission.lock().await;
        let request: acp::LoadSessionRequest = decode_params(params)?;
        if !request.cwd.is_absolute() {
            return Err(invalid_params("session cwd must be absolute"));
        }
        if !request.additional_directories.is_empty() || !request.mcp_servers.is_empty() {
            return Err(invalid_params(
                "additionalDirectories and mcpServers are not advertised by Roder ACP",
            ));
        }
        self.require_authentication().await?;
        let id = request.session_id.to_string();
        if id.contains('/') || id.contains('\\') || id == "." || id == ".." {
            return Err(invalid_params("invalid ACP session id"));
        }
        let existing = self.sessions.lock().await.get(&id).cloned();
        let _idle =
            match &existing {
                Some(session) => {
                    Some(session.active.clone().try_lock_owned().map_err(|_| {
                        invalid_params("ACP session already has an active operation")
                    })?)
                }
                None => None,
            };
        let result: ThreadReadResult = self
            .call(
                "thread/read",
                serde_json::to_value(ThreadReadParams {
                    thread_id: id.clone(),
                    include_turns: replay,
                })
                .map_err(internal_error)?,
            )
            .await?;
        let thread = result.thread.ok_or_else(|| JsonRpcError {
            code: -32002,
            message: format!("ACP session not found: {id}"),
            data: None,
        })?;
        let cwd = std::fs::canonicalize(&request.cwd).map_err(invalid_params)?;
        let stored_cwd = std::fs::canonicalize(&thread.cwd).map_err(invalid_params)?;
        if cwd != stored_cwd {
            return Err(invalid_params(
                "session cwd does not match its stored workspace",
            ));
        }
        if thread.status.active_turn_id.is_some() {
            return Err(invalid_params(
                "cannot reopen a session with active or recovering work",
            ));
        }
        if !thread.external_tools.is_empty() {
            return Err(invalid_params(
                "session requires native host external tools",
            ));
        }
        if replay {
            for turn in thread.turns.unwrap_or_default() {
                for item in turn.items {
                    for update in super::translation::replay_item(&item) {
                        peer.send_notification(JsonRpcNotification {
                            jsonrpc: "2.0".into(),
                            method: "session/update".into(),
                            params: serde_json::to_value(acp::SessionNotification::new(
                                id.clone(),
                                update,
                            ))
                            .map_err(internal_error)?,
                        })
                        .await
                        .map_err(internal_error)?;
                    }
                }
            }
        }
        self.sessions
            .lock()
            .await
            .entry(id.clone())
            .or_insert_with(|| AcpSession {
                thread_id: id,
                active: Arc::new(Mutex::new(())),
            });
        Ok(serde_json::json!({}))
    }

    pub(super) async fn handle_session_close(
        &self,
        params: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let _admission = self.admission.lock().await;
        let request: acp::CloseSessionRequest = decode_params(params)?;
        let id = request.session_id.to_string();
        let session = self
            .sessions
            .lock()
            .await
            .get(&id)
            .cloned()
            .ok_or_else(|| JsonRpcError {
                code: -32002,
                message: format!("ACP session is not active: {id}"),
                data: None,
            })?;
        let _: serde_json::Value = self
            .call(
                "thread/unload",
                serde_json::json!({"threadId": session.thread_id}),
            )
            .await?;
        // A prompt can be sending its final updates after native cleanup has finished.
        let _idle = tokio::time::timeout(std::time::Duration::from_secs(30), session.active.lock())
            .await
            .map_err(internal_error)?;
        self.sessions.lock().await.remove(&id);
        Ok(serde_json::json!({}))
    }
}
