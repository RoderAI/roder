use std::sync::Arc;

use roder_api::events::{RoderEvent, TeamMemberCompleted, ThreadId, TurnId};
use roder_api::inference::ToolCallCompleted;
use roder_api::teams::{TeamId, TeamMemberDescriptor, TeamMemberStatus};
use roder_api::tools::{ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult, ToolSpec};
use serde::Deserialize;
use serde_json::json;
use time::OffsetDateTime;
use tokio::time::{Duration, Instant};

use crate::runtime::Runtime;
use crate::teams::{TeamMemberStartRequest, TeamState};

const SPAWN_AGENT: &str = "spawn_agent";
const SEND_MESSAGE: &str = "send_message";
const FOLLOWUP_TASK: &str = "followup_task";
const WAIT_AGENT: &str = "wait_agent";
const LIST_AGENTS: &str = "list_agents";
const CLOSE_AGENT: &str = "close_agent";
const DEFAULT_WAIT_TIMEOUT_MS: u64 = 30_000;
const MAX_WAIT_TIMEOUT_MS: u64 = 300_000;

pub(crate) fn contribute_agent_control_tools(registry: &mut ToolRegistry) -> anyhow::Result<()> {
    for kind in AgentControlToolKind::all() {
        registry.register(Arc::new(AgentControlTool { kind }))?;
    }
    Ok(())
}

pub(crate) fn is_agent_control_tool(name: &str) -> bool {
    matches!(
        name,
        SPAWN_AGENT | SEND_MESSAGE | FOLLOWUP_TASK | WAIT_AGENT | LIST_AGENTS | CLOSE_AGENT
    )
}

#[derive(Debug, Clone, Copy)]
enum AgentControlToolKind {
    SpawnAgent,
    SendMessage,
    FollowupTask,
    WaitAgent,
    ListAgents,
    CloseAgent,
}

impl AgentControlToolKind {
    fn all() -> [Self; 6] {
        [
            Self::SpawnAgent,
            Self::SendMessage,
            Self::FollowupTask,
            Self::WaitAgent,
            Self::ListAgents,
            Self::CloseAgent,
        ]
    }

    fn name(self) -> &'static str {
        match self {
            Self::SpawnAgent => SPAWN_AGENT,
            Self::SendMessage => SEND_MESSAGE,
            Self::FollowupTask => FOLLOWUP_TASK,
            Self::WaitAgent => WAIT_AGENT,
            Self::ListAgents => LIST_AGENTS,
            Self::CloseAgent => CLOSE_AGENT,
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::SpawnAgent => {
                "Spawn a long-lived subagent teammate with its own Roder thread and send its initial task."
            }
            Self::SendMessage => {
                "Send a direct message to an existing subagent teammate and wake it if needed."
            }
            Self::FollowupTask => {
                "Assign follow-up work to an existing subagent teammate and wake it if needed."
            }
            Self::WaitAgent => "Wait for one or more subagent teammates to report completion.",
            Self::ListAgents => "List live subagent teammates owned by this caller thread.",
            Self::CloseAgent => {
                "Close or interrupt a subagent teammate when it is no longer needed."
            }
        }
    }

    fn parameters(self) -> serde_json::Value {
        match self {
            Self::SpawnAgent => json!({
                "type": "object",
                "properties": {
                    "task_name": {
                        "type": "string",
                        "description": "Stable lowercase task name for the spawned subagent."
                    },
                    "message": {
                        "type": "string",
                        "description": "Initial task or prompt for the spawned subagent."
                    },
                    "agent_type": {
                        "type": "string",
                        "description": "Optional role label for the subagent."
                    },
                    "model": {
                        "type": "string",
                        "description": "Optional model override for the spawned subagent."
                    },
                    "model_provider": {
                        "type": "string",
                        "description": "Optional provider override for the spawned subagent."
                    }
                },
                "required": ["task_name", "message"],
                "additionalProperties": false
            }),
            Self::SendMessage | Self::FollowupTask => json!({
                "type": "object",
                "properties": {
                    "target": {
                        "type": "string",
                        "description": "Subagent target from spawn_agent or list_agents. Accepts task name, member id, or team/member."
                    },
                    "message": {
                        "type": "string",
                        "description": "Message text for the subagent."
                    }
                },
                "required": ["target", "message"],
                "additionalProperties": false
            }),
            Self::WaitAgent => json!({
                "type": "object",
                "properties": {
                    "target": {
                        "type": "string",
                        "description": "Optional subagent target. Omit to wait for any subagent owned by this caller thread."
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Maximum time to wait in milliseconds."
                    }
                },
                "additionalProperties": false
            }),
            Self::ListAgents => json!({
                "type": "object",
                "properties": {
                    "path_prefix": {
                        "type": "string",
                        "description": "Optional task-name prefix."
                    }
                },
                "additionalProperties": false
            }),
            Self::CloseAgent => json!({
                "type": "object",
                "properties": {
                    "target": {
                        "type": "string",
                        "description": "Subagent target from spawn_agent or list_agents."
                    }
                },
                "required": ["target"],
                "additionalProperties": false
            }),
        }
    }
}

