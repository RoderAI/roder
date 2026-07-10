use std::collections::HashSet;
use std::sync::Arc;

use roder_api::events::{ThreadId, TurnId};
use roder_api::inference::ToolCallCompleted;
use roder_api::teams::TeamMemberStatus;
use roder_api::tools::ToolResult;
use serde_json::json;

use crate::runtime::Runtime;
use crate::teams::TeamMemberStartRequest;

mod spec;
mod targets;
mod wait;

use spec::{
    FOLLOWUP_TASK, INTERRUPT_AGENT, InterruptAgentArgs, LIST_AGENTS, ListAgentsArgs,
    MessageAgentArgs, SEND_MESSAGE, SPAWN_AGENT, SpawnAgentArgs, WAIT_AGENT,
    full_history_overrides_present, normalize_fork_turns, normalized_optional, valid_task_name,
};
pub(crate) use spec::{contribute_agent_control_tools, is_agent_control_tool};
use targets::{
    agent_path_matches_prefix, canonical_agent_path, member_identity, reject_root_or_self,
};

impl Runtime {
    pub(crate) async fn execute_agent_control_tool(
        self: &Arc<Self>,
        parent_thread_id: &ThreadId,
        parent_turn_id: &TurnId,
        call: &ToolCallCompleted,
        arguments: serde_json::Value,
    ) -> ToolResult {
        match call.name.as_str() {
            SPAWN_AGENT => {
                self.spawn_agent_tool(parent_thread_id, parent_turn_id, call, arguments)
                    .await
            }
            SEND_MESSAGE => {
                self.send_message_tool(parent_thread_id, call, arguments)
                    .await
            }
            FOLLOWUP_TASK => {
                self.followup_task_tool(parent_thread_id, call, arguments)
                    .await
            }
            WAIT_AGENT => {
                self.wait_agent_tool(parent_thread_id, parent_turn_id, call, arguments)
                    .await
            }
            LIST_AGENTS => {
                self.list_agents_tool(parent_thread_id, call, arguments)
                    .await
            }
            INTERRUPT_AGENT => {
                self.interrupt_agent_tool(parent_thread_id, call, arguments)
                    .await
            }
            _ => control_error(
                call,
                "unknown_agent_control_tool",
                "unknown agent control tool",
            ),
        }
    }

    async fn spawn_agent_tool(
        self: &Arc<Self>,
        parent_thread_id: &ThreadId,
        parent_turn_id: &TurnId,
        call: &ToolCallCompleted,
        arguments: serde_json::Value,
    ) -> ToolResult {
        let args = match serde_json::from_value::<SpawnAgentArgs>(arguments) {
            Ok(args) => args,
            Err(err) => return control_error(call, "invalid_arguments", err.to_string()),
        };
        let SpawnAgentArgs {
            task_name,
            message,
            agent_type,
            model,
            model_provider,
            reasoning_effort,
            fork_turns,
        } = args;
        let task_name = task_name.trim().to_string();
        if !valid_task_name(&task_name) || message.trim().is_empty() {
            return control_error(
                call,
                "invalid_arguments",
                "task_name must contain only lowercase letters, digits, and underscores, and message is required",
            );
        }
        let agent_type = normalized_optional(agent_type);
        let model = normalized_optional(model);
        let model_provider = normalized_optional(model_provider);
        let reasoning_effort = normalized_optional(reasoning_effort);
        let fork_turns = match normalize_fork_turns(fork_turns.as_deref()) {
            Ok(fork_turns) => fork_turns,
            Err(message) => return control_error(call, "invalid_arguments", message),
        };
        if full_history_overrides_present(
            &fork_turns,
            agent_type.as_deref(),
            model.as_deref(),
            model_provider.as_deref(),
            reasoning_effort.as_deref(),
        ) {
            return control_error(
                call,
                "invalid_arguments",
                "Full-history forked agents inherit the parent agent type, model, provider, and reasoning effort; omit agent_type, model, model_provider, and reasoning_effort, or spawn without a full-history fork.",
            );
        }
        let (next, turn_id) = match self
            .spawn_and_start_team_member_for_caller(
                parent_thread_id,
                parent_turn_id,
                TeamMemberStartRequest {
                    name: task_name.clone(),
                    model_provider,
                    model,
                },
                reasoning_effort,
                fork_turns,
                agent_type,
                message,
            )
            .await
        {
            Ok(spawned) => spawned,
            Err(err) => return control_error(call, "spawn_failed", err.to_string()),
        };
        let Some(member) = next.members.last().cloned() else {
            return control_error(call, "spawn_failed", "spawned team member was not recorded");
        };
        control_ok(
            call,
            format!("spawned subagent {} as {}", task_name, member.thread_id),
            json!({
                "team_id": next.id,
                "member_id": member.id,
                "thread_id": member.thread_id,
                "task_name": member.agent_path.unwrap_or(task_name),
                "turn_id": turn_id,
                "status": "running"
            }),
        )
    }

