use serde::{Deserialize, Serialize};

use crate::model::WorkflowDefinition;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowAgentLaunch {
    pub index: u32,
    pub role: String,
    pub lane: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    pub description: String,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub timeout_seconds: u64,
    #[serde(default)]
    pub input: serde_json::Value,
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowCheckpoint {
    pub key: String,
    #[serde(default)]
    pub value: serde_json::Value,
    pub byte_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowExecution {
    pub definition: WorkflowDefinition,
    pub report: String,
    #[serde(default)]
    pub phases: Vec<String>,
    #[serde(default)]
    pub agent_launches: Vec<WorkflowAgentLaunch>,
    #[serde(default)]
    pub checkpoints: Vec<WorkflowCheckpoint>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RawWorkflowExecution {
    pub report: String,
    #[serde(default)]
    pub phases: Vec<String>,
    #[serde(default)]
    pub agent_launches: Vec<WorkflowAgentLaunch>,
    #[serde(default)]
    pub checkpoints: Vec<WorkflowCheckpoint>,
}
