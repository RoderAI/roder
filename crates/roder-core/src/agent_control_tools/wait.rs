use std::sync::Arc;

use roder_api::events::{RoderEvent, TeamMemberCompleted, ThreadId, TurnId};
use roder_api::inference::ToolCallCompleted;
use roder_api::teams::TeamMemberStatus;
use roder_api::tools::ToolResult;
use serde_json::json;
use tokio::time::{Duration, Instant};

use crate::runtime::Runtime;

use super::spec::{
    DEFAULT_WAIT_TIMEOUT_MS, MAX_WAIT_TIMEOUT_MS, MIN_WAIT_TIMEOUT_MS, WaitAgentArgs,
};
use super::targets::{AgentTarget, member_agent_path};
use super::{control_error, control_ok};

impl Runtime {
    pub(super) async fn wait_agent_tool(
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
        let timeout_ms = args.timeout_ms.unwrap_or(DEFAULT_WAIT_TIMEOUT_MS);
        if !(MIN_WAIT_TIMEOUT_MS..=MAX_WAIT_TIMEOUT_MS).contains(&timeout_ms) {
            return control_error(
                call,
                "invalid_arguments",
                format!(
                    "timeout_ms must be between {MIN_WAIT_TIMEOUT_MS} and {MAX_WAIT_TIMEOUT_MS}"
                ),
            );
        }

        // Subscribe before reading status so a completion racing with the snapshot is retained
        // by the receiver rather than turning into a false timeout.
        let mut rx = self.subscribe_events();
        let target_selector = args.target.clone();
        let mut targets = match self
            .wait_targets(parent_thread_id, target_selector.as_deref())
            .await
        {
            Ok(targets) => targets,
            Err(err) => return control_error(call, "unknown_agent", err.to_string()),
        };
        if targets.is_empty() {
            return control_ok(
                call,
                "no subagents to wait for".to_string(),
                json!({ "timed_out": false, "agents": [] }),
            );
        }
        if target_selector.is_none() {
            let nonterminal_targets = targets
                .iter()
                .filter(|target| !is_terminal(target.member.status))
                .cloned()
                .collect::<Vec<_>>();
            if !nonterminal_targets.is_empty() {
                targets = nonterminal_targets;
            }
        }
        let terminal_targets = targets
            .iter()
            .filter(|target| is_terminal(target.member.status))
            .cloned()
            .collect::<Vec<_>>();
        if !terminal_targets.is_empty() {
            return control_ok(
                call,
                "agent result already available".to_string(),
                json!({
                    "timed_out": false,
                    "agents": agent_status_payloads(terminal_targets)
                }),
            );
        }
        if self.has_pending_turn_steers(parent_turn_id).await {
            return wait_activity_result(call, parent_thread_id, parent_turn_id, targets);
        }

        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        let mut completed = Vec::new();
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, rx.recv()).await {
                Ok(Ok(envelope)) => match envelope.event {
                    RoderEvent::TeamMemberCompleted(event)
                        if event_matches_targets(&event, &targets) =>
                    {
                        let target = targets.iter().find(|target| {
                            target.team_id == event.team_id && target.member.id == event.member_id
                        });
                        completed.push(json!({
                            "team_id": event.team_id,
                            "member_id": event.member_id,
                            "thread_id": event.member_thread_id,
                            "agent_path": target.and_then(|target| target.member.agent_path.clone()),
                            "task_name": target.and_then(|target| target.member.task_name.clone()),
                            "turn_id": event.turn_id,
                            "status": event.status,
                            "final_message": event.final_message,
                            "terminal_error": event.error
                        }));
                        break;
                    }
                    RoderEvent::TurnSteered(event) if event.thread_id == *parent_thread_id => {
                        let current_targets = self
                            .wait_targets(parent_thread_id, target_selector.as_deref())
                            .await
                            .unwrap_or_else(|_| targets.clone());
                        return wait_activity_result(
                            call,
                            parent_thread_id,
                            parent_turn_id,
                            current_targets,
                        );
                    }
                    _ => {}
                },
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
                Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) | Err(_) => break,
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

    async fn wait_targets(
        &self,
        parent_thread_id: &ThreadId,
        target: Option<&str>,
    ) -> anyhow::Result<Vec<AgentTarget>> {
        if let Some(target) = target {
            return Ok(vec![
                self.resolve_agent_target(parent_thread_id, target).await?,
            ]);
        }
        Ok(self
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
                            && member.status != TeamMemberStatus::Closed
                    })
                    .map(move |member| AgentTarget {
                        team_id: team_id.clone(),
                        member,
                    })
            })
            .collect())
    }
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
            let agent_path = member_agent_path(&target.member);
            let task_name = target
                .member
                .task_name
                .clone()
                .unwrap_or_else(|| target.member.name.clone());
            json!({
                "team_id": target.team_id,
                "member_id": target.member.id,
                "thread_id": target.member.thread_id,
                "agent_path": agent_path,
                "task_name": task_name,
                "status": target.member.status,
                "final_message": target.member.final_message,
                "terminal_error": target.member.terminal_error
            })
        })
        .collect()
}

fn wait_activity_result(
    call: &ToolCallCompleted,
    parent_thread_id: &ThreadId,
    parent_turn_id: &TurnId,
    targets: Vec<AgentTarget>,
) -> ToolResult {
    control_ok(
        call,
        "wait interrupted by mailbox or steer activity".to_string(),
        json!({
            "timed_out": false,
            "activity": "mailbox_or_steer",
            "agents": agent_status_payloads(targets),
            "parent_thread_id": parent_thread_id,
            "parent_turn_id": parent_turn_id
        }),
    )
}
