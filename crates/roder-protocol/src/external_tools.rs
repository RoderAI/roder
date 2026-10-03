//! Authenticated hosted executor binding for client-executed tools.

use serde::{Deserialize, Serialize};

/// A server-issued binding to one authenticated WebSocket connection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolExecutorLease {
    pub thread_id: String,
    pub lease_id: String,
    pub generation: u64,
    pub contract_version: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolsBindExecutorParams {
    pub thread_id: String,
    #[serde(default)]
    pub takeover: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolsUnbindExecutorParams {
    pub executor: ToolExecutorLease,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolsExecutionReadParams {
    pub executor: ToolExecutorLease,
    pub request_id: String,
}

/// Metadata only; tool inputs and result bodies are deliberately excluded.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolExecutionState {
    pub thread_id: String,
    pub turn_id: String,
    pub request_id: String,
    pub tool_name: String,
    pub state: String,
    pub is_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsBindExecutorResult {
    pub executor: ToolExecutorLease,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsUnbindExecutorResult {
    pub unbound: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsExecutionReadResult {
    pub execution: Option<ToolExecutionState>,
}
