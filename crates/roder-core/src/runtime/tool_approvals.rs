use super::*;

impl Runtime {
    pub(crate) async fn auto_resolve_pending_tool_approvals(&self) {
        let pending: Vec<_> = self
            .pending_tool_approvals
            .lock()
            .await
            .iter()
            .map(|(id, approval)| (id.clone(), approval.call.clone(), approval.context.clone()))
            .collect();
        let gate = DefaultPolicyGate::new();
        for (id, call, context) in pending {
            // A runtime mode change must not widen a child's independently
            // restricted permissions or skip an extension's explicit denial.
            let mode = self.effective_policy_mode_for_thread(&call.thread_id).await;
            let config = self.status().await;
            let mut ctx = context.unwrap_or_else(|| {
                self.tool_execution_context(
                    call.thread_id.clone(),
                    call.turn_id.clone(),
                    mode,
                    config.workspace.as_deref(),
                    Some(&config.command_shell),
                )
            });
            ctx.effective_mode = mode;
            if mode != PolicyMode::Bypass
                && !matches!(
                    gate.decide(&call, mode, &ctx),
                    PolicyDecision::AutoApproved { .. }
                )
            {
                continue;
            }
            let Ok(decision) = gate
                .decide_with_contributors(&call, mode, &ctx, &self.registry.policy_contributors)
                .await
            else {
                continue;
            };
            let approved = matches!(
                decision,
                PolicyDecision::AutoApproved { .. } | PolicyDecision::Allowed
            );
            if !(approved || matches!(decision, PolicyDecision::Denied { .. }))
                || self.effective_policy_mode_for_thread(&call.thread_id).await != mode
            {
                continue;
            }
            let Some(approval) = self.pending_tool_approvals.lock().await.remove(&id) else {
                continue;
            };
            self.emit(RoderEvent::PolicyDecisionRecorded(PolicyDecisionRecorded {
                thread_id: approval.thread_id.clone(),
                turn_id: approval.turn_id.clone(),
                tool_id: approval.tool_id.clone(),
                tool_name: approval.tool_name.clone(),
                mode,
                decision,
                timestamp: OffsetDateTime::now_utc(),
            }))
            .await;
            if approved && mode == PolicyMode::Bypass {
                self.emit(RoderEvent::PolicyBypassActive(PolicyBypassActive {
                    thread_id: approval.thread_id.clone(),
                    turn_id: approval.turn_id.clone(),
                    tool_id: approval.tool_id.clone(),
                    tool_name: approval.tool_name.clone(),
                    timestamp: OffsetDateTime::now_utc(),
                }))
                .await;
            }
            self.emit(RoderEvent::ApprovalResolved(ApprovalResolved {
                thread_id: approval.thread_id,
                turn_id: approval.turn_id,
                approval_id: id,
                tool_id: approval.tool_id,
                tool_name: approval.tool_name,
                approved,
                timestamp: OffsetDateTime::now_utc(),
            }))
            .await;
            let _ = approval.tx.send(approved);
        }
    }

