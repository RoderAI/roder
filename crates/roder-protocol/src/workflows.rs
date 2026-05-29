use roder_api::dynamic_workflows::{
    WorkflowAgentRun, WorkflowAgentRunId, WorkflowApproval, WorkflowApprovalDecision, WorkflowRun,
    WorkflowRunId, WorkflowRunSummary, WorkflowScript, WorkflowScriptId, WorkflowScriptSourceKind,
};
use roder_api::events::{ThreadId, TurnId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowsPlanParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<ThreadId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    #[serde(default)]
    pub arguments: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowsPlanResult {
    pub run: WorkflowRun,
    pub approval_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowsApproveParams {
    pub run_id: WorkflowRunId,
    pub decision: WorkflowApprovalDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowsApproveResult {
    pub run: WorkflowRun,
    pub approval: WorkflowApproval,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowsListParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<ThreadId>,
    #[serde(default)]
    pub include_terminal: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowsListResult {
    pub runs: Vec<WorkflowRunSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowsGetParams {
    pub run_id: WorkflowRunId,
    #[serde(default)]
    pub include_script_body: bool,
    #[serde(default)]
    pub include_agents: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowsGetResult {
    pub run: WorkflowRun,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowsPauseParams {
    pub run_id: WorkflowRunId,
    #[serde(default)]
    pub cancel_running_agents: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowsPauseResult {
    pub run: WorkflowRun,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowsResumeParams {
    pub run_id: WorkflowRunId,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowsResumeResult {
    pub run: WorkflowRun,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowsStopParams {
    pub run_id: WorkflowRunId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowsStopResult {
    pub run: WorkflowRun,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowsRestartAgentParams {
    pub run_id: WorkflowRunId,
    pub agent_id: WorkflowAgentRunId,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowsRestartAgentResult {
    pub run: WorkflowRun,
    pub agent: WorkflowAgentRun,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum WorkflowsSaveScope {
    User,
    Workspace,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowsSaveParams {
    pub run_id: WorkflowRunId,
    pub name: String,
    pub scope: WorkflowsSaveScope,
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowsSaveResult {
    pub script: WorkflowScript,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowsScriptsListParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    #[serde(default)]
    pub include_user: bool,
    #[serde(default)]
    pub include_builtin: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowsScriptsListResult {
    pub scripts: Vec<WorkflowScript>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowsScriptsReadParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script_id: Option<WorkflowScriptId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<WorkflowScriptSourceKind>,
    #[serde(default)]
    pub include_body: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowsScriptsReadResult {
    pub script: WorkflowScript,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowsScriptsDeleteParams {
    pub script_id: WorkflowScriptId,
    #[serde(default)]
    pub delete_file: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowsScriptsDeleteResult {
    pub script_id: WorkflowScriptId,
    pub deleted: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_protocol_params_use_camel_case_fields() {
        let plan: WorkflowsPlanParams = serde_json::from_value(serde_json::json!({
            "threadId": "thread-1",
            "turnId": "turn-1",
            "prompt": "use a workflow to audit auth",
            "workspace": "/tmp/repo",
            "arguments": { "scope": "auth" }
        }))
        .unwrap();
        assert_eq!(plan.thread_id.as_deref(), Some("thread-1"));
        assert_eq!(plan.turn_id.as_deref(), Some("turn-1"));

        let approve: WorkflowsApproveParams = serde_json::from_value(serde_json::json!({
            "runId": "run-1",
            "decision": "alwaysForScriptAndWorkspace",
            "reason": "trusted generated workflow"
        }))
        .unwrap();
        assert_eq!(
            approve.decision,
            WorkflowApprovalDecision::AlwaysForScriptAndWorkspace
        );

        let restart: WorkflowsRestartAgentParams = serde_json::from_value(serde_json::json!({
            "runId": "run-1",
            "agentId": "agent-1"
        }))
        .unwrap();
        assert_eq!(restart.agent_id, "agent-1");

        let read: WorkflowsScriptsReadParams = serde_json::from_value(serde_json::json!({
            "name": "deep-research",
            "source": "builtIn",
            "includeBody": true
        }))
        .unwrap();
        assert_eq!(read.source, Some(WorkflowScriptSourceKind::BuiltIn));
        assert!(read.include_body);

        let delete = serde_json::to_value(WorkflowsScriptsDeleteResult {
            script_id: "script-1".to_string(),
            deleted: true,
        })
        .unwrap();
        assert_eq!(delete["scriptId"], "script-1");
    }
}