    async fn send_message_tool(
        self: &Arc<Self>,
        parent_thread_id: &ThreadId,
        call: &ToolCallCompleted,
        arguments: serde_json::Value,
    ) -> ToolResult {
        let args = match serde_json::from_value::<MessageAgentArgs>(arguments) {
            Ok(args) => args,
            Err(err) => return control_error(call, "invalid_arguments", err.to_string()),
        };
        if args.target.trim().is_empty() || args.message.trim().is_empty() {
            return control_error(call, "invalid_arguments", "target and message are required");
        }
        let target = match self
            .resolve_agent_target(parent_thread_id, &args.target)
            .await
        {
            Ok(target) => target,
            Err(err) => return control_error(call, "unknown_agent", err.to_string()),
        };
        if matches!(target.member.status, TeamMemberStatus::Closed) {
            return control_error(
                call,
                "agent_closed",
                format!("subagent {} is closed", target.member.name),
            );
        }
        let turn_id = match self
            .queue_team_member_message(
                parent_thread_id,
                &target.team_id,
                &target.member.id,
                args.message,
            )
            .await
        {
            Ok(turn_id) => turn_id,
            Err(err) => return control_error(call, "message_failed", err.to_string()),
        };
        control_ok(
            call,
            format!(
                "queued message for agent {}",
                member_identity(&target.member)
            ),
            json!({
                "team_id": target.team_id,
                "member_id": target.member.id,
                "thread_id": target.member.thread_id,
                "turn_id": turn_id,
                "queued": true
            }),
        )
    }

    async fn followup_task_tool(
        self: &Arc<Self>,
        parent_thread_id: &ThreadId,
        call: &ToolCallCompleted,
        arguments: serde_json::Value,
    ) -> ToolResult {
        let args = match serde_json::from_value::<MessageAgentArgs>(arguments) {
            Ok(args) => args,
            Err(err) => return control_error(call, "invalid_arguments", err.to_string()),
        };
        if args.target.trim().is_empty() || args.message.trim().is_empty() {
            return control_error(call, "invalid_arguments", "target and message are required");
        }
        let target = match self
            .resolve_agent_target(parent_thread_id, &args.target)
            .await
        {
            Ok(target) => target,
            Err(err) => return control_error(call, "unknown_agent", err.to_string()),
        };
        if let Err(message) = reject_root_or_self(parent_thread_id, &target.member, "follow up") {
            return control_error(call, "invalid_target", message);
        }
        if matches!(target.member.status, TeamMemberStatus::Closed) {
            return control_error(
                call,
                "agent_closed",
                format!("agent {} is closed", member_identity(&target.member)),
            );
        }
        let turn_id = match self
            .followup_team_member(
                parent_thread_id,
                &target.team_id,
                &target.member.id,
                args.message,
            )
            .await
        {
            Ok(turn_id) => turn_id,
            Err(err) => return control_error(call, "followup_failed", err.to_string()),
        };
        control_ok(
            call,
            format!(
                "assigned follow-up to agent {}",
                member_identity(&target.member)
            ),
            json!({
                "team_id": target.team_id,
                "member_id": target.member.id,
                "thread_id": target.member.thread_id,
                "turn_id": turn_id,
                "status": "running"
            }),
        )
    }

