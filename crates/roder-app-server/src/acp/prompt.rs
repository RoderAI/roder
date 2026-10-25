use super::*;

impl<C: AppClient> AcpAdapter<C> {
    pub(super) async fn handle_session_prompt<P>(
        &self,
        params: Option<serde_json::Value>,
        peer: &P,
    ) -> Result<serde_json::Value, JsonRpcError>
    where
        P: AcpClientPeer,
    {
        let admission = self.admission.lock().await;
        let request: acp::PromptRequest = decode_params(params)?;
        let session_id = request.session_id.to_string();
        let session = self
            .sessions
            .lock()
            .await
            .get(&session_id)
            .cloned()
            .ok_or_else(|| invalid_params(format!("unknown ACP session {session_id:?}")))?;
        let _active = session
            .active
            .clone()
            .try_lock_owned()
            .map_err(|_| invalid_params("ACP session already has an active operation"))?;
        let input = prompt_blocks_to_turn_input(request.prompt)?;
        let mut notifications = self.client.subscribe_notifications();
        let started: TurnStartResult = self
            .call(
                "turn/start",
                serde_json::to_value(TurnStartParams {
                    thread_id: session.thread_id.clone(),
                    input,
                    prompt: None,
                    model_provider: None,
                    model: None,
                    reasoning: None,
                    service_tier: None,
                    developer_context: None,
                    mcp_auth_token: None,
                    policy_mode: None,
                    task_ledger_required: false,
                })
                .map_err(internal_error)?,
            )
            .await?;

        drop(admission);
        use futures::StreamExt;
        let mut approvals: futures::stream::FuturesUnordered<
            futures::future::BoxFuture<'_, Result<(), JsonRpcError>>,
        > = futures::stream::FuturesUnordered::new();
        loop {
            let notification = tokio::select! {
                result = approvals.next(), if !approvals.is_empty() => {
                    result.expect("pending permission result")?;
                    continue;
                }
                notification = notifications.recv() => notification.map_err(internal_error)?,
            };
            if !notification_matches_turn(&notification, &session.thread_id, &started.turn_id) {
                continue;
            }
            if let Some(update) = acp_session_update(&session_id, &notification) {
                peer.send_notification(update)
                    .await
                    .map_err(internal_error)?;
                continue;
            }
            if notification.method == "thread/approvalRequested" {
                let session_id = session_id.clone();
                approvals.push(Box::pin(async move {
                    self.handle_approval_request(&session_id, &notification, peer)
                        .await
                }));
                continue;
            }
            if notification.method == "turn/completed" {
                let stop_reason = turn_completed_stop_reason(&notification);
                return serde_json::to_value(acp::PromptResponse::new(stop_reason))
                    .map_err(internal_error);
            }
        }
    }

    async fn handle_approval_request<P>(
        &self,
        session_id: &str,
        notification: &JsonRpcNotification,
        peer: &P,
    ) -> Result<(), JsonRpcError>
    where
        P: AcpClientPeer,
    {
        notification_string(notification, "threadId")?;
        let approval_id = notification_string(notification, "approvalId")?;
        let tool_id = notification_string(notification, "toolId")?;
        let tool_name = notification_string(notification, "toolName")?;
        let reason = notification
            .params
            .get("reason")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("Roder needs permission to run this tool.");
        let content = vec![acp::ToolCallContent::from(reason.to_string())];
        let tool_call = acp::ToolCallUpdate::new(
            tool_id,
            acp::ToolCallUpdateFields::new()
                .title(tool_name.clone())
                .kind(tool_kind_for_name(&tool_name))
                .status(acp::ToolCallStatus::Pending)
                .content(content),
        );
        let response = peer
            .request_permission(acp::RequestPermissionRequest::new(
                session_id.to_string(),
                tool_call,
                vec![
                    acp::PermissionOption::new(
                        "allow_once",
                        "Allow once",
                        acp::PermissionOptionKind::AllowOnce,
                    ),
                    acp::PermissionOption::new(
                        "reject_once",
                        "Reject once",
                        acp::PermissionOptionKind::RejectOnce,
                    ),
                ],
            ))
            .await
            .map_err(internal_error)?;
        let approved = matches!(
            response.outcome,
            acp::RequestPermissionOutcome::Selected(ref selected)
                if selected.option_id.to_string() == "allow_once"
        );
        let _: serde_json::Value = self
            .call(
                "thread/resolve_approval",
                serde_json::to_value(ThreadResolveApprovalParams {
                    approval_id,
                    approved,
                })
                .map_err(internal_error)?,
            )
            .await?;
        Ok(())
    }
}
