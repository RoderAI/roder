use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::events::{ThreadId, TurnId};
use crate::inference::TokenUsage;
use crate::subagents::{SubagentExitReason, SubagentLane};

pub type WorkflowRunId = String;
pub type WorkflowScriptId = String;
pub type WorkflowScriptHash = String;
pub type WorkflowPhaseId = String;
pub type WorkflowAgentRunId = String;
pub type WorkflowApprovalId = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum WorkflowScriptSourceKind {
    Generated,
    BuiltIn,
    User,
    Workspace,
    Extension,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowScriptSource {
    pub kind: WorkflowScriptSourceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extension_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowScript {
    pub script_id: WorkflowScriptId,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub source: WorkflowScriptSource,
    pub hash: WorkflowScriptHash,
    pub host_api_version: u32,
    #[serde(default)]
    pub arguments_schema: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    pub limits: WorkflowRunLimits,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum WorkflowRunStatus {
    Drafted,
    AwaitingApproval,
    Queued,
    #[default]
    Running,
    Paused,
    ApprovalWait,
    Completed,
    Failed,
    Stopped,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum WorkflowPhaseStatus {
    #[default]
    Queued,
    Running,
    Completed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum WorkflowAgentStatus {
    #[default]
    Queued,
    Running,
    Completed,
    Failed,
    Timeout,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunLimits {
    pub max_concurrent_agents: u32,
    pub max_agents_per_run: u32,
    pub default_agent_timeout_seconds: u64,
    pub default_run_timeout_seconds: u64,
    pub default_checkpoint_bytes: u64,
    pub max_report_bytes: u64,
}

impl Default for WorkflowRunLimits {
    fn default() -> Self {
        Self {
            max_concurrent_agents: 16,
            max_agents_per_run: 1000,
            default_agent_timeout_seconds: 180,
            default_run_timeout_seconds: 14_400,
            default_checkpoint_bytes: 1_048_576,
            max_report_bytes: 65_536,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowCostEstimate {
    #[serde(default)]
    pub min_child_agents: u32,
    #[serde(default)]
    pub max_child_agents: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_prompt_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_completion_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum WorkflowApprovalDecision {
    RunOnce,
    AlwaysForScriptAndWorkspace,
    Deny,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowApproval {
    pub approval_id: WorkflowApprovalId,
    pub run_id: WorkflowRunId,
    pub script_hash: WorkflowScriptHash,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    pub decision: WorkflowApprovalDecision,
    #[serde(default)]
    pub approved_capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub decided_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowConsent {
    pub script_hash: WorkflowScriptHash,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    pub decision: WorkflowApprovalDecision,
    #[serde(default)]
    pub approved_capabilities: Vec<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub decided_at: OffsetDateTime,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub expires_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowPhase {
    pub phase_id: WorkflowPhaseId,
    pub name: String,
    pub status: WorkflowPhaseStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub queued_agents: u32,
    #[serde(default)]
    pub completed_agents: u32,
    #[serde(default)]
    pub failed_agents: u32,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub started_at: Option<OffsetDateTime>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub completed_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowAgentRun {
    pub agent_id: WorkflowAgentRunId,
    pub phase_id: WorkflowPhaseId,
    pub description: String,
    pub status: WorkflowAgentStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lane: Option<SubagentLane>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<ThreadId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_reason: Option<SubagentExitReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub started_at: Option<OffsetDateTime>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub completed_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunSummary {
    pub run_id: WorkflowRunId,
    pub status: WorkflowRunStatus,
    pub title: String,
    #[serde(default)]
    pub phase_count: u32,
    #[serde(default)]
    pub completed_phase_count: u32,
    #[serde(default)]
    pub agent_count: u32,
    #[serde(default)]
    pub completed_agent_count: u32,
    #[serde(default)]
    pub failed_agent_count: u32,
    #[serde(default)]
    pub concurrency_peak: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report_preview: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRun {
    pub run_id: WorkflowRunId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<ThreadId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    pub script: WorkflowScript,
    pub status: WorkflowRunStatus,
    pub limits: WorkflowRunLimits,
    #[serde(default)]
    pub phases: Vec<WorkflowPhase>,
    #[serde(default)]
    pub agents: Vec<WorkflowAgentRun>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<WorkflowApproval>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_estimate: Option<WorkflowCostEstimate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<WorkflowRunSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub started_at: Option<OffsetDateTime>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub completed_at: Option<OffsetDateTime>,
}

macro_rules! workflow_event {
    ($name:ident { $($field:ident : $ty:ty),* $(,)? }) => {
        #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
        #[serde(rename_all = "camelCase")]
        pub struct $name {
            pub run_id: WorkflowRunId,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub thread_id: Option<ThreadId>,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub turn_id: Option<TurnId>,
            $(pub $field: $ty,)*
            #[serde(with = "time::serde::rfc3339")]
            pub timestamp: OffsetDateTime,
        }
    };
}

workflow_event!(WorkflowRunDrafted { run: WorkflowRun });
workflow_event!(WorkflowApprovalRequested {
    approval_id: WorkflowApprovalId,
    run: WorkflowRun
});
workflow_event!(WorkflowRunApproved {
    approval: WorkflowApproval
});
workflow_event!(WorkflowRunDenied {
    approval: WorkflowApproval
});
workflow_event!(WorkflowRunQueued {
    status: WorkflowRunStatus
});
workflow_event!(WorkflowRunStarted {
    status: WorkflowRunStatus
});
workflow_event!(WorkflowPhaseStarted {
    phase: WorkflowPhase
});
workflow_event!(WorkflowPhaseCompleted {
    phase: WorkflowPhase
});
workflow_event!(WorkflowAgentQueued {
    agent: WorkflowAgentRun
});
workflow_event!(WorkflowAgentStarted {
    agent: WorkflowAgentRun
});
workflow_event!(WorkflowAgentCompleted {
    agent: WorkflowAgentRun
});
workflow_event!(WorkflowAgentFailed {
    agent: WorkflowAgentRun,
    error: String
});
workflow_event!(WorkflowOutputRecorded { phase_id: Option<WorkflowPhaseId>, output: String, truncated: bool });
workflow_event!(WorkflowCheckpointRecorded { phase_id: Option<WorkflowPhaseId>, key: String, byte_count: u64 });
workflow_event!(WorkflowRunPaused { reason: Option<String> });
workflow_event!(WorkflowRunResumed {
    status: WorkflowRunStatus
});
workflow_event!(WorkflowRunStopped { reason: Option<String> });
workflow_event!(WorkflowRunCompleted {
    summary: WorkflowRunSummary
});
workflow_event!(WorkflowRunFailed { error: String, summary: Option<WorkflowRunSummary> });

#[cfg(test)]
mod tests {
    use super::*;

    fn script() -> WorkflowScript {
        WorkflowScript {
            script_id: "script-1".to_string(),
            name: "audit".to_string(),
            description: Some("Audit the repo".to_string()),
            source: WorkflowScriptSource {
                kind: WorkflowScriptSourceKind::Generated,
                path: None,
                command_name: None,
                extension_id: None,
            },
            hash: "sha256:abc".to_string(),
            host_api_version: 1,
            arguments_schema: serde_json::json!({"type": "object"}),
            body: None,
            limits: WorkflowRunLimits::default(),
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn workflow_run_uses_camel_case_wire_shape() {
        let run = WorkflowRun {
            run_id: "run-1".to_string(),
            thread_id: Some("thread-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            script: script(),
            status: WorkflowRunStatus::AwaitingApproval,
            limits: WorkflowRunLimits::default(),
            phases: vec![WorkflowPhase {
                phase_id: "phase-1".to_string(),
                name: "Scout".to_string(),
                status: WorkflowPhaseStatus::Queued,
                description: None,
                queued_agents: 2,
                completed_agents: 0,
                failed_agents: 0,
                started_at: None,
                completed_at: None,
            }],
            agents: vec![WorkflowAgentRun {
                agent_id: "agent-1".to_string(),
                phase_id: "phase-1".to_string(),
                description: "Inspect crate".to_string(),
                status: WorkflowAgentStatus::Queued,
                lane: Some(SubagentLane::Scout),
                model: Some("mock".to_string()),
                thread_id: None,
                turn_id: None,
                usage: None,
                exit_reason: None,
                error: None,
                started_at: None,
                completed_at: None,
            }],
            approval: None,
            cost_estimate: Some(WorkflowCostEstimate {
                min_child_agents: 2,
                max_child_agents: 4,
                estimated_prompt_tokens: Some(1000),
                estimated_completion_tokens: Some(500),
                warning: Some("workflow may fan out".to_string()),
            }),
            summary: None,
            error: None,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
            started_at: None,
            completed_at: None,
        };

        let value = serde_json::to_value(&run).unwrap();

        assert_eq!(value["runId"], "run-1");
        assert_eq!(value["threadId"], "thread-1");
        assert_eq!(value["status"], "awaitingApproval");
        assert_eq!(value["script"]["hostApiVersion"], 1);
        assert_eq!(value["phases"][0]["queuedAgents"], 2);
        assert_eq!(value["agents"][0]["lane"], "scout");
        assert_eq!(value["costEstimate"]["maxChildAgents"], 4);

        let decoded: WorkflowRun = serde_json::from_value(value).unwrap();
        assert_eq!(decoded, run);
    }
}