    async fn list_agents_tool(
        self: &Arc<Self>,
        parent_thread_id: &ThreadId,
        call: &ToolCallCompleted,
        arguments: serde_json::Value,
    ) -> ToolResult {
        let args = match serde_json::from_value::<ListAgentsArgs>(arguments) {
            Ok(args) => args,
            Err(err) => return control_error(call, "invalid_arguments", err.to_string()),
        };
        let teams = self.caller_agents(parent_thread_id).await;
        let caller_path = teams
            .iter()
            .flat_map(|team| team.members.iter())
            .find(|member| member.thread_id == *parent_thread_id)
            .and_then(|member| member.agent_path.as_deref())
            .unwrap_or("/root");
        let prefix = args
            .path_prefix
            .as_deref()
            .map(str::trim)
            .filter(|prefix| !prefix.is_empty())
            .map(|prefix| canonical_agent_path(caller_path, prefix));
        let mut seen = HashSet::new();
        let mut agents = teams
            .into_iter()
            .flat_map(|team| {
                let team_id = team.id;
                team.members
                    .into_iter()
                    .map(move |member| (team_id.clone(), member))
            })
            .filter(|(_, member)| !matches!(member.status, TeamMemberStatus::Closed))
            .filter(|(_, member)| seen.insert(member.thread_id.clone()))
            .filter(|(_, member)| {
                prefix.as_deref().is_none_or(|prefix| {
                    member
                        .agent_path
                        .as_deref()
                        .is_some_and(|path| agent_path_matches_prefix(path, prefix))
                })
            })
            .map(|(team_id, member)| {
                let agent_path = member
                    .agent_path
                    .clone()
                    .unwrap_or_else(|| member_identity(&member).to_string());
                let task_name = member.task_name.clone().unwrap_or_else(|| {
                    if member.role == roder_api::teams::TeamMemberRole::Lead {
                        "root".to_string()
                    } else {
                        member.name.clone()
                    }
                });
                json!({
                    "team_id": team_id,
                    "member_id": member.id,
                    "thread_id": member.thread_id,
                    "agent_name": agent_path.clone(),
                    "agent_path": agent_path,
                    "task_name": task_name,
                    "status": member.status,
                    "final_message": member.final_message,
                    "terminal_error": member.terminal_error
                })
            })
            .collect::<Vec<_>>();
        agents.sort_by(|left, right| {
            left["agent_path"]
                .as_str()
                .unwrap_or_default()
                .cmp(right["agent_path"].as_str().unwrap_or_default())
        });
        control_ok(
            call,
            format!("{} subagent(s)", agents.len()),
            json!({ "agents": agents }),
        )
    }

    async fn interrupt_agent_tool(
        self: &Arc<Self>,
        parent_thread_id: &ThreadId,
        call: &ToolCallCompleted,
        arguments: serde_json::Value,
    ) -> ToolResult {
        let args = match serde_json::from_value::<InterruptAgentArgs>(arguments) {
            Ok(args) => args,
            Err(err) => return control_error(call, "invalid_arguments", err.to_string()),
        };
        let target = match self
            .resolve_agent_target(parent_thread_id, &args.target)
            .await
        {
            Ok(target) => target,
            Err(err) => return control_error(call, "unknown_agent", err.to_string()),
        };
        if let Err(message) = reject_root_or_self(parent_thread_id, &target.member, "interrupt") {
            return control_error(call, "invalid_target", message);
        }
        if matches!(target.member.status, TeamMemberStatus::Closed) {
            return control_error(
                call,
                "agent_closed",
                format!("agent {} is closed", member_identity(&target.member)),
            );
        }
        let interrupted_turn_id = match self
            .interrupt_team_member(&target.team_id, &target.member.id)
            .await
        {
            Ok(turn_id) => turn_id,
            Err(err) => return control_error(call, "interrupt_failed", err.to_string()),
        };
        control_ok(
            call,
            format!("interrupted agent {}", member_identity(&target.member)),
            json!({
                "team_id": target.team_id,
                "member_id": target.member.id,
                "thread_id": target.member.thread_id,
                "previous_status": target.member.status,
                "interrupted_turn_id": interrupted_turn_id
            }),
        )
    }
}

fn control_ok(call: &ToolCallCompleted, text: String, data: serde_json::Value) -> ToolResult {
    ToolResult {
        id: call.id.clone(),
        name: call.name.clone(),
        text,
        data,
        is_error: false,
    }
}

fn control_error(
    call: &ToolCallCompleted,
    kind: &'static str,
    message: impl ToString,
) -> ToolResult {
    let message = message.to_string();
    ToolResult {
        id: call.id.clone(),
        name: call.name.clone(),
        text: message.clone(),
        data: json!({
            "error": {
                "kind": kind,
                "message": message
            }
        }),
        is_error: true,
    }
}
