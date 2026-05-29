use std::fmt;

use roder_api::dynamic_workflows::{WorkflowRunId, WorkflowRunLimits};
use serde::{Deserialize, Serialize};

pub const WORKFLOW_HOST_API_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowDefinition {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub arguments_schema: serde_json::Value,
    #[serde(default)]
    pub phases: Vec<String>,
    pub host_api_version: u32,
    pub limits: WorkflowRunLimits,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRuntimeOptions {
    pub max_loop_iterations: u64,
    pub max_promise_drains: usize,
    pub max_report_bytes: u64,
    pub limits: WorkflowRunLimits,
}

impl Default for WorkflowRuntimeOptions {
    fn default() -> Self {
        let limits = WorkflowRunLimits::default();
        Self {
            max_loop_iterations: 100_000,
            max_promise_drains: 8,
            max_report_bytes: limits.max_report_bytes,
            limits,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunInput {
    pub run_id: WorkflowRunId,
    #[serde(default)]
    pub arguments: serde_json::Value,
    #[serde(default)]
    pub abort_before_start: bool,
    #[serde(default)]
    pub checkpoints: Vec<crate::host_api::WorkflowCheckpoint>,
}

impl WorkflowRunInput {
    pub fn new(run_id: impl Into<WorkflowRunId>) -> Self {
        Self {
            run_id: run_id.into(),
            arguments: serde_json::Value::Object(Default::default()),
            abort_before_start: false,
            checkpoints: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowRuntimeErrorKind {
    MissingDefinition,
    InvalidMetadata,
    UnsupportedHostApiVersion,
    DeniedAmbientApi,
    ScriptExecution,
    LimitExceeded,
    Aborted,
    Store,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowRuntimeError {
    kind: WorkflowRuntimeErrorKind,
    message: String,
}

impl WorkflowRuntimeError {
    pub fn new(kind: WorkflowRuntimeErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> WorkflowRuntimeErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for WorkflowRuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}

impl std::error::Error for WorkflowRuntimeError {}

pub type WorkflowRuntimeResult<T> = Result<T, WorkflowRuntimeError>;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RawWorkflowDefinition {
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub arguments_schema: serde_json::Value,
    #[serde(default)]
    pub phases: Vec<String>,
    #[serde(default)]
    pub host_api_version: Option<u32>,
    #[serde(default)]
    pub limits: WorkflowRunLimitsPatch,
}

impl RawWorkflowDefinition {
    pub fn into_definition(
        self,
        base_limits: &WorkflowRunLimits,
    ) -> WorkflowRuntimeResult<WorkflowDefinition> {
        let Some(name) = self.name.filter(|name| !name.trim().is_empty()) else {
            return Err(WorkflowRuntimeError::new(
                WorkflowRuntimeErrorKind::InvalidMetadata,
                "workflow metadata must include a non-empty name",
            ));
        };
        let host_api_version = self.host_api_version.unwrap_or(WORKFLOW_HOST_API_VERSION);
        if host_api_version != WORKFLOW_HOST_API_VERSION {
            return Err(WorkflowRuntimeError::new(
                WorkflowRuntimeErrorKind::UnsupportedHostApiVersion,
                format!("unsupported workflow host API version {host_api_version}"),
            ));
        }

        Ok(WorkflowDefinition {
            name,
            description: self.description,
            arguments_schema: self.arguments_schema,
            phases: self.phases,
            host_api_version,
            limits: self.limits.apply_to(base_limits.clone()),
        })
    }
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkflowRunLimitsPatch {
    pub max_concurrent_agents: Option<u32>,
    pub max_agents_per_run: Option<u32>,
    pub default_agent_timeout_seconds: Option<u64>,
    pub default_run_timeout_seconds: Option<u64>,
    pub default_checkpoint_bytes: Option<u64>,
    pub max_report_bytes: Option<u64>,
}

impl WorkflowRunLimitsPatch {
    fn apply_to(self, mut limits: WorkflowRunLimits) -> WorkflowRunLimits {
        if let Some(value) = self.max_concurrent_agents {
            limits.max_concurrent_agents = value;
        }
        if let Some(value) = self.max_agents_per_run {
            limits.max_agents_per_run = value;
        }
        if let Some(value) = self.default_agent_timeout_seconds {
            limits.default_agent_timeout_seconds = value;
        }
        if let Some(value) = self.default_run_timeout_seconds {
            limits.default_run_timeout_seconds = value;
        }
        if let Some(value) = self.default_checkpoint_bytes {
            limits.default_checkpoint_bytes = value;
        }
        if let Some(value) = self.max_report_bytes {
            limits.max_report_bytes = value;
        }
        limits
    }
}
