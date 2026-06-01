use std::sync::Arc;

use roder_api::ToolSchemaPolicy;
use roder_api::artifacts::{ContextArtifactKind, format_artifact_reference};
use roder_api::events::*;
use roder_api::policy_mode::{PolicyDecision, PolicyMode};
use roder_api::subagents::SubagentExitReason;
use roder_api::tools::ToolCall;
use roder_api::tools::ToolResult;
use roder_api::transcript::{ToolResultRecord, tool_display_payload};
use serde_json::Value;
use time::OffsetDateTime;

use crate::policy_gate::DefaultPolicyGate;
use crate::runtime::{PendingPlanExit, Runtime};
use crate::tool_output::{
    artifact_backed_tool_output, cap_tool_output_lines, should_spill_tool_output,
};
use crate::tool_preview::file_change_preview;
use crate::tool_validation::{
    emit_tool_validation_recorded, validate_tool_call_arguments, validation_error_tool_result,
};
use roder_api::artifacts::CreateArtifactRequest;

impl Runtime {
    pub(crate) async fn route_tool_call(
        self: &Arc<Self>,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        mut call: roder_api::inference::ToolCallCompleted,
        workspace: Option<&str>,
        deadline: Option<OffsetDateTime>,
    ) -> anyhow::Result<ToolResultRecord> {
        let mut parsed_args: Value = serde_json::from_str(&call.arguments)
            .unwrap_or_else(|_| serde_json::json!({ "raw": call.arguments }));
        if is_subagent_task_tool(&call.name) {
            if let Some(deadline) = deadline {
                let remaining =
                    crate::runtime::deadline_remaining_seconds(Some(deadline)).unwrap_or_default();
                if remaining <= crate::runtime::MIN_CHILD_DEADLINE_SECONDS {
                    let item = ToolResultRecord {
                        id: call.id.clone(),
                        name: Some(call.name.clone()),
                        result: format!(
                            "deadline policy skipped subagent work: {remaining}s remaining"
                        ),
                        display_payload: tool_display_payload(
                            Some(&call.name),
                            Some(&parsed_args),
                            None,
                        ),
                        is_error: true,
                    };
                    self.persist_turn_item(
                        thread_id,
                        turn_id,
                        &roder_api::transcript::TranscriptItem::ToolResult(item.clone()),
                    )
                    .await?;
                    self.emit(RoderEvent::ToolCallCompleted(ToolCallCompleted {
                        thread_id: thread_id.clone(),
                        turn_id: turn_id.clone(),
                        tool_id: call.id,
                        tool_name: item.name.clone(),
                        display_payload: item.display_payload.clone(),
                        is_error: true,
                        output: Some(item.result.clone()),
                        timestamp: OffsetDateTime::now_utc(),
                    }))
                    .await;
                    return Ok(item);
                }
                if let Some(object) = parsed_args.as_object_mut() {
                    object
                        .entry("parent_deadline_seconds".to_string())
                        .or_insert_with(|| serde_json::json!(remaining));
                    call.arguments = serde_json::to_string(&parsed_args)?;
                }
            }
        }
        self.emit(RoderEvent::ToolCallRequested(ToolCallRequested {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            tool_id: call.id.clone(),
            tool_name: call.name.clone(),
            display_payload: tool_display_payload(Some(&call.name), Some(&parsed_args), None),
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        self.emit(crate::retrieval_metrics::route_choice_event(
            thread_id,
            turn_id,
            &call.name,
            &parsed_args,
        ))
        .await;
        let Some(executor) = self.tool_registry.get(&call.name) else {
            emit_tool_validation_recorded(
                self,
                thread_id,
                turn_id,
                &call.id,
                &call.name,
                ToolCallValidationFailureClass::UnknownTool,
                ToolCallValidationRepairStatus::NotNeeded,
                "tool is not registered".to_string(),
            )
            .await;
            let item = ToolResultRecord {
                id: call.id.clone(),
                name: Some(call.name),
                result: "tool not found".to_string(),
                display_payload: tool_display_payload(None, Some(&parsed_args), None),
                is_error: true,
            };
            self.persist_turn_item(
                thread_id,
                turn_id,
                &roder_api::transcript::TranscriptItem::ToolResult(item.clone()),
            )
            .await?;
            self.emit(RoderEvent::ToolCallCompleted(ToolCallCompleted {
                thread_id: thread_id.clone(),
                turn_id: turn_id.clone(),
                tool_id: call.id,
                tool_name: item.name.clone(),
                display_payload: item.display_payload.clone(),
                is_error: true,
                output: Some(item.result.clone()),
                timestamp: OffsetDateTime::now_utc(),
            }))
            .await;
            self.emit(crate::retrieval_metrics::route_failed_event(
                thread_id,
                turn_id,
                item.name.as_deref().unwrap_or("unknown"),
                "tool is not registered",
            ))
            .await;
            self.emit(crate::retrieval_metrics::unknown_tool_result_event(
                thread_id,
                turn_id,
                item.name.as_deref().unwrap_or("unknown"),
            ))
            .await;
            return Ok(item);
        };
        let spec = executor
            .spec()
            .normalized_for_model(ToolSchemaPolicy::strict());
        let arguments = match validate_tool_call_arguments(
            &call.arguments,
            &spec,
            thread_id,
            turn_id,
            &call.id,
            self,
        )
        .await
        {
            Ok(arguments) => arguments,
            Err(error) => {
                let item = validation_error_tool_result(&call.id, &call.name, &parsed_args, error);
                self.persist_turn_item(
                    thread_id,
                    turn_id,
                    &roder_api::transcript::TranscriptItem::ToolResult(item.clone()),
                )
                .await?;
                self.emit(RoderEvent::ToolCallCompleted(ToolCallCompleted {
                    thread_id: thread_id.clone(),
                    turn_id: turn_id.clone(),
                    tool_id: call.id,
                    tool_name: item.name.clone(),
                    display_payload: item.display_payload.clone(),
                    is_error: true,
                    output: Some(item.result.clone()),
                    timestamp: OffsetDateTime::now_utc(),
                }))
                .await;
                self.emit(crate::retrieval_metrics::route_failed_event(
                    thread_id,
                    turn_id,
                    &call.name,
                    "tool argument validation failed",
                ))
                .await;
                return Ok(item);
            }
        };
        let mut runtime_config = self.status().await;
        let mode = self.effective_policy_mode_for_thread(thread_id).await;
        runtime_config.policy_mode = mode;
        let tool_call = ToolCall {
            id: call.id.clone(),
            name: call.name.clone(),
            arguments,
            raw_arguments: call.arguments.clone(),
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
        };
        let mut ctx = self.tool_execution_context(
            thread_id.clone(),
            turn_id.clone(),
            mode,
            workspace.or(runtime_config.workspace.as_deref()),
            Some(&runtime_config.command_shell),
        );
        if let Some(remaining) = crate::runtime::deadline_remaining_seconds(deadline) {
            ctx = ctx.with_deadline_remaining_seconds(remaining);
        }
        let decision = DefaultPolicyGate::new()
            .decide_with_contributors(&tool_call, mode, &ctx, &self.registry.policy_contributors)
            .await?;
        self.emit_policy_decision(thread_id, turn_id, &tool_call, mode, decision.clone())
            .await;
        if matches!(decision, PolicyDecision::AutoApproved { .. }) && mode == PolicyMode::Bypass {
            self.emit(RoderEvent::PolicyBypassActive(PolicyBypassActive {
                thread_id: thread_id.clone(),
                turn_id: turn_id.clone(),
                tool_id: tool_call.id.clone(),
                tool_name: tool_call.name.clone(),
                timestamp: OffsetDateTime::now_utc(),
            }))
            .await;
        }
        if let PolicyDecision::Denied { reason } = decision {
            if let Some(event) = crate::plan_review::plan_review_for_blocked_tool(
                thread_id, turn_id, &tool_call, mode,
            ) {
                self.emit(event).await;
            }
            let item = ToolResultRecord {
                id: tool_call.id.clone(),
                name: Some(tool_call.name.clone()),
                result: format!("policy denied tool call: {reason}"),
                display_payload: tool_display_payload(
                    Some(&tool_call.name),
                    Some(&parsed_args),
                    None,
                ),
                is_error: true,
            };
            self.persist_turn_item(
                thread_id,
                turn_id,
                &roder_api::transcript::TranscriptItem::ToolResult(item.clone()),
            )
            .await?;
            self.emit(RoderEvent::ToolCallCompleted(ToolCallCompleted {
                thread_id: thread_id.clone(),
                turn_id: turn_id.clone(),
                tool_id: tool_call.id,
                tool_name: item.name.clone(),
                display_payload: item.display_payload.clone(),
                is_error: true,
                output: Some(item.result.clone()),
                timestamp: OffsetDateTime::now_utc(),
            }))
            .await;
            self.emit(crate::retrieval_metrics::route_failed_event(
                thread_id,
                turn_id,
                &tool_call.name,
                "policy denied tool call",
            ))
            .await;
            return Ok(item);
        }

        let preview = file_change_preview(
            &tool_call,
            workspace.or(runtime_config.workspace.as_deref()),
        );
        if let Some(preview) = preview.clone() {
            self.emit(RoderEvent::FileChangePreviewReady(preview)).await;
        }

        if let PolicyDecision::RequiresApproval { reason } = &decision
            && !self
                .request_tool_approval(thread_id, turn_id, &tool_call, reason.clone())
                .await?
        {
            let item = ToolResultRecord {
                id: tool_call.id.clone(),
                name: Some(tool_call.name.clone()),
                result: "policy rejected tool call: approval denied".to_string(),
                display_payload: tool_display_payload(
                    Some(&tool_call.name),
                    Some(&parsed_args),
                    None,
                ),
                is_error: true,
            };
            self.persist_turn_item(
                thread_id,
                turn_id,
                &roder_api::transcript::TranscriptItem::ToolResult(item.clone()),
            )
            .await?;
            self.emit(RoderEvent::ToolCallCompleted(ToolCallCompleted {
                thread_id: thread_id.clone(),
                turn_id: turn_id.clone(),
                tool_id: tool_call.id,
                tool_name: item.name.clone(),
                display_payload: item.display_payload.clone(),
                is_error: true,
                output: Some(item.result.clone()),
                timestamp: OffsetDateTime::now_utc(),
            }))
            .await;
            self.emit(crate::retrieval_metrics::route_failed_event(
                thread_id,
                turn_id,
                &tool_call.name,
                "approval denied",
            ))
            .await;
            return Ok(item);
        }
        ctx.effective_mode = self.effective_policy_mode_for_thread(thread_id).await;
        let workspace_change_baseline =
            crate::workspace_changes::WorkspaceChangeBaseline::capture_for_tool(
                &tool_call,
                workspace.or(runtime_config.workspace.as_deref()),
                self.registry.version_control_resolver(),
            )
            .await;

        self.emit(RoderEvent::ToolCallStarted(ToolCallStarted {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            tool_id: tool_call.id.clone(),
            tool_name: Some(tool_call.name.clone()),
            display_payload: tool_display_payload(
                Some(&tool_call.name),
                Some(&tool_call.arguments),
                None,
            ),
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        let result = if crate::agent_control_tools::is_agent_control_tool(&tool_call.name) {
            self.execute_agent_control_tool(thread_id, turn_id, &call, tool_call.arguments.clone())
                .await
        } else {
            match executor.execute(ctx, tool_call.clone()).await {
                Ok(result) => result,
                Err(err) => ToolResult {
                    id: tool_call.id.clone(),
                    name: tool_call.name.clone(),
                    text: err.to_string(),
                    data: serde_json::json!({
                        "error": {
                            "kind": "tool_execution_failed",
                            "message": err.to_string(),
                        }
                    }),
                    is_error: true,
                },
            }
        };
        let result = self
            .resolve_user_input_request(thread_id, turn_id, result)
            .await?;
        self.emit_subagent_events(thread_id, turn_id, &parsed_args, &result)
            .await;
        self.emit_policy_exit_plan_request(thread_id, turn_id, &result)
            .await;
        self.emit_hunk_records(&result).await;
        self.emit_workspace_change_observed(
            thread_id,
            turn_id,
            &tool_call,
            &result,
            workspace_change_baseline,
        )
        .await;
        self.emit_media_artifacts(thread_id, turn_id, &result).await;
        self.emit_task_ledger_update(thread_id, turn_id, &result)
            .await;
        self.emit_verification_result(thread_id, turn_id, &result)
            .await;
        let raw_text = result.text;
        let original_line_count = raw_text.lines().count() as u64;
        let original_char_count = raw_text.chars().count() as u64;
        let artifact_backed =
            runtime_config.file_backed_dynamic_context && should_spill_tool_output(&raw_text);
        let item_result = if artifact_backed {
            let artifact = self.context_artifacts().create(CreateArtifactRequest {
                kind: ContextArtifactKind::ToolOutput,
                thread_id,
                turn_id,
                source_tool_id: Some(&result.id),
                label: Some(&result.name),
                bytes: raw_text.as_bytes(),
            })?;
            let reference = format_artifact_reference(&artifact, &result.name);
            let inline = artifact_backed_tool_output(&raw_text, &reference, &result.name);
            self.emit(RoderEvent::ContextArtifactCreated(ContextArtifactCreated {
                thread_id: thread_id.clone(),
                turn_id: turn_id.clone(),
                artifact: artifact.clone(),
                timestamp: OffsetDateTime::now_utc(),
            }))
            .await;
            self.emit(RoderEvent::ContextArtifactCapped(ContextArtifactCapped {
                thread_id: thread_id.clone(),
                turn_id: turn_id.clone(),
                artifact_id: artifact.id,
                inline_byte_count: inline.len() as u64,
                original_byte_count: raw_text.len() as u64,
                timestamp: OffsetDateTime::now_utc(),
            }))
            .await;
            inline
        } else {
            cap_tool_output_lines(raw_text)
        };
        let inline_char_count = item_result.chars().count() as u64;
        if artifact_backed || inline_char_count < original_char_count {
            self.emit(RoderEvent::ToolOutputTruncated(ToolOutputTruncated {
                thread_id: thread_id.clone(),
                turn_id: turn_id.clone(),
                tool_id: result.id.clone(),
                tool_name: Some(result.name.clone()),
                original_line_count,
                original_char_count,
                inline_char_count,
                artifact_backed,
                timestamp: OffsetDateTime::now_utc(),
            }))
            .await;
        }
        let item = ToolResultRecord {
            id: result.id.clone(),
            name: Some(result.name.clone()),
            result: item_result,
            display_payload: tool_display_payload(
                Some(&result.name),
                Some(&parsed_args),
                Some(&result.data),
            ),
            is_error: result.is_error,
        };
        self.persist_turn_item(
            thread_id,
            turn_id,
            &roder_api::transcript::TranscriptItem::ToolResult(item.clone()),
        )
        .await?;
        self.emit(RoderEvent::ToolCallCompleted(ToolCallCompleted {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            tool_id: result.id,
            tool_name: Some(result.name.clone()),
            display_payload: item.display_payload.clone(),
            is_error: item.is_error,
            output: Some(item.result.clone()),
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        if let Some(event) = crate::retrieval_metrics::result_used_event(
            thread_id,
            turn_id,
            &result.name,
            item.display_payload
                .as_ref()
                .unwrap_or(&serde_json::Value::Null),
            &item.result,
            item.is_error,
        ) {
            self.emit(event).await;
        }
        Ok(item)
    }

    async fn emit_policy_decision(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        call: &ToolCall,
        mode: PolicyMode,
        decision: PolicyDecision,
    ) {
        self.emit(RoderEvent::PolicyDecisionRecorded(PolicyDecisionRecorded {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            tool_id: call.id.clone(),
            tool_name: call.name.clone(),
            mode,
            decision,
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
    }

    async fn emit_hunk_records(&self, result: &ToolResult) {
        let Some(value) = result.data.get("hunks") else {
            return;
        };
        let Ok(hunks) =
            serde_json::from_value::<Vec<roder_api::plan_review::HunkRecord>>(value.clone())
        else {
            return;
        };
        for hunk in hunks {
            self.emit(RoderEvent::HunkRecorded(HunkRecorded {
                hunk,
                timestamp: OffsetDateTime::now_utc(),
            }))
            .await;
        }
    }

    async fn emit_workspace_change_observed(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        call: &ToolCall,
        result: &ToolResult,
        baseline: Option<crate::workspace_changes::WorkspaceChangeBaseline>,
    ) {
        if result.data.get("hunks").is_some() {
            return;
        }
        let Some(baseline) = baseline else {
            return;
        };
        let Some(change) = baseline.observed_after(thread_id, turn_id, call).await else {
            return;
        };
        self.emit(RoderEvent::WorkspaceChangeObserved(
            WorkspaceChangeObserved {
                change,
                timestamp: OffsetDateTime::now_utc(),
            },
        ))
        .await;
    }

    async fn emit_media_artifacts(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        result: &ToolResult,
    ) {
        let Some(value) = result.data.get("mediaArtifact") else {
            return;
        };
        let Ok(artifact) = serde_json::from_value::<roder_api::media::MediaArtifact>(value.clone())
        else {
            return;
        };
        self.emit(RoderEvent::MediaArtifactCreated(
            roder_api::events::MediaArtifactCreated {
                thread_id: thread_id.clone(),
                turn_id: turn_id.clone(),
                artifact: artifact.clone(),
                timestamp: OffsetDateTime::now_utc(),
            },
        ))
        .await;
        if let Ok(preview) = serde_json::from_value::<roder_api::media::MediaPreview>(
            result.data.get("mediaPreview").cloned().unwrap_or_default(),
        ) {
            self.emit(RoderEvent::MediaPreviewReady(
                roder_api::events::MediaPreviewReady {
                    thread_id: thread_id.clone(),
                    turn_id: turn_id.clone(),
                    preview,
                    timestamp: OffsetDateTime::now_utc(),
                },
            ))
            .await;
        }
    }

    async fn emit_task_ledger_update(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        result: &ToolResult,
    ) {
        let Some(value) = result.data.get("taskLedger") else {
            return;
        };
        let Ok(snapshot) =
            serde_json::from_value::<roder_api::task_ledger::TaskLedgerSnapshot>(value.clone())
        else {
            return;
        };
        self.emit(RoderEvent::TaskLedgerUpdated(TaskLedgerUpdated {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            completed_count: snapshot.completed_count() as u64,
            tasks: snapshot.tasks,
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
    }

    async fn emit_verification_result(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        result: &ToolResult,
    ) {
        let Some(value) = result.data.get("verification") else {
            return;
        };
        let status = value
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let changed_files = json_string_array(value.get("changedFiles"));
        let tool_evidence = json_string_array(value.get("toolEvidence"));
        let tests_run = json_string_array(value.get("testsRun"));
        let open_gaps = json_string_array(value.get("openGaps"));
        match status {
            "completed" | "failed" => {
                self.emit(RoderEvent::VerificationCompleted(VerificationCompleted {
                    thread_id: thread_id.clone(),
                    turn_id: turn_id.clone(),
                    passed: status == "completed",
                    changed_files,
                    tool_evidence,
                    tests_run,
                    open_gaps,
                    timestamp: OffsetDateTime::now_utc(),
                }))
                .await;
            }
            "skipped" => {
                let reason = value
                    .get("skipReason")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                self.emit(RoderEvent::VerificationSkipped(VerificationSkipped {
                    thread_id: thread_id.clone(),
                    turn_id: turn_id.clone(),
                    reason,
                    timestamp: OffsetDateTime::now_utc(),
                }))
                .await;
            }
            _ => {}
        }
    }

    async fn request_tool_approval(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        call: &ToolCall,
        reason: Option<String>,
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
                tx,
            },
        );
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

    async fn emit_subagent_events(
        &self,
        parent_thread_id: &ThreadId,
        parent_turn_id: &TurnId,
        arguments: &Value,
        result: &ToolResult,
    ) {
        let data = &result.data;
        let Some(child_thread_id) = data.get("thread_id").and_then(Value::as_str) else {
            return;
        };
        let Some(child_turn_id) = data.get("turn_id").and_then(Value::as_str) else {
            return;
        };
        let agent_type = data
            .get("agent_type")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let description = arguments
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let model = data
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_string);

        self.emit(RoderEvent::SubagentStarted(SubagentStarted {
            thread_id: child_thread_id.to_string(),
            turn_id: child_turn_id.to_string(),
            parent_thread_id: parent_thread_id.clone(),
            parent_turn_id: parent_turn_id.clone(),
            agent_type: agent_type.clone(),
            description,
            model,
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;

        if !result.text.is_empty() {
            self.emit(RoderEvent::SubagentMessage(SubagentMessage {
                thread_id: child_thread_id.to_string(),
                turn_id: child_turn_id.to_string(),
                parent_thread_id: parent_thread_id.clone(),
                parent_turn_id: parent_turn_id.clone(),
                agent_type: agent_type.clone(),
                text: result.text.clone(),
                timestamp: OffsetDateTime::now_utc(),
            }))
            .await;
        }

        let exit_reason = data
            .get("exit_reason")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or({
                if result.is_error {
                    SubagentExitReason::Failed
                } else {
                    SubagentExitReason::Completed
                }
            });
        if result.is_error {
            self.emit(RoderEvent::SubagentFailed(SubagentFailed {
                thread_id: child_thread_id.to_string(),
                turn_id: child_turn_id.to_string(),
                parent_thread_id: parent_thread_id.clone(),
                parent_turn_id: parent_turn_id.clone(),
                agent_type,
                error: subagent_error_kind(data),
                timestamp: OffsetDateTime::now_utc(),
            }))
            .await;
        } else {
            self.emit(RoderEvent::SubagentCompleted(SubagentCompleted {
                thread_id: child_thread_id.to_string(),
                turn_id: child_turn_id.to_string(),
                parent_thread_id: parent_thread_id.clone(),
                parent_turn_id: parent_turn_id.clone(),
                agent_type,
                exit_reason,
                timestamp: OffsetDateTime::now_utc(),
            }))
            .await;
        }
    }

    async fn emit_policy_exit_plan_request(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        result: &ToolResult,
    ) {
        let Some(request) = result.data.get("policy_exit_plan_request") else {
            return;
        };
        let Some(request_id) = request.get("request_id").and_then(Value::as_str) else {
            return;
        };
        let target_mode = request
            .get("target_mode")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or(PolicyMode::Default);
        let plan_summary = request
            .get("summary")
            .and_then(Value::as_str)
            .map(str::to_string);
        self.record_pending_plan_exit(PendingPlanExit::new(
            thread_id.clone(),
            turn_id.clone(),
            request_id.to_string(),
            target_mode,
            plan_summary,
        ))
        .await;
    }

    async fn resolve_user_input_request(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        result: ToolResult,
    ) -> anyhow::Result<ToolResult> {
        let Some(request) = result.data.get("user_input_request") else {
            return Ok(result);
        };
        let Some(request_id) = request.get("request_id").and_then(Value::as_str) else {
            return Ok(result);
        };
        let questions = request
            .get("questions")
            .cloned()
            .unwrap_or_else(|| serde_json::json!([]));
        if self.status().await.runtime_profile.is_non_interactive() {
            return Ok(ToolResult {
                id: result.id,
                name: result.name,
                text: "User input is unavailable in the current non-interactive runtime profile. Continue with reasonable defaults or fail with a clear blocker.".to_string(),
                data: serde_json::json!({
                    "request_id": request_id,
                    "questions": questions,
                    "unavailable": true,
                    "reason": "runtime_profile_non_interactive",
                }),
                is_error: true,
            });
        }
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.pending_user_inputs.lock().await.insert(
            request_id.to_string(),
            crate::runtime::PendingUserInput {
                thread_id: thread_id.clone(),
                turn_id: turn_id.clone(),
                tx,
            },
        );
        self.emit(RoderEvent::UserInputRequested(UserInputRequested {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            request_id: request_id.to_string(),
            questions,
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        let answers = rx.await.unwrap_or_else(|_| serde_json::json!({}));
        Ok(ToolResult {
            id: result.id,
            name: result.name,
            text: format!("User input received:\n{}", answers),
            data: serde_json::json!({
                "request_id": request_id,
                "answers": answers,
            }),
            is_error: false,
        })
    }
}

fn is_subagent_task_tool(name: &str) -> bool {
    name == "task" || (name.starts_with("task_") && !name.contains('.'))
}

fn subagent_error_kind(data: &Value) -> String {
    data.get("error")
        .and_then(|error| error.get("kind"))
        .and_then(Value::as_str)
        .unwrap_or("subagent_failed")
        .to_string()
}

fn json_string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Adapter exposing [`Runtime::route_tool_call`] as a [`TurnToolExecutor`] so a
/// provider that drives its own in-stream agent loop (the Cursor bidi runtime
/// client) can execute read/write/shell tool calls through Roder's registry and
/// policy mid-stream.
pub(crate) struct RuntimeTurnToolExecutor {
    pub(crate) runtime: Arc<Runtime>,
    pub(crate) thread_id: ThreadId,
    pub(crate) turn_id: TurnId,
    pub(crate) workspace: Option<String>,
    pub(crate) deadline: Option<OffsetDateTime>,
}

#[async_trait::async_trait]
impl roder_api::inference::TurnToolExecutor for RuntimeTurnToolExecutor {
    async fn execute(
        &self,
        call: roder_api::inference::ToolCallCompleted,
    ) -> anyhow::Result<roder_api::inference::TurnToolOutcome> {
        let record = self
            .runtime
            .route_tool_call(
                &self.thread_id,
                &self.turn_id,
                call,
                self.workspace.as_deref(),
                self.deadline,
            )
            .await?;
        Ok(roder_api::inference::TurnToolOutcome {
            result: record.result,
            is_error: record.is_error,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::is_subagent_task_tool;

    #[test]
    fn subagent_task_tool_detection_excludes_task_ledger_namespace() {
        assert!(is_subagent_task_tool("task"));
        assert!(is_subagent_task_tool("task_explore"));
        assert!(!is_subagent_task_tool("task_ledger.update"));
    }
}
