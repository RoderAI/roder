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
        if !request.additional_directories.is_empty() {
            return Err(invalid_params(
                "additionalDirectories are not advertised by Roder ACP",
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
        let instructions = super::config::host_instructions(request.meta.as_ref())?;
        if instructions.is_some() && instructions != thread.developer_instructions {
            return Err(invalid_params(
                "host instructions differ from the stored session; start a new session",
            ));
        }
        let configuration = self.session_configuration(&id).await?;
        let mcp =
            Arc::new(super::mcp::SessionMcp::connect(request.mcp_servers, &request.cwd).await?);
        let prepared: Result<(), JsonRpcError> = async {
            let mut stored_tools = thread.external_tools.clone();
            stored_tools.sort_by(|a, b| a.name.cmp(&b.name));
            if mcp.specs() != stored_tools {
                return Err(invalid_params(
                    "session MCP tool bindings differ from the stored session",
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
            if let Some(existing) = existing.as_ref() {
                existing.mcp.close().await?;
            }
            Ok(())
        }
        .await;
        if let Err(error) = prepared {
            let _ = mcp.close().await;
            return Err(error);
        }
        self.sessions.lock().await.insert(
            id.clone(),
            AcpSession {
                thread_id: id.clone(),
                active: Arc::new(Mutex::new(())),
                mcp,
            },
        );
        Ok(configuration)
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
        session.mcp.close().await?;
        self.sessions.lock().await.remove(&id);
        Ok(serde_json::json!({}))
    }
}
