use roder_api::ToolSchemaPolicy;
use roder_api::artifacts::{ContextArtifactKind, format_artifact_reference};
use roder_api::conversation::{ToolResultRecord, tool_display_payload};
use roder_api::events::*;
use roder_api::policy_mode::{PolicyDecision, PolicyMode};
use roder_api::subagents::SubagentExitReason;
use roder_api::tools::ToolCall;
use roder_api::tools::ToolResult;
use serde_json::Value;
use time::OffsetDateTime;

use crate::artifacts::CreateArtifactRequest;
use crate::policy_gate::DefaultPolicyGate;
use crate::runtime::{PendingPlanExit, Runtime};
use crate::tool_output::{
    artifact_backed_tool_output, cap_tool_output_lines, should_spill_tool_output,
};
use crate::tool_preview::file_change_preview;
use crate::tool_validation::{
    emit_tool_validation_recorded, validate_tool_call_arguments, validation_error_tool_result,
};

impl Runtime {
    pub(crate) async fn route_tool_call(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        call: roder_api::inference::ToolCallCompleted,
        workspace: Option<&str>,
    ) -> anyhow::Result<ToolResultRecord> {
        let parsed_args = serde_json::from_str(&call.arguments)
            .unwrap_or_else(|_| serde_json::json!({ "raw": call.arguments }));
        self.emit(RoderEvent::ToolCallRequested(ToolCallRequested {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            tool_id: call.id.clone(),
            tool_name: call.name.clone(),
            display_payload: tool_display_payload(Some(&call.name), Some(&parsed_args), None),
            timestamp: OffsetDateTime::now_utc(),
        }))
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
                &roder_api::conversation::ConversationItem::ToolResult(item.clone()),
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
                    &roder_api::conversation::ConversationItem::ToolResult(item.clone()),
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
        };
        let mut runtime_config = self.status().await;
        let mode = self.effective_policy_mode_for_thread(thread_id).await;
        runtime_config.policy_mode = mode;
        let tool_call = ToolCall {
            id: call.id.clone(),
            name: call.name.clone(),
            arguments,
            raw_arguments: call.arguments,
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
        };
        let mut ctx = self.tool_execution_context(
            thread_id.clone(),
            turn_id.clone(),
            mode,
            workspace.or(runtime_config.workspace.as_deref()),
        );
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
                &roder_api::conversation::ConversationItem::ToolResult(item.clone()),
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
                &roder_api::conversation::ConversationItem::ToolResult(item.clone()),
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
            return Ok(item);
        }
        ctx.effective_mode = self.effective_policy_mode_for_thread(thread_id).await;

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
        let result = match executor.execute(ctx, tool_call.clone()).await {
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
        };
        let result = self
            .resolve_user_input_request(thread_id, turn_id, result)
            .await?;
        self.emit_subagent_events(thread_id, turn_id, &parsed_args, &result)
            .await;
        self.emit_policy_exit_plan_request(thread_id, turn_id, &result)
            .await;
        self.emit_hunk_records(&result).await;
        self.emit_media_artifacts(thread_id, turn_id, &result).await;
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
            &roder_api::conversation::ConversationItem::ToolResult(item.clone()),
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

fn subagent_error_kind(data: &Value) -> String {
    data.get("error")
        .and_then(|error| error.get("kind"))
        .and_then(Value::as_str)
        .unwrap_or("subagent_failed")
        .to_string()
}
