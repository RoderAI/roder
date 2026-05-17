use roder_api::conversation::InputImage;
use roder_api::events::{ThreadId, TurnId};
use roder_api::extension::ExtensionManifest;
use roder_api::inference::{
    HostedWebSearchMode, InferenceCapabilities, ModelDescriptor, ProviderAuthType,
};
use roder_api::policy_mode::PolicyMode;
use roder_api::session::{SessionMetadata, ThreadSnapshot};
use roder_api::subagents::SubagentPermissionMode;
use roder_api::teams::{TeamChannelId, TeamMemberId, TeamMessage, TeamSnapshot};
use roder_api::tools::ToolSpec;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    #[serde(default = "default_jsonrpc_version")]
    pub jsonrpc: String,
    pub id: Option<serde_json::Value>,
    pub method: String,
    pub params: Option<serde_json::Value>,
}

fn default_jsonrpc_version() -> String {
    "2.0".to_string()
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
    pub reasoning: String,
    pub web_search: WebSearchSettings,
    pub extensions: usize,
    pub providers: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WebSearchSettings {
    pub mode: HostedWebSearchMode,
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
    pub active_reasoning: String,
    pub providers: Vec<ProviderDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopModelDescriptor {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "modelProvider")]
    pub model_provider: String,
    #[serde(rename = "defaultReasoningEffort")]
    pub default_reasoning_effort: Option<String>,
    #[serde(rename = "reasoningEfforts")]
    pub reasoning_efforts: Vec<String>,
    #[serde(rename = "isDefault")]
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelListResult {
    pub models: Vec<DesktopModelDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSelectParams {
    pub provider: String,
    pub model: Option<String>,
    pub reasoning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSelectResult {
    pub provider: String,
    pub model: String,
    pub reasoning: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsGetResult {
    pub web_search: WebSearchSettings,
    pub default_mode: PolicyMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsSetWebSearchParams {
    pub mode: HostedWebSearchMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsSetWebSearchResult {
    pub web_search: WebSearchSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsSetDefaultModeParams {
    pub mode: PolicyMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsSetDefaultModeResult {
    pub default_mode: PolicyMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexAuthResult {
    pub signed_in: bool,
    pub account_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessionParams {
    pub title: Option<String>,
    pub workspace: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessionResult {
    pub thread_id: ThreadId,
    pub provider: String,
    pub model: String,
    pub reasoning: String,
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
pub struct SessionResolveUserInputParams {
    pub request_id: String,
    pub answers: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionResolveUserInputResult {
    pub resolved: bool,
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
pub struct StartTurnParams {
    pub thread_id: ThreadId,
    pub message: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<InputImage>,
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
pub struct SteerTurnParams {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub message: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<InputImage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SteerTurnResult {
    pub turn_id: TurnId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamStartParams {
    pub name: Option<String>,
    pub workspace: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamStartResult {
    pub team: TeamSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamListResult {
    pub teams: Vec<TeamSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamReadParams {
    pub team_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamReadResult {
    pub team: TeamSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamChannelMessageParams {
    pub team_id: String,
    pub channel_id: TeamChannelId,
    pub text: String,
    pub author_member_id: Option<TeamMemberId>,
    pub project_context: Option<String>,
    pub thread_ts: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamChannelMessageResult {
    pub team: TeamSnapshot,
    pub message: TeamMessage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamMemberMessageParams {
    pub team_id: String,
    pub member_id: TeamMemberId,
    pub channel_id: Option<TeamChannelId>,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamMemberMessageResult {
    pub team: TeamSnapshot,
    pub member: roder_api::teams::TeamMember,
    pub message: TeamMessage,
    pub turn_id: Option<TurnId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamMemberInterruptParams {
    pub team_id: String,
    pub member_id: TeamMemberId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamMemberInterruptResult {
    pub team: TeamSnapshot,
    pub member: roder_api::teams::TeamMember,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamSchedulerSetParams {
    pub team_id: String,
    pub running: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamSchedulerSetResult {
    pub team: TeamSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamCleanupParams {
    pub team_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamCleanupResult {
    pub team_id: String,
    pub cleaned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolsListResult {
    pub tools: Vec<ToolSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallParams {
    pub thread_id: ThreadId,
    pub tool_name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallResult {
    pub text: String,
    pub data: serde_json::Value,
    pub is_error: bool,
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