struct AgentControlTool {
    kind: AgentControlToolKind,
}

#[async_trait::async_trait]
impl ToolExecutor for AgentControlTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.kind.name().to_string(),
            description: self.kind.description().to_string(),
            parameters: self.kind.parameters(),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: roder_api::tools::ToolCall,
    ) -> anyhow::Result<ToolResult> {
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text: "agent control tools are executed by the runtime control plane".to_string(),
            data: json!({
                "error": {
                    "kind": "runtime_control_required",
                    "message": "agent control tools must run through Runtime::route_tool_call"
                }
            }),
            is_error: true,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SpawnAgentArgs {
    task_name: String,
    message: String,
    #[serde(default)]
    agent_type: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    model_provider: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MessageAgentArgs {
    target: String,
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WaitAgentArgs {
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListAgentsArgs {
    #[serde(default)]
    path_prefix: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CloseAgentArgs {
    target: String,
}

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
            SEND_MESSAGE | FOLLOWUP_TASK => {
                self.message_agent_tool(parent_thread_id, call, arguments)
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
            CLOSE_AGENT => {
                self.close_agent_tool(parent_thread_id, call, arguments)
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
        if args.task_name.trim().is_empty() || args.message.trim().is_empty() {
            return control_error(
                call,
                "invalid_arguments",
                "task_name and message are required",
            );
        }
        let member_name = args
            .agent_type
            .as_deref()
            .map(|role| format!("{}:{role}", args.task_name))
            .unwrap_or_else(|| args.task_name.clone());
        let next = match self
            .spawn_team_member_for_caller(
                parent_thread_id,
                parent_turn_id,
                TeamMemberStartRequest {
                    name: member_name,
                    model_provider: args.model_provider,
                    model: args.model,
                },
            )
            .await
        {
            Ok(team) => team,
            Err(err) => return control_error(call, "spawn_failed", err.to_string()),
        };
        let Some(member) = next.members.last().cloned() else {
            return control_error(call, "spawn_failed", "spawned team member was not recorded");
        };
        let turn_id = match self
            .message_team_member(&next.id, &member.id, args.message)
            .await
        {
            Ok(turn_id) => turn_id,
            Err(err) => return control_error(call, "message_failed", err.to_string()),
        };
        control_ok(
            call,
            format!(
                "spawned subagent {} as {}",
                args.task_name, member.thread_id
            ),
            json!({
                "team_id": next.id,
                "member_id": member.id,
                "thread_id": member.thread_id,
                "task_name": args.task_name,
                "turn_id": turn_id,
                "status": "running"
            }),
        )
    }

    async fn message_agent_tool(
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
            .message_team_member(&target.team_id, &target.member.id, args.message)
            .await
        {
            Ok(turn_id) => turn_id,
            Err(err) => return control_error(call, "message_failed", err.to_string()),
        };
        control_ok(
            call,
            format!("sent message to subagent {}", target.member.name),
            json!({
                "team_id": target.team_id,
                "member_id": target.member.id,
                "thread_id": target.member.thread_id,
                "turn_id": turn_id
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
        let prefix = args.path_prefix.unwrap_or_default();
        let agents = self
            .caller_agents(parent_thread_id)
            .await
            .into_iter()
            .flat_map(|team| {
                let team_id = team.id.clone();
                let prefix = prefix.clone();
                let caller_thread_id = parent_thread_id.clone();
                team.members
                    .into_iter()
                    .filter(|member| !matches!(member.status, TeamMemberStatus::Closed))
                    .filter(move |member| {
                        member.role != roder_api::teams::TeamMemberRole::Lead
                            && member.thread_id != caller_thread_id
                            && (prefix.is_empty() || member.name.starts_with(&prefix))
                    })
                    .map(move |member| {
                        json!({
                            "team_id": team_id,
                            "member_id": member.id,
                            "thread_id": member.thread_id,
                            "task_name": member.name,
                            "status": member.status
                        })
                    })
            })
            .collect::<Vec<_>>();
        control_ok(
            call,
            format!("{} subagent(s)", agents.len()),
            json!({ "agents": agents }),
        )
    }

    async fn wait_agent_tool(
        self: &Arc<Self>,
        parent_thread_id: &ThreadId,
        parent_turn_id: &TurnId,
        call: &ToolCallCompleted,
        arguments: serde_json::Value,
    ) -> ToolResult {
        let args = match serde_json::from_value::<WaitAgentArgs>(arguments) {
            Ok(args) => args,
            Err(err) => return control_error(call, "invalid_arguments", err.to_string()),
        };
        let targets = match args.target.as_deref() {
            Some(target) => match self.resolve_agent_target(parent_thread_id, target).await {
                Ok(target) => vec![target],
                Err(err) => return control_error(call, "unknown_agent", err.to_string()),
            },
            None => self
                .caller_agents(parent_thread_id)
                .await
                .into_iter()
                .flat_map(|team| {
                    let team_id = team.id;
                    let caller_thread_id = parent_thread_id.clone();
                    team.members
                        .into_iter()
                        .filter(move |member| {
                            member.role != roder_api::teams::TeamMemberRole::Lead
                                && member.thread_id != caller_thread_id
                        })
                        .map(move |member| AgentTarget {
                            team_id: team_id.clone(),
                            member,
                        })
                })
                .collect(),
        };
        if targets.is_empty() {
            return control_ok(
                call,
                "no subagents to wait for".to_string(),
                json!({ "timed_out": false, "agents": [] }),
            );
        }
        if targets
            .iter()
            .all(|target| is_terminal(target.member.status))
        {
            return control_ok(
                call,
                "all subagents already finished".to_string(),
                json!({
                    "timed_out": false,
                    "agents": agent_status_payloads(targets)
                }),
            );
        }

        let mut rx = self.subscribe_events();
        let timeout_ms = args
            .timeout_ms
            .unwrap_or(DEFAULT_WAIT_TIMEOUT_MS)
            .clamp(1, MAX_WAIT_TIMEOUT_MS);
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        let mut completed = Vec::new();
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, rx.recv()).await {
                Ok(Ok(envelope)) => {
                    if let RoderEvent::TeamMemberCompleted(event) = envelope.event
                        && event_matches_targets(&event, &targets)
                    {
                        completed.push(json!({
                            "team_id": event.team_id,
                            "member_id": event.member_id,
                            "thread_id": event.member_thread_id,
                            "turn_id": event.turn_id,
                            "status": event.status
                        }));
                        break;
                    }
                }
                Ok(Err(_)) | Err(_) => break,
            }
        }
        let timed_out = completed.is_empty();
        control_ok(
            call,
            if timed_out {
                "wait timed out".to_string()
            } else {
                "subagent completed".to_string()
            },
            json!({
                "timed_out": timed_out,
                "agents": completed,
                "parent_thread_id": parent_thread_id,
                "parent_turn_id": parent_turn_id
            }),
        )
    }

    async fn close_agent_tool(
        self: &Arc<Self>,
        parent_thread_id: &ThreadId,
        call: &ToolCallCompleted,
        arguments: serde_json::Value,
    ) -> ToolResult {
        let args = match serde_json::from_value::<CloseAgentArgs>(arguments) {
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
        let closed = match self
            .close_team_member(&target.team_id, &target.member.id)
            .await
        {
            Ok(closed) => closed,
            Err(err) => return control_error(call, "close_failed", err.to_string()),
        };
        control_ok(
            call,
            format!("closed subagent {}", closed.name),
            json!({
                "team_id": target.team_id,
                "member_id": closed.id,
                "thread_id": closed.thread_id,
                "previous_status": target.member.status,
                "status": closed.status
            }),
        )
    }

    async fn caller_agents(&self, parent_thread_id: &ThreadId) -> Vec<TeamState> {
        self.list_teams()
            .await
            .into_iter()
            .filter(|team| {
                team.lead_thread_id == *parent_thread_id
                    || team
                        .members
                        .iter()
                        .any(|member| member.thread_id == *parent_thread_id)
            })
            .collect()
    }

    async fn resolve_agent_target(
        &self,
        parent_thread_id: &ThreadId,
        target: &str,
    ) -> anyhow::Result<AgentTarget> {
        if let Some((team_id, member_id)) = target.split_once('/') {
            let team = self
                .read_team(team_id)
                .await
                .ok_or_else(|| anyhow::anyhow!("unknown team {team_id:?}"))?;
            if !team
                .members
                .iter()
                .any(|member| member.thread_id == *parent_thread_id)
            {
                anyhow::bail!("caller thread is not a member of team {team_id:?}");
            }
            let member = team
                .members
                .into_iter()
                .find(|member| {
                    member.role != roder_api::teams::TeamMemberRole::Lead
                        && member.thread_id != *parent_thread_id
                        && (member.id == member_id || member.name == member_id)
                })
                .ok_or_else(|| anyhow::anyhow!("unknown team member {member_id:?}"))?;
            return Ok(AgentTarget {
                team_id: team_id.to_string(),
                member,
            });
        }

        let matches = self
            .caller_agents(parent_thread_id)
            .await
            .into_iter()
            .flat_map(|team| {
                let team_id = team.id;
                team.members
                    .into_iter()
                    .filter(|member| member.role != roder_api::teams::TeamMemberRole::Lead)
                    .filter(move |member| {
                        member.thread_id != *parent_thread_id
                            && (member.id == target
                                || member.name == target
                                || member.thread_id == target
                                || member.name.split(':').next() == Some(target))
                    })
                    .map(move |member| AgentTarget {
                        team_id: team_id.clone(),
                        member,
                    })
            })
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [target] => Ok(target.clone()),
            [] => anyhow::bail!("unknown subagent target {target:?}"),
            _ => anyhow::bail!("ambiguous subagent target {target:?}; use team/member"),
        }
    }
}

#[derive(Debug, Clone)]
struct AgentTarget {
    team_id: TeamId,
    member: TeamMemberDescriptor,
}

fn event_matches_targets(event: &TeamMemberCompleted, targets: &[AgentTarget]) -> bool {
    targets
        .iter()
        .any(|target| target.team_id == event.team_id && target.member.id == event.member_id)
}

fn is_terminal(status: TeamMemberStatus) -> bool {
    matches!(
        status,
        TeamMemberStatus::Completed
            | TeamMemberStatus::Failed
            | TeamMemberStatus::Interrupted
            | TeamMemberStatus::Closed
    )
}

fn agent_status_payloads(targets: Vec<AgentTarget>) -> Vec<serde_json::Value> {
    targets
        .into_iter()
        .map(|target| {
            json!({
                "team_id": target.team_id,
                "member_id": target.member.id,
                "thread_id": target.member.thread_id,
                "task_name": target.member.name,
                "status": target.member.status
            })
        })
        .collect()
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

pub(crate) fn closed_member_event(
    team_id: TeamId,
    member: &TeamMemberDescriptor,
    interrupted_turn_id: Option<TurnId>,
) -> RoderEvent {
    RoderEvent::TeamMemberCompleted(TeamMemberCompleted {
        team_id,
        member_id: member.id.clone(),
        member_thread_id: member.thread_id.clone(),
        turn_id: interrupted_turn_id,
        status: TeamMemberStatus::Closed,
        timestamp: OffsetDateTime::now_utc(),
    })
}
