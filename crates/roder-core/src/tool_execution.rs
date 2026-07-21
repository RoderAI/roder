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
        if is_subagent_task_tool(&call.name)
            && let Some(deadline) = deadline
        {
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
        /*
         * External tools are checked before the registry so a host-declared tool shadows a
         * built-in of the same name, matching the advertised toolset (`filtered_tool_specs`).
         * They skip schema validation and the policy gate: the host supplied the schema and
         * executes the call itself.
         */
        let overrides = self.thread_turn_overrides(thread_id).await?;
        if overrides
            .external_tools
            .iter()
            .any(|spec| spec.name == call.name)
        {
            return self
                .execute_external_tool_call(thread_id, turn_id, &call, parsed_args)
                .await;
        }
        /*
         * Allowlists gate dispatch, not just advertisement: models call unadvertised
         * tools by name from training priors, and in bypass/auto-approve configs the
         * policy gate would otherwise execute them. The task ledger tool is exempt
         * because eval turns advertise it outside `filtered_tool_specs`.
         */
        if call.name != crate::runtime::TASK_LEDGER_TOOL_NAME {
            let runtime_allowlist = self.status().await.tool_allowlist;
            if !crate::runtime::allowlist_permits(&runtime_allowlist, &call.name)
                || !crate::runtime::allowlist_permits(&overrides.tool_allowlist, &call.name)
            {
                let item = ToolResultRecord {
                    id: call.id.clone(),
                    name: Some(call.name.clone()),
                    result: format!("tool {} is not permitted by the tool allowlist", call.name),
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
        }
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
        let native_workspace_tool = native_tool_uses_remote_workspace(&tool_call.name);
        let runner_binding = if native_workspace_tool {
            Some(
                self.runner_binding_for_thread(thread_id)
                    .await
                    .map_err(|err| format!("remote runner workspace is unavailable: {err}")),
            )
        } else {
            None
        };
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

        // A preview reads the existing file synchronously. Only produce one
        // when this call is known to use a permitted local workspace; remote
        // and fail-closed hosted calls must not inspect the host filesystem.
        let local_preview_allowed = match &runner_binding {
            None => true,
            Some(Ok(None)) => self.allows_local_workspaces(),
            Some(Ok(Some(_))) | Some(Err(_)) => false,
        };
        let preview = if local_preview_allowed {
            file_change_preview(
                &tool_call,
                workspace.or(runtime_config.workspace.as_deref()),
            )
        } else {
            None
        };
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
        /*
         * Provision only when an approved native workspace tool is about to
         * execute. Text-only, external/MCP, and runtime-control turns never
         * touch the runner. A bound workspace tool must never fall back to
         * local execution: resolution failures become error tool results.
         */
        if let Some(runner_binding) = runner_binding {
            let workspace_error = match runner_binding {
                Ok(Some(binding)) => match self
                    .remote_workspace_for_binding(thread_id, binding)
                    .await
                {
                    Ok(remote) => {
                        ctx = ctx.with_remote_workspace(remote);
                        None
                    }
                    Err(err) => Some(format!("remote runner workspace is unavailable: {err}")),
                },
                Ok(None) if self.allows_local_workspaces() => None,
                Ok(None) => Some(
                    "local workspace execution is disabled and the thread has no remote runner binding"
                        .to_string(),
                ),
                Err(error) => Some(error),
            };
            if let Some(error) = workspace_error {
                let item = ToolResultRecord {
                    id: call.id.clone(),
                    name: Some(call.name.clone()),
                    result: error,
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
        }
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
        // Long-lived collaboration controls have their own team lifecycle events. Projecting
        // their tool result through the legacy one-shot task bridge would emit a synthetic
        // SubagentCompleted immediately after spawn_agent returns, while the agent is still
        // running.
        if !crate::agent_control_tools::is_agent_control_tool(&tool_call.name) {
            self.emit_subagent_events(thread_id, turn_id, &parsed_args, &result)
                .await;
        }
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
        let Some(value) = result.data.get("mediaArtifacts") else {
            return;
        };
        let Ok(artifacts) =
            serde_json::from_value::<Vec<roder_api::media::MediaArtifact>>(value.clone())
        else {
            return;
        };
        for artifact in artifacts {
            self.emit(RoderEvent::MediaArtifactCreated(
                roder_api::events::MediaArtifactCreated {
                    thread_id: thread_id.clone(),
                    turn_id: turn_id.clone(),
                    artifact,
                    timestamp: OffsetDateTime::now_utc(),
                },
            ))
            .await;
        }
        let previews = serde_json::from_value::<Vec<roder_api::media::MediaPreview>>(
            result
                .data
                .get("mediaPreviews")
                .cloned()
                .unwrap_or_default(),
        )
        .unwrap_or_default();
        for preview in previews {
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

    /**
     * Publishes the call to the host client and pauses the turn on a oneshot until
     * `tools/resolve` answers, the configured timeout expires, or the turn is interrupted
     * (which cancels the pending entry). Timeout and cancellation surface as error tool
     * results so the turn continues instead of hanging.
     */
    async fn execute_external_tool_call(
        self: &Arc<Self>,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        call: &roder_api::inference::ToolCallCompleted,
        parsed_args: Value,
    ) -> anyhow::Result<ToolResultRecord> {
        self.emit(RoderEvent::ToolCallStarted(ToolCallStarted {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            tool_id: call.id.clone(),
            tool_name: Some(call.name.clone()),
            display_payload: tool_display_payload(Some(&call.name), Some(&parsed_args), None),
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        let request_id = format!("exttool-{}", uuid::Uuid::new_v4());
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.pending_external_tool_calls.lock().await.insert(
            request_id.clone(),
            crate::runtime::PendingExternalToolCall {
                thread_id: thread_id.clone(),
                turn_id: turn_id.clone(),
                tool_id: call.id.clone(),
                tool_name: call.name.clone(),
                tx,
            },
        );
        self.emit(RoderEvent::ExternalToolCallRequested(
            ExternalToolCallRequested {
                thread_id: thread_id.clone(),
                turn_id: turn_id.clone(),
                request_id: request_id.clone(),
                tool_id: call.id.clone(),
                tool_name: call.name.clone(),
                arguments: parsed_args.clone(),
                timestamp: OffsetDateTime::now_utc(),
            },
        ))
        .await;
        let timeout_seconds = self.status().await.external_tool_timeout_seconds;
        let timeout = std::time::Duration::from_secs(timeout_seconds);
        let mut rx = rx;
        let wait_result = tokio::time::timeout(timeout, &mut rx).await;
        let resolution = match wait_result {
            Ok(Ok(resolution)) => resolution,
            // Sender dropped: the pending entry was cancelled by a turn interrupt.
            Ok(Err(_)) => crate::runtime::ExternalToolResolution {
                output: format!("external tool call {} was cancelled", call.name),
                is_error: true,
            },
            Err(_) => {
                /*
                 * Whoever removes the map entry owns the outcome. When the timer fires
                 * but `remove` returns `None`, `resolve_external_tool_call` won the race:
                 * it already acked the host (`resolved: true`, outcome=resolved), so the
                 * delivered resolution must be honored instead of fabricating a timeout
                 * the host never saw.
                 */
                let removed = self
                    .pending_external_tool_calls
                    .lock()
                    .await
                    .remove(&request_id);
                if removed.is_some() {
                    self.emit(RoderEvent::ExternalToolCallResolved(
                        ExternalToolCallResolved {
                            thread_id: thread_id.clone(),
                            turn_id: turn_id.clone(),
                            request_id: request_id.clone(),
                            tool_id: call.id.clone(),
                            tool_name: call.name.clone(),
                            outcome: ExternalToolCallOutcome::TimedOut,
                            is_error: true,
                            timestamp: OffsetDateTime::now_utc(),
                        },
                    ))
                    .await;
                    crate::runtime::ExternalToolResolution {
                        output: format!(
                            "external tool call {} timed out after {timeout_seconds}s waiting for tools/resolve",
                            call.name
                        ),
                        is_error: true,
                    }
                } else {
                    match rx.await {
                        Ok(resolution) => resolution,
                        // Entry was removed by the interrupt sweep, which drops the sender.
                        Err(_) => crate::runtime::ExternalToolResolution {
                            output: format!("external tool call {} was cancelled", call.name),
                            is_error: true,
                        },
                    }
                }
            }
        };
        let item = ToolResultRecord {
            id: call.id.clone(),
            name: Some(call.name.clone()),
            result: cap_tool_output_lines(resolution.output),
            display_payload: tool_display_payload(Some(&call.name), Some(&parsed_args), None),
            is_error: resolution.is_error,
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
            tool_id: item.id.clone(),
            tool_name: item.name.clone(),
            display_payload: item.display_payload.clone(),
            is_error: item.is_error,
            output: Some(item.result.clone()),
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        Ok(item)
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
        let next_steps = json_string_array(request.get("next_steps"));
        self.record_pending_plan_exit(PendingPlanExit::new(
            thread_id.clone(),
            turn_id.clone(),
            request_id.to_string(),
            target_mode,
            plan_summary,
            next_steps,
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

fn native_tool_uses_remote_workspace(name: &str) -> bool {
    matches!(
        name,
        "read_file"
            | "list_files"
            | "write_file"
            | "grep"
            | "glob"
            | "edit"
            | "multi_edit"
            | "apply_patch"
            | "shell"
            | "exec_command"
            | "write_stdin"
            | "unified_exec"
            | "view_image"
    ) || name.starts_with("design_")
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
        let tool_item = roder_api::transcript::TranscriptItem::ToolCall(
            roder_api::transcript::ToolCallRecord {
                id: call.id.clone(),
                name: call.name.clone(),
                arguments: call.arguments.clone(),
            },
        );
        self.runtime
            .persist_turn_item(&self.thread_id, &self.turn_id, &tool_item)
            .await?;

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

    fn register_provider_cleanup(
        &self,
        cleanup: std::sync::Arc<dyn roder_api::inference::ProviderTurnCleanup>,
    ) {
        self.runtime
            .register_provider_turn_cleanup(&self.turn_id, cleanup);
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

    #[tokio::test]
    async fn test_runtime_turn_tool_executor_persists_tool_call() {
        use super::RuntimeTurnToolExecutor;
        use roder_api::extension::ExtensionRegistryBuilder;
        use roder_api::inference::{
            InferenceCapabilities, InferenceEngine, InferenceEventStream, InferenceProviderContext,
            InferenceTurnContext, ModelDescriptor, ToolCallCompleted, TurnToolExecutor,
        };
        use roder_api::transcript::TranscriptItem;
        use std::sync::Arc;

        struct MockInferenceEngine;

        #[async_trait::async_trait]
        impl InferenceEngine for MockInferenceEngine {
            fn id(&self) -> roder_api::extension::InferenceEngineId {
                "mock".to_string()
            }

            fn capabilities(&self) -> InferenceCapabilities {
                InferenceCapabilities::coding_agent_default()
            }

            async fn list_models(
                &self,
                _ctx: InferenceProviderContext<'_>,
            ) -> anyhow::Result<Vec<ModelDescriptor>> {
                Ok(Vec::new())
            }

            async fn stream_turn(
                &self,
                _ctx: InferenceTurnContext<'_>,
                _request: roder_api::inference::AgentInferenceRequest,
            ) -> anyhow::Result<InferenceEventStream> {
                Ok(Box::pin(futures::stream::empty()))
            }
        }

        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(MockInferenceEngine));
        let registry = builder.build().unwrap();
        let runtime = Arc::new(
            crate::runtime::Runtime::new(registry, crate::runtime::RuntimeConfig::default())
                .unwrap(),
        );
        let thread = runtime.create_thread(None).await.unwrap();
        let thread_id = thread.thread_id;
        let turn_id = "test_turn_1".to_string();

        let executor = RuntimeTurnToolExecutor {
            runtime: Arc::clone(&runtime),
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            workspace: None,
            deadline: None,
        };

        let call = ToolCallCompleted {
            id: "call_1".to_string(),
            name: "test_tool".to_string(),
            arguments: "{}".to_string(),
        };

        let mut events = runtime.subscribe_events();

        // Note: the test_tool is not registered, so we expect route_tool_call to fail with tool not found / error
        // but the ToolCall completed item itself MUST still be persisted beforehand!
        let _ = executor.execute(call).await;

        let mut found_tool_call = false;
        while let Ok(envelope) = events.try_recv() {
            if let roder_api::events::RoderEvent::TranscriptItemAppended(appended) = envelope.event
            {
                if let Some(TranscriptItem::ToolCall(tc)) = appended.item {
                    if tc.id == "call_1" && tc.name == "test_tool" {
                        found_tool_call = true;
                        break;
                    }
                }
            }
        }
        assert!(
            found_tool_call,
            "The ToolCall record must be persisted and broadcast when execute() is called on RuntimeTurnToolExecutor"
        );
    }
}