    pub(crate) async fn request_tool_approval(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        call: &ToolCall,
        reason: Option<String>,
        context: &ToolExecutionContext,
    ) -> anyhow::Result<bool> {
        let approval_id = call.id.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.pending_tool_approvals.lock().await.insert(
            approval_id.clone(),
            crate::runtime::PendingToolApproval {
                thread_id: thread_id.clone(),
                turn_id: turn_id.clone(),
                tool_id: call.id.clone(),
                tool_name: call.name.clone(),
                call: call.clone(),
                context: Some(context.clone()),
                tx,
            },
        );
        // Close the race where permissions changed after policy review but
        // before the approval was registered. Full Access must not strand it.
        self.auto_resolve_pending_tool_approvals().await;
        if !self
            .pending_tool_approvals
            .lock()
            .await
            .contains_key(&approval_id)
        {
            return Ok(rx.await.unwrap_or(false));
        }
        let runtime_config = self.status().await;
        crate::hooks::run_lifecycle(
            self,
            thread_id,
            turn_id,
            runtime_config.workspace.as_deref(),
            "PermissionRequest",
            Some(&call.name),
            serde_json::json!({"toolName": call.name, "toolInput": call.arguments, "reason": reason}),
        )
        .await;
        self.emit(RoderEvent::ApprovalRequested(ApprovalRequested {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            approval_id,
            tool_id: call.id.clone(),
            tool_name: call.name.clone(),
            reason,
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        Ok(rx.await.unwrap_or(false))
    }

    pub async fn request_app_server_tool_approval(
        &self,
        call: ToolCall,
        reason: Option<String>,
    ) -> anyhow::Result<bool> {
        let approval_id = call.id.clone();
        let (tx, rx) = oneshot::channel();
        self.pending_tool_approvals.lock().await.insert(
            approval_id.clone(),
            PendingToolApproval {
                thread_id: call.thread_id.clone(),
                turn_id: call.turn_id.clone(),
                tool_id: call.id.clone(),
                tool_name: call.name.clone(),
                call: call.clone(),
                context: None,
                tx,
            },
        );
        self.auto_resolve_pending_tool_approvals().await;
        if !self
            .pending_tool_approvals
            .lock()
            .await
            .contains_key(&approval_id)
        {
            return Ok(rx.await.unwrap_or(false));
        }
        self.emit(RoderEvent::ApprovalRequested(ApprovalRequested {
            thread_id: call.thread_id.clone(),
            turn_id: call.turn_id.clone(),
            approval_id,
            tool_id: call.id.clone(),
            tool_name: call.name.clone(),
            reason,
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        Ok(rx.await.unwrap_or(false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::teams::{TeamMemberStartRequest, TeamStartRequest};

    fn call(thread: &str) -> ToolCall {
        ToolCall {
            id: "pending-permission".into(),
            name: "shell".into(),
            arguments: serde_json::json!({"command":"cargo test"}),
            raw_arguments: String::new(),
            thread_id: thread.into(),
            turn_id: "turn".into(),
        }
    }

    async fn pending_request(
        runtime: &Arc<Runtime>,
        thread: &str,
    ) -> tokio::task::JoinHandle<anyhow::Result<bool>> {
        let mut events = runtime.subscribe_events();
        let request = call(thread);
        let runtime = runtime.clone();
        let pending = tokio::spawn(async move {
            runtime
                .request_app_server_tool_approval(request, None)
                .await
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !matches!(
                events.recv().await.unwrap().event,
                RoderEvent::ApprovalRequested(_)
            ) {}
        })
        .await
        .unwrap();
        pending
    }

    #[tokio::test]
    async fn full_access_switch_resolves_pending_internal_tool() {
        let runtime = Arc::new(Runtime::fake().unwrap());
        let mut events = runtime.subscribe_events();
        let mut request = call("lead");
        request.name = "spawn_agent".into();
        let pending = {
            let runtime = runtime.clone();
            tokio::spawn(async move {
                runtime
                    .request_app_server_tool_approval(request, None)
                    .await
            })
        };
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !matches!(
                events.recv().await.unwrap().event,
                RoderEvent::ApprovalRequested(_)
            ) {}
        })
        .await
        .unwrap();
        runtime
            .set_policy_mode(PolicyMode::Bypass, None)
            .await
            .unwrap();
        assert!(pending.await.unwrap().unwrap());
    }

    #[tokio::test]
    async fn full_access_auto_resolution_does_not_elevate_restricted_child() {
        let data = tempfile::tempdir().unwrap();
        let mut builder = roder_api::extension::ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(crate::fake_provider::FakeInferenceEngine));
        let runtime = Arc::new(
            Runtime::new(
                builder.build().unwrap(),
                RuntimeConfig {
                    team_data_dir: Some(data.path().into()),
                    ..Default::default()
                },
            )
            .unwrap(),
        );
        let team = runtime
            .start_team(TeamStartRequest {
                lead_thread_id: None,
                display_mode: roder_api::teams::AgentTeamDisplayMode::InProcess,
                members: vec![TeamMemberStartRequest {
                    name: "Restricted".into(),
                    model_provider: None,
                    model: None,
                }],
            })
            .await
            .unwrap();
        let child = &team.members[1].thread_id;
        let pending = pending_request(&runtime, child).await;
        runtime
            .set_policy_mode(PolicyMode::Bypass, None)
            .await
            .unwrap();
        assert!(
            runtime
                .pending_tool_approvals
                .lock()
                .await
                .contains_key("pending-permission")
        );
        assert_eq!(
            runtime.effective_policy_mode_for_thread(child).await,
            PolicyMode::Default
        );
        runtime
            .resolve_tool_approval("pending-permission", false)
            .await
            .unwrap();
        assert!(!pending.await.unwrap().unwrap());
    }

    struct Deny;
    #[async_trait::async_trait]
    impl roder_api::context::PolicyContributor for Deny {
        fn id(&self) -> String {
            "mandatory-deny".into()
        }
        async fn review_tool(
            &self,
            _: roder_api::context::PolicyReview,
        ) -> anyhow::Result<roder_api::context::PolicyContribution> {
            Ok(roder_api::context::PolicyContribution::Deny {
                reason: "host restriction".into(),
            })
        }
    }

    #[tokio::test]
    async fn full_access_switch_rejects_pending_call_when_extension_denies() {
        let mut builder = roder_api::extension::ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(crate::fake_provider::FakeInferenceEngine));
        builder.policy_contributor(Arc::new(Deny));
        let runtime = Arc::new(Runtime::new(builder.build().unwrap(), Default::default()).unwrap());
        let pending = pending_request(&runtime, "lead").await;
        runtime
            .set_policy_mode(PolicyMode::Bypass, None)
            .await
            .unwrap();
        assert!(!pending.await.unwrap().unwrap());
    }
}
