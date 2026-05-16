use roder_api::context::ContextBlock;
use roder_api::events::{ThreadId, TurnId};
use roder_api::extension::{ExtensionManifest, ExtensionStateKey, ExtensionStateRecord};
use roder_api::inference::{InferenceCapabilities, ModelDescriptor, ProviderAuthType};
use roder_api::policy_mode::PolicyMode;
use roder_api::session::{SessionMetadata, ThreadSnapshot};
use roder_api::subagents::SubagentPermissionMode;
use roder_api::tasks::{TaskHandle, TaskOutputStream};
use roder_api::tools::ToolSpec;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<serde_json::Value>,
    pub method: String,
    pub params: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemStatusResult {
    pub provider: String,
    pub model: String,
    pub extensions: usize,
    pub providers: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionsListResult {
    pub extensions: Vec<ExtensionManifest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderDescriptor {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub auth_type: ProviderAuthType,
    pub auth_label: Option<String>,
    pub authenticated: bool,
    pub auth_detail: Option<String>,
    pub recommended: bool,
    pub sort_order: i32,
    pub capabilities: InferenceCapabilities,
    pub models: Vec<ModelDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvidersListResult {
    pub active_provider: String,
    pub active_model: String,
    pub providers: Vec<ProviderDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSelectParams {
    pub provider: String,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSelectResult {
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexAuthResult {
    pub signed_in: bool,
    pub account_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessionParams {
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessionResult {
    pub thread_id: ThreadId,
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionsListResult {
    pub sessions: Vec<SessionMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionLoadParams {
    pub thread_id: ThreadId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionLoadResult {
    pub snapshot: Option<ThreadSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionGetResult {
    pub mode: PolicyMode,
    pub pending_plan_exit: Option<PendingPlanExitDescriptor>,
    pub pending_tool_approval: Option<PendingToolApprovalDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingPlanExitDescriptor {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub request_id: String,
    pub target_mode: PolicyMode,
    pub plan_summary: Option<String>,
    pub requested_at: OffsetDateTime,
    pub expires_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingToolApprovalDescriptor {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub approval_id: String,
    pub tool_id: String,
    pub tool_name: String,
    pub reason: Option<String>,
    pub requested_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSetModeParams {
    pub mode: PolicyMode,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSetModeResult {
    pub mode: PolicyMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionExitPlanParams {
    pub request_id: String,
    pub approved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionExitPlanResult {
    pub resolved: bool,
    pub mode: PolicyMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionResolveApprovalParams {
    pub approval_id: String,
    pub approved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionResolveApprovalResult {
    pub resolved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionStateGetParams {
    pub key: ExtensionStateKey,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionStateGetResult {
    pub record: Option<ExtensionStateRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionStateSetParams {
    pub record: ExtensionStateRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionStateSetResult {
    pub saved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartTurnParams {
    pub thread_id: ThreadId,
    pub message: String,
    pub provider_override: Option<String>,
    pub model_override: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartTurnResult {
    pub turn_id: TurnId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterruptTurnParams {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolsListResult {
    pub tools: Vec<ToolSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentsListResult {
    pub agents: Vec<AgentDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDescriptor {
    pub agent_type: String,
    pub description: String,
    pub tools: Vec<String>,
    pub model: Option<String>,
    pub permission_mode: SubagentPermissionMode,
    pub max_turns: Option<u32>,
    pub max_result_chars: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandsListResult {
    pub commands: Vec<CommandDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandDescriptor {
    pub name: String,
    pub description: Option<String>,
    pub argument_hint: Option<String>,
    pub source: String,
    pub model: Option<String>,
    pub agent: Option<String>,
    pub has_shell_includes: bool,
    pub has_url_includes: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandsExpandParams {
    pub name: String,
    pub arguments: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandsExpandResult {
    pub name: String,
    pub message: String,
    pub context_blocks: Vec<ContextBlock>,
    pub allowed_tools: Vec<String>,
    pub model: Option<String>,
    pub agent: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandsRunParams {
    pub thread_id: ThreadId,
    pub name: String,
    pub arguments: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandsRunResult {
    pub turn_id: TurnId,
    pub expanded: CommandsExpandResult,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TasksSubmitParams {
    pub executor_id: String,
    #[serde(default)]
    pub input: serde_json::Value,
    pub thread_id: Option<ThreadId>,
    pub turn_id: Option<TurnId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TasksSubmitResult {
    pub task: TaskHandle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TasksListResult {
    pub tasks: Vec<TaskHandle>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TasksGetParams {
    pub task_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskLogEntryDescriptor {
    pub stream: TaskOutputStream,
    pub chunk: String,
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TasksGetResult {
    pub task: Option<TaskHandle>,
    pub logs: Vec<TaskLogEntryDescriptor>,
    pub dropped_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TasksCancelParams {
    pub task_id: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TasksCancelResult {
    pub cancelled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TasksSubscribeResult {
    pub subscribed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptOpenFileParams {
    pub thread_id: ThreadId,
    pub path: String,
    pub line: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptOpenFileResult {
    pub requested: bool,
}
