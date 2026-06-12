//! Subagent-dispatcher and task-executor payloads for the process-extension
//! protocol (roadmap phase 95).
//!
//! These mirror the canonical [`crate::subagents`] and [`crate::tasks`]
//! contracts so any-language children can dispatch long-running work (for
//! example remote Cursor cloud agents) through ordinary Roder surfaces. The
//! flow matches `inference/streamTurn`: the host sends a request carrying a
//! host-chosen id, the child acks it, then streams `subagents/event` /
//! `tasks/event` notifications until a terminal `completed` / `failed` /
//! `cancelled` event.

use serde::{Deserialize, Serialize};

use crate::events::{ThreadId, TurnId};
use crate::subagents::{SubagentDefinition, SubagentRequest, SubagentResult};
use crate::tasks::{TaskExecutionResult, TaskOutputStream, TaskSpec};

/// `subagents/definitions` params (host -> child).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessSubagentDefinitionsParams {
    pub dispatcher_id: String,
}

/// `subagents/definitions` result (child -> host).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessSubagentDefinitionsResult {
    pub definitions: Vec<SubagentDefinition>,
}

/// `subagents/dispatch` params: a canonical request plus parent provenance
/// and a host-chosen dispatch id the child must echo and stream against.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessSubagentDispatchParams {
    pub dispatcher_id: String,
    pub dispatch_id: String,
    pub parent_thread_id: ThreadId,
    pub parent_turn_id: TurnId,
    pub request: SubagentRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessSubagentDispatchAck {
    pub dispatch_id: String,
}

/// `subagents/event` notification payload (child -> host). The host routes
/// by `dispatch_id`; `completed`/`failed` are terminal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessSubagentEventNotification {
    pub dispatch_id: String,
    pub event: ProcessSubagentEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ProcessSubagentEvent {
    /// Non-terminal progress (e.g. a remote agent lifecycle transition).
    /// Payloads must already be redacted by the child.
    Status {
        status: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    Completed {
        result: Box<SubagentResult>,
    },
    Failed {
        error: String,
    },
}

/// `subagents/cancel` params (host -> child request; result is empty).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessSubagentCancelParams {
    pub dispatcher_id: String,
    pub dispatch_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// `tasks/spec` params (host -> child).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessTaskSpecParams {
    pub executor_id: String,
}

/// `tasks/spec` result (child -> host).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessTaskSpecResult {
    pub spec: TaskSpec,
}

/// `tasks/execute` params: canonical task input plus execution provenance
/// and a host-chosen execution id the child must echo and stream against.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessTaskExecuteParams {
    pub executor_id: String,
    pub execution_id: String,
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<ThreadId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessTaskExecuteAck {
    pub execution_id: String,
}

/// `tasks/event` notification payload (child -> host). The host routes by
/// `execution_id`; `completed`/`failed` are terminal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessTaskEventNotification {
    pub execution_id: String,
    pub event: ProcessTaskEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ProcessTaskEvent {
    /// Incremental output forwarded into the task's output sink.
    Output {
        stream: TaskOutputStream,
        chunk: String,
    },
    Completed {
        result: TaskExecutionResult,
    },
    Failed {
        error: String,
    },
}

/// `tasks/cancel` params (host -> child request; result is empty).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessTaskCancelParams {
    pub executor_id: String,
    pub execution_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}
