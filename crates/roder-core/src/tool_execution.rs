use roder_api::context::PolicyGate;
use roder_api::conversation::ToolResultRecord;
use roder_api::events::*;
use roder_api::policy_mode::{PolicyDecision, PolicyMode};
use roder_api::subagents::SubagentExitReason;
use roder_api::tools::ToolResult;
use roder_api::tools::{ToolCall, ToolExecutionContext};
use serde_json::Value;
use time::OffsetDateTime;

use crate::policy_gate::DefaultPolicyGate;
use crate::runtime::{PendingPlanExit, Runtime};

impl Runtime {
    pub(crate) async fn route_tool_call(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        call: roder_api::inference::ToolCallCompleted,
    ) -> anyhow::Result<ToolResultRecord> {
        self.emit(RoderEvent::ToolCallRequested(ToolCallRequested {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            tool_id: call.id.clone(),
            tool_name: call.name.clone(),
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        let Some(executor) = self.tool_registry.get(&call.name) else {
            let item = ToolResultRecord {
                id: call.id,
                name: Some(call.name),
                result: "tool not found".to_string(),
                is_error: true,
            };
            self.persist_turn_item(
                thread_id,
                turn_id,
                &roder_api::conversation::ConversationItem::ToolResult(item.clone()),
            )
            .await?;
            return Ok(item);
        };
        let mode = self.status().await.policy_mode;
        let parsed_args = serde_json::from_str(&call.arguments)
            .unwrap_or_else(|_| serde_json::json!({ "raw": call.arguments }));
        let tool_call = ToolCall {
            id: call.id.clone(),
            name: call.name.clone(),
            arguments: parsed_args.clone(),
            raw_arguments: call.arguments,
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
        };
        let ctx = ToolExecutionContext {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            effective_mode: mode,
        };
        let decision = DefaultPolicyGate::new().decide(&tool_call, mode, &ctx);
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
            let item = ToolResultRecord {
                id: tool_call.id.clone(),
                name: Some(tool_call.name.clone()),
                result: format!("policy denied tool call: {reason}"),
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
                timestamp: OffsetDateTime::now_utc(),
            }))
            .await;
            return Ok(item);
        }

        self.emit(RoderEvent::ToolCallStarted(ToolCallStarted {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            tool_id: tool_call.id.clone(),
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        let result = executor.execute(ctx, tool_call).await?;
        self.emit_subagent_events(thread_id, turn_id, &parsed_args, &result)
            .await;
        self.emit_policy_exit_plan_request(thread_id, turn_id, &result)
            .await;
        let item = ToolResultRecord {
            id: result.id.clone(),
            name: Some(result.name.clone()),
            result: result.text,
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
}

fn subagent_error_kind(data: &Value) -> String {
    data.get("error")
        .and_then(|error| error.get("kind"))
        .and_then(Value::as_str)
        .unwrap_or("subagent_failed")
        .to_string()
}
