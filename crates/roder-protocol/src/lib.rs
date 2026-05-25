pub mod methods;
pub mod schema;

use roder_api::artifacts::{
    ArtifactGrepPage, ArtifactReadPage, ArtifactTailPage, ContextArtifactDescriptor,
    ContextArtifactKind,
};
use roder_api::automations::{
    AutomationConcurrencyPolicy, AutomationDefinition, AutomationId, AutomationProject,
    AutomationRunId, AutomationRunState, AutomationRunSummary, AutomationSchedule, CatchUpPolicy,
};
use roder_api::capabilities::CapabilityStatus;
use roder_api::code_index::{
    CodeChunk, CodeIndexGenerationId, CodeIndexSearchResponse, CodeIndexStats, CodeIndexStatus,
    ContentProof,
};
use roder_api::context::ContextBlock;
use roder_api::conversation::InputImage;
use roder_api::discovery::{
    DiscoveryCatalog, DiscoveryCatalogGroup, DiscoveryCatalogItem, DiscoveryPromotionRecord,
};
use roder_api::events::{ThreadId, TurnId};
use roder_api::extension::{ExtensionId, ExtensionManifest};
pub use roder_api::goals::{ThreadGoal, ThreadGoalStatus};
use roder_api::inference::{
    HostedWebSearchMode, InferenceCapabilities, ModelDescriptor, ProviderAuthType,
};
use roder_api::marketplace::{
    DedupedMarketplacePlugin, DefaultMarketplaceSelection, InstalledPluginRecord,
    MarketplaceDescriptor, MarketplaceKind, MarketplacePluginEntry, MarketplaceSource,
};
use roder_api::media::{MediaArtifact, MediaArtifactId, MediaAttachment, MediaPreview};
use roder_api::memory::{
    MemoryId, MemoryProviderSelection, MemoryRecord, MemoryScope, MemorySearchResult,
};
use roder_api::plan_review::{
    HunkId, HunkRecord, PagedHunkDiff, PlanComment, PlanCommentAnchor, PlanReview, PlanReviewId,
    PlanRewrite,
};
use roder_api::policy_mode::PolicyMode;
use roder_api::processes::{ProcessDescriptor, ProcessId, ProcessOutput, ProcessStopResult};
use roder_api::retrieval::{RetrievalMeasuredOutcome, RetrievalMode, RetrievalRoutePlan};
use roder_api::skills::{Skill, SkillDescriptor, SkillExposure, SkillSelector};
use roder_api::subagents::SubagentPermissionMode;
use roder_api::tasks::{TaskHandle, TaskOutputStream};
use roder_api::teams::{
    AgentTeamDisplayMode, TeamId, TeamMailboxMessage, TeamMemberDescriptor, TeamMemberId,
    TeamMemberStatus, TeamTaskDescriptor,
};
use roder_api::tools::ToolSpec;
use roder_api::trace::{SubagentTraceDelta, SubagentTraceId, SubagentTraceSummary};
use roder_api::workflow::{
    WorkflowImportDecision, WorkflowImportItem, WorkflowImportScan, WorkflowImportState,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::collections::HashMap;
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
pub struct JsonRpcNotification {
    #[serde(default = "default_jsonrpc_version")]
    pub jsonrpc: String,
    pub method: String,
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    pub provider: String,
    pub model: String,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopThreadStatus {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub active_flags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopThread {
    pub id: ThreadId,
    pub session_id: ThreadId,
    pub preview: String,
    pub model_provider: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub status: DesktopThreadStatus,
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turns: Option<Vec<DesktopTurn>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopTurn {
    pub id: TurnId,
    pub items: Vec<DesktopItem>,
    pub items_view: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopItem {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadStartParams {
    pub model: Option<String>,
    pub model_provider: Option<String>,
    pub cwd: Option<String>,
    #[serde(default)]
    pub ephemeral: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadStartResult {
    pub thread: DesktopThread,
    pub model: String,
    pub model_provider: String,
    pub cwd: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadListParams {
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadListResult {
    pub data: Vec<DesktopThread>,
    pub next_cursor: Option<String>,
    pub backwards_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadReadParams {
    pub thread_id: ThreadId,
    #[serde(default)]
    pub include_turns: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadReadResult {
    pub thread: Option<DesktopThread>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadArchiveParams {
    pub thread_id: ThreadId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadArchiveResult {
    pub thread_id: ThreadId,
    pub archived: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadGoalGetParams {
    pub thread_id: ThreadId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadGoalGetResult {
    pub goal: Option<ThreadGoal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadGoalSetParams {
    pub thread_id: ThreadId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objective: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<ThreadGoalStatus>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_goal_token_budget_patch"
    )]
    pub token_budget: Option<Option<i64>>,
}

fn deserialize_goal_token_budget_patch<'de, D>(
    deserializer: D,
) -> Result<Option<Option<i64>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Null => Ok(Some(None)),
        value => i64::deserialize(value)
            .map(Some)
            .map(Some)
            .map_err(serde::de::Error::custom),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadGoalSetResult {
    pub goal: Option<ThreadGoal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadGoalClearParams {
    pub thread_id: ThreadId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadGoalClearResult {
    pub cleared: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnInputItem {
    #[serde(rename = "type")]
    pub kind: String,
    pub text: Option<String>,
    pub path: Option<String>,
    #[serde(default, alias = "image_url", skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnStartParams {
    pub thread_id: ThreadId,
    #[serde(default)]
    pub input: Vec<TurnInputItem>,
    pub prompt: Option<String>,
    #[serde(default)]
    pub task_ledger_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnStartResult {
    pub turn_id: TurnId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnSteerParams {
    pub thread_id: ThreadId,
    pub expected_turn_id: TurnId,
    #[serde(default)]
    pub input: Vec<TurnInputItem>,
    pub prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnSteerResult {
    pub turn_id: TurnId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnInterruptParams {
    pub thread_id: ThreadId,
    pub turn_id: Option<TurnId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnInterruptResult {
    pub turn_id: Option<TurnId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelListResult {
    pub models: Vec<DesktopModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopModel {
    pub id: String,
    pub name: String,
    pub model_provider: String,
    pub default_reasoning_effort: Option<String>,
    pub reasoning_efforts: Vec<String>,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryListParams {
    pub scope: Option<MemoryScope>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryListResult {
    pub memories: Vec<MemoryRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryReadParams {
    pub memory_id: MemoryId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryReadResult {
    pub memory: Option<MemoryRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySaveParams {
    pub scope: MemoryScope,
    pub text: String,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySaveResult {
    pub memory_id: MemoryId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryUpdateParams {
    pub memory_id: MemoryId,
    pub text: String,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryDeleteParams {
    pub memory_id: MemoryId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryDeleteResult {
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryQueryParams {
    pub scope: Option<MemoryScope>,
    pub text: String,
    pub limit: Option<usize>,
    #[serde(default)]
    pub include_global: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryQueryResult {
    pub results: Vec<MemorySearchResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryProviderListResult {
    pub providers: Vec<roder_api::embeddings::EmbeddingProviderDescriptor>,
    pub selected: MemoryProviderSelection,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryProviderSetParams {
    pub provider_id: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryRecallPreviewParams {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub scope: Option<MemoryScope>,
    pub text: String,
    pub limit: Option<usize>,
    #[serde(default)]
    pub include_global: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryRecallPreviewResult {
    pub citations: Vec<roder_api::memory::MemoryCitation>,
    pub results: Vec<MemorySearchResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadStartedNotification {
    pub thread: DesktopThread,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadStatusChangedNotification {
    pub thread_id: ThreadId,
    pub status: DesktopThreadStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadGoalUpdatedNotification {
    pub thread_id: ThreadId,
    pub goal: ThreadGoal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadGoalClearedNotification {
    pub thread_id: ThreadId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnStartedNotification {
    pub thread_id: ThreadId,
    pub turn: DesktopTurn,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnCompletedNotification {
    pub thread_id: ThreadId,
    pub turn: DesktopTurn,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnDeadlineExceededNotification {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub deadline: time::OffsetDateTime,
    pub partial_result: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnPartialResultNotification {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemStartedNotification {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub item: DesktopItem,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemCompletedNotification {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub item: DesktopItem,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMessageDeltaNotification {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub item_id: String,
    pub delta: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequestedNotification {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub approval_id: String,
    pub tool_id: String,
    pub tool_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalResolvedNotification {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub approval_id: String,
    pub tool_id: String,
    pub tool_name: String,
    pub approved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserInputRequestedNotification {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub request_id: String,
    pub questions: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserInputResolvedNotification {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub request_id: String,
    pub answers: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationRequiredNotification {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub reason: String,
    pub changed_files: Vec<String>,
    pub tool_evidence: Vec<String>,
    pub tests_run: Vec<String>,
    pub open_gaps: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationCompletedNotification {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub passed: bool,
    pub changed_files: Vec<String>,
    pub tool_evidence: Vec<String>,
    pub tests_run: Vec<String>,
    pub open_gaps: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationSkippedNotification {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationRunNotification {
    pub run: AutomationRunSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationRunFailedNotification {
    pub run: AutomationRunSummary,
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationRunSkippedNotification {
    pub run: AutomationRunSummary,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanExitRequestedNotification {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub request_id: String,
    pub target_mode: roder_api::policy_mode::PolicyMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanExitResolvedNotification {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub request_id: String,
    pub approved: bool,
    pub target_mode: roder_api::policy_mode::PolicyMode,
    pub resolved_mode: roder_api::policy_mode::PolicyMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FsReadFileParams {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FsReadFileResponse {
    pub data_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FsReadDirectoryParams {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FsReadDirectoryEntry {
    pub file_name: String,
    pub is_directory: bool,
    pub is_file: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FsReadDirectoryResponse {
    pub entries: Vec<FsReadDirectoryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandExecParams {
    pub command: Vec<String>,
    pub process_id: Option<String>,
    #[serde(default)]
    pub tty: bool,
    #[serde(default)]
    pub stream_stdin: bool,
    #[serde(default)]
    pub stream_stdout_stderr: bool,
    pub output_bytes_cap: Option<usize>,
    #[serde(default)]
    pub disable_output_cap: bool,
    #[serde(default)]
    pub disable_timeout: bool,
    pub timeout_ms: Option<u64>,
    pub cwd: Option<String>,
    pub env: Option<HashMap<String, Option<String>>>,
    #[serde(default)]
    pub size: Option<serde_json::Value>,
    #[serde(default)]
    pub sandbox_policy: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandExecResponse {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout_artifact: Option<ContextArtifactDescriptor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr_artifact: Option<ContextArtifactDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandExecOutputDeltaNotification {
    pub process_id: String,
    pub stream: String,
    pub delta_base64: String,
    pub cap_reached: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamDescriptor {
    pub id: TeamId,
    pub lead_thread_id: ThreadId,
    pub display_mode: AgentTeamDisplayMode,
    pub members: Vec<TeamMemberDescriptor>,
    pub tasks: Vec<TeamTaskDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamStartMemberParams {
    pub name: String,
    pub model_provider: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamStartParams {
    pub lead_thread_id: Option<ThreadId>,
    pub display_mode: Option<AgentTeamDisplayMode>,
    #[serde(default)]
    pub members: Vec<TeamStartMemberParams>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamStartResult {
    pub team: TeamDescriptor,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamListParams {
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamListResult {
    pub data: Vec<TeamDescriptor>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamReadParams {
    pub team_id: TeamId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamReadResult {
    pub team: Option<TeamDescriptor>,
    pub messages: Vec<TeamMailboxMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamMemberStartParams {
    pub team_id: TeamId,
    pub name: String,
    pub model_provider: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamMemberStartResult {
    pub member: TeamMemberDescriptor,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamMemberMessageParams {
    pub team_id: TeamId,
    pub member_id: TeamMemberId,
    pub text: String,
    pub expected_turn_id: Option<TurnId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamMemberMessageResult {
    pub turn_id: TurnId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamMemberInterruptParams {
    pub team_id: TeamId,
    pub member_id: TeamMemberId,
    pub turn_id: Option<TurnId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamMemberInterruptResult {
    pub interrupted: bool,
    pub turn_id: Option<TurnId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamMemberFocusParams {
    pub team_id: TeamId,
    pub member_id: TeamMemberId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamMemberFocusResult {
    pub focused_member_id: TeamMemberId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamCleanupParams {
    pub team_id: TeamId,
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamCleanupResult {
    pub cleaned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamStartedNotification {
    pub team: TeamDescriptor,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamMemberStartedNotification {
    pub team_id: TeamId,
    pub member: TeamMemberDescriptor,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamMemberStatusChangedNotification {
    pub team_id: TeamId,
    pub member_id: TeamMemberId,
    pub status: TeamMemberStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamMemberMessageDeltaNotification {
    pub team_id: TeamId,
    pub member_id: TeamMemberId,
    pub turn_id: TurnId,
    pub delta: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamMemberCompletedNotification {
    pub team_id: TeamId,
    pub member_id: TeamMemberId,
    pub turn_id: Option<TurnId>,
    pub status: TeamMemberStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamDisplayModeChangedNotification {
    pub team_id: TeamId,
    pub display_mode: AgentTeamDisplayMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamTaskChangedNotification {
    pub team_id: TeamId,
    pub task: TeamTaskDescriptor,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamCleanupCompletedNotification {
    pub team_id: TeamId,
    pub forced: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunnerStatus {
    pub destination_id: String,
    pub provider_id: String,
    pub state: String,
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunnerProviderDescriptor {
    pub provider_id: String,
    pub capabilities: roder_api::remote_runner::RunnerCapabilities,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunnersListResult {
    pub active: Option<RunnerStatus>,
    pub providers: Vec<RunnerProviderDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunnersSelectParams {
    pub destination_id: String,
    pub provider_id: Option<String>,
    #[serde(default)]
    pub config: serde_json::Value,
    #[serde(default)]
    pub manifest: roder_api::remote_runner::RunnerManifest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunnersSelectResult {
    pub active: Option<RunnerStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunnersSessionResult {
    pub active: Option<RunnerStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunnersSnapshotResult {
    pub snapshot: Option<roder_api::remote_runner::RunnerSnapshotRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunnersDeleteResult {
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunnersPortsResult {
    pub ports: Vec<roder_api::remote_runner::RunnerPortResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WebSearchSettings {
    pub mode: HostedWebSearchMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearchIndexSettings {
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShellSettings {
    pub shell: String,
    pub options: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SearchIndexStatusState {
    Disabled,
    Missing,
    Building,
    Ready,
    Stale,
    Failed,
    Cleared,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SearchIndexStatus {
    pub state: SearchIndexStatusState,
    pub enabled: bool,
    pub workspace: String,
    pub store_dir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_time_ms: Option<u64>,
    pub stale: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SearchIndexStatusParams {
    pub workspace: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SearchIndexWarmupParams {
    pub workspace: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SearchIndexRebuildParams {
    pub workspace: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SearchIndexClearParams {
    pub workspace: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchIndexStatusResult {
    pub status: SearchIndexStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchIndexWarmupResult {
    pub status: SearchIndexStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchIndexRebuildResult {
    pub status: SearchIndexStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchIndexClearResult {
    pub status: SearchIndexStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchIndexStatusNotification {
    pub status: SearchIndexStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodeIndexStatusView {
    pub status: CodeIndexStatus,
    pub workspace: String,
    pub store_path: String,
    pub generation_id: Option<CodeIndexGenerationId>,
    pub root_hash: Option<String>,
    pub stale: bool,
    pub stats: CodeIndexStats,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CodeIndexStatusParams {
    pub workspace: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CodeIndexRebuildParams {
    pub workspace: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeIndexSearchParams {
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeIndexReadChunkParams {
    pub chunk_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    #[serde(default)]
    pub offset: Option<usize>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub include_source: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CodeIndexProofsListParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeIndexStatusResult {
    pub status: CodeIndexStatusView,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeIndexRebuildResult {
    pub status: CodeIndexStatusView,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeIndexSearchResultEnvelope {
    pub status: CodeIndexStatusView,
    pub response: CodeIndexSearchResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeIndexChunkReadPage {
    pub chunk: CodeChunk,
    pub text: String,
    pub offset: usize,
    pub limit: usize,
    pub total_bytes: usize,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeIndexReadChunkResult {
    pub status: CodeIndexStatusView,
    pub page: CodeIndexChunkReadPage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeIndexProofsListResult {
    pub status: CodeIndexStatusView,
    pub proofs: Vec<ContentProof>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeIndexStatusNotification {
    pub status: CodeIndexStatusView,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionsListResult {
    pub extensions: Vec<ExtensionManifest>,
    #[serde(default)]
    pub capability_statuses: std::collections::BTreeMap<ExtensionId, Vec<CapabilityStatus>>,
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
pub struct ProviderConfigureParams {
    pub provider: String,
    pub api_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfigureResult {
    pub provider: String,
    pub authenticated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentTracesListParams {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentTracesListResult {
    pub traces: Vec<SubagentTraceSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentTraceReadParams {
    pub thread_id: ThreadId,
    pub trace_id: SubagentTraceId,
    #[serde(default)]
    pub offset: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentTraceReadResult {
    pub trace_id: SubagentTraceId,
    pub events: Vec<SubagentTraceDelta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanReviewReadParams {
    pub thread_id: ThreadId,
    pub review_id: PlanReviewId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanReviewReadResult {
    pub review: Option<PlanReview>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanReviewCommentParams {
    pub thread_id: ThreadId,
    pub review_id: PlanReviewId,
    pub anchor: PlanCommentAnchor,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanReviewCommentResult {
    pub comment: PlanComment,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanReviewRewriteParams {
    pub thread_id: ThreadId,
    pub review_id: PlanReviewId,
    pub replacement_markdown: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanReviewRewriteResult {
    pub rewrite: PlanRewrite,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanReviewApproveParams {
    pub thread_id: ThreadId,
    pub review_id: PlanReviewId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanReviewApproveResult {
    pub approved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanReviewRejectParams {
    pub thread_id: ThreadId,
    pub review_id: PlanReviewId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanReviewRejectResult {
    pub rejected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HunkListParams {
    pub thread_id: ThreadId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_id: Option<PlanReviewId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HunkListResult {
    pub hunks: Vec<HunkRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HunkReadParams {
    pub thread_id: ThreadId,
    pub hunk_id: HunkId,
    #[serde(default)]
    pub offset: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HunkReadResult {
    pub page: Option<PagedHunkDiff>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HunkRollbackParams {
    pub thread_id: ThreadId,
    pub hunk_id: HunkId,
    #[serde(default)]
    pub confirmed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HunkRollbackResult {
    pub rolled_back: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaListParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<ThreadId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<roder_api::media::MediaKind>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaListResult {
    pub artifacts: Vec<MediaArtifact>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactListParams {
    pub thread_id: ThreadId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ContextArtifactKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactListResult {
    pub artifacts: Vec<ContextArtifactDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactReadParams {
    pub thread_id: ThreadId,
    pub artifact_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_line: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactReadResult {
    pub page: ArtifactReadPage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactGrepParams {
    pub thread_id: ThreadId,
    pub artifact_id: String,
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactGrepResult {
    pub page: ArtifactGrepPage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactTailParams {
    pub thread_id: ThreadId,
    pub artifact_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactTailResult {
    pub page: ArtifactTailPage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactDeleteParams {
    pub thread_id: ThreadId,
    pub artifact_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactDeleteResult {
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryGroupsParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryGroupsResult {
    pub catalog_id: String,
    pub title: String,
    pub hidden_item_count: u64,
    pub groups: Vec<DiscoveryCatalogGroup>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverySearchParams {
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverySearchResult {
    pub query: String,
    pub items: Vec<DiscoveryCatalogItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryReadParams {
    pub item_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_line: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub promote: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<ThreadId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryReadPage {
    pub text: String,
    pub start_line: usize,
    pub end_line: usize,
    pub total_lines: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryReadResult {
    pub item: DiscoveryCatalogItem,
    pub page: DiscoveryReadPage,
    pub promoted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryRefreshResult {
    pub catalog: DiscoveryCatalog,
    pub catalog_root: String,
    pub session_state_dir: String,
    pub written_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryPromoteParams {
    pub item_id: String,
    pub thread_id: ThreadId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryPromoteResult {
    pub record: DiscoveryPromotionRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryPromotedListParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<ThreadId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryPromotedListResult {
    pub records: Vec<DiscoveryPromotionRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryPromotedClearParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<ThreadId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryPromotedClearResult {
    pub cleared: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetrievalTurnParams {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetrievalDebugSummary {
    pub text: String,
    #[serde(default)]
    pub notes: Vec<String>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetrievalRecommendationsResult {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub plans: Vec<RetrievalRoutePlan>,
    pub summary: RetrievalDebugSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetrievalMetricsResult {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub outcomes: Vec<RetrievalMeasuredOutcome>,
    pub accepted_count: u64,
    pub ignored_count: u64,
    pub failed_count: u64,
    pub outcome_counts: BTreeMap<String, u64>,
    pub mode_counts: BTreeMap<RetrievalMode, u64>,
    pub summary: RetrievalDebugSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetrievalPromotedCapabilityState {
    pub item_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_id: Option<String>,
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub thread_id: ThreadId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetrievalPromotedResult {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub states: Vec<RetrievalPromotedCapabilityState>,
    pub summary: RetrievalDebugSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaReadParams {
    pub artifact_id: MediaArtifactId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaReadResult {
    pub artifact: MediaArtifact,
    pub bytes_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaThumbnailParams {
    pub artifact_id: MediaArtifactId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaThumbnailResult {
    pub preview: MediaPreview,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaDeleteParams {
    pub artifact_id: MediaArtifactId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaDeleteResult {
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaAttachToTurnParams {
    pub artifact_id: MediaArtifactId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaAttachToTurnResult {
    pub attachment: MediaAttachment,
    pub image: Option<InputImage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowScanParams {
    pub workspace: Option<String>,
    #[serde(default)]
    pub include_user: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowScanResult {
    pub scan: WorkflowImportScan,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowPreviewParams {
    pub workspace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowPreviewResult {
    pub items: Vec<WorkflowImportItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowEnableParams {
    pub workspace: Option<String>,
    pub item_id: String,
    #[serde(default)]
    pub approve_side_effects: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowEnableResult {
    pub item: WorkflowImportItem,
    pub decision: WorkflowImportDecision,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowIgnoreParams {
    pub workspace: Option<String>,
    pub item_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowIgnoreResult {
    pub item_id: String,
    pub decision: WorkflowImportDecision,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRefreshParams {
    pub workspace: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRefreshResult {
    pub scan: WorkflowImportScan,
    pub stale: Vec<WorkflowImportItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRemoveParams {
    pub workspace: Option<String>,
    pub item_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRemoveResult {
    pub item_id: String,
    pub state: WorkflowImportState,
    pub decision: WorkflowImportDecision,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EvalReportsListParams {
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EvalReliabilitySummary {
    #[serde(default)]
    pub error_class_counts: BTreeMap<String, u64>,
    pub retry_attempts: u64,
    pub retry_recoveries: u64,
    pub failure_limit_stops: u64,
    pub unknown_errors: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EvalReportSummary {
    pub id: String,
    pub suite_id: String,
    pub fixture_count: usize,
    pub passed: usize,
    pub failed: usize,
    #[serde(default)]
    pub reliability: EvalReliabilitySummary,
    #[serde(with = "time::serde::rfc3339")]
    pub generated_at: time::OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EvalReportsListResult {
    pub reports: Vec<EvalReportSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EvalReportReadParams {
    pub report_id: String,
    pub max_bytes: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EvalReportReadResult {
    pub summary: EvalReportSummary,
    pub markdown: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketplacesListResult {
    pub marketplaces: Vec<MarketplaceDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketplacesInstallDefaultParams {
    pub selection: DefaultMarketplaceSelection,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketplacesInstallDefaultResult {
    pub marketplaces: Vec<MarketplaceDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketplacesAddParams {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<MarketplaceKind>,
    pub display_name: String,
    pub source: MarketplaceSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketplacesAddResult {
    pub marketplace: MarketplaceDescriptor,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketplacesRemoveParams {
    pub marketplace_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketplacesRemoveResult {
    pub removed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketplacesRefreshParams {
    pub marketplace_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketplacesRefreshResult {
    pub marketplace: MarketplaceDescriptor,
    pub plugins: Vec<MarketplacePluginEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketplacesSearchParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketplacesSearchResult {
    pub plugins: Vec<DedupedMarketplacePlugin>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketplacePluginParams {
    pub marketplace_id: String,
    pub plugin_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketplacePluginResult {
    pub plugin: Option<MarketplacePluginEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginPreviewInstallParams {
    pub marketplace_id: String,
    pub plugin_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginPreviewInstallResult {
    pub preview: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInstallParams {
    pub marketplace_id: String,
    pub plugin_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInstallResult {
    pub plugin: InstalledPluginRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInstallAllVariantsParams {
    pub marketplace_id: String,
    pub plugin_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInstallAllVariantsResult {
    pub plugins: Vec<InstalledPluginRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginListInstalledResult {
    pub plugins: Vec<InstalledPluginRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginDisableParams {
    pub variant_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginDisableResult {
    pub plugin: Option<InstalledPluginRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginUninstallParams {
    pub variant_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginUninstallResult {
    pub removed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSelectParams {
    pub provider: String,
    pub model: Option<String>,
    pub reasoning: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<ThreadId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSelectResult {
    pub provider: String,
    pub model: String,
    pub reasoning: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_switch_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsGetResult {
    pub web_search: WebSearchSettings,
    pub search_index: SearchIndexSettings,
    pub shell: ShellSettings,
    pub default_mode: PolicyMode,
    pub file_backed_dynamic_context: bool,
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
pub struct SettingsSetSearchIndexParams {
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsSetSearchIndexResult {
    pub search_index: SearchIndexSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsSetShellParams {
    pub shell: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsSetShellResult {
    pub shell: ShellSettings,
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
pub struct SettingsSetFileBackedDynamicContextParams {
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsSetFileBackedDynamicContextResult {
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderAuthResult {
    pub signed_in: bool,
    pub account_id: Option<String>,
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
pub struct CommandsListResult {
    pub commands: Vec<CommandDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandsExpandParams {
    pub name: String,
    #[serde(default)]
    pub arguments: String,
    pub workspace: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandsExpandResult {
    pub command: CommandDescriptor,
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
    #[serde(default)]
    pub arguments: String,
    pub workspace: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandsRunResult {
    pub turn_id: TurnId,
    pub expanded: CommandsExpandResult,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsListResult {
    pub skills: Vec<SkillDescriptor>,
    #[serde(default)]
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsReadParams {
    pub selector: SkillSelector,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsReadResult {
    pub skill: Option<Skill>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsSetEnabledParams {
    pub selector: SkillSelector,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsSetExposureParams {
    pub selector: SkillSelector,
    pub exposure: SkillExposure,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsUpdateResult {
    pub skills: Vec<SkillDescriptor>,
    #[serde(default)]
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationsListParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_disabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationsListResult {
    pub automations: Vec<AutomationDefinition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationsCreateParams {
    pub name: String,
    pub project: AutomationProject,
    pub schedule: AutomationSchedule,
    pub prompt: String,
    #[serde(default = "default_true_bool")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_mode: Option<roder_api::policy_mode::PolicyMode>,
    pub catch_up: CatchUpPolicy,
    pub concurrency: AutomationConcurrencyPolicy,
}

fn default_true_bool() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationsCreateResult {
    pub automation: AutomationDefinition,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AutomationsUpdatePatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<AutomationProject>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<AutomationSchedule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_mode: Option<roder_api::policy_mode::PolicyMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catch_up: Option<CatchUpPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concurrency: Option<AutomationConcurrencyPolicy>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationsUpdateParams {
    pub automation_id: AutomationId,
    pub patch: AutomationsUpdatePatch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationsUpdateResult {
    pub automation: AutomationDefinition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationsDeleteParams {
    pub automation_id: AutomationId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationsDeleteResult {
    pub automation_id: AutomationId,
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationsRunNowParams {
    pub automation_id: AutomationId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_override: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationsRunNowResult {
    pub run: AutomationRunSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationsRunsParams {
    pub automation_id: AutomationId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<AutomationRunState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationsRunsResult {
    pub runs: Vec<AutomationRunSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationsCancelRunParams {
    pub run_id: AutomationRunId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationsCancelRunResult {
    pub run_id: AutomationRunId,
    pub cancelled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationsStatusResult {
    pub scheduler_enabled: bool,
    pub read_api_enabled: bool,
    pub server_id: String,
    pub server_role: String,
    pub store_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_tick_at: Option<OffsetDateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_tick_at: Option<OffsetDateTime>,
    #[serde(default)]
    pub active_runs: usize,
    #[serde(default)]
    pub due_count: usize,
    #[serde(default)]
    pub leased_count: usize,
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
pub struct TasksSubmitParams {
    pub executor_id: String,
    #[serde(default)]
    pub input: serde_json::Value,
    pub thread_id: Option<ThreadId>,
    pub turn_id: Option<TurnId>,
    pub workspace: Option<String>,
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
pub struct TaskLogDescriptor {
    pub stream: TaskOutputStream,
    pub chunk: String,
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TasksGetResult {
    pub task: TaskHandle,
    pub logs: Vec<TaskLogDescriptor>,
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
    pub event_kinds: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProcessesListParams {
    #[serde(default)]
    pub include_completed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessesListResult {
    pub processes: Vec<ProcessDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessesGetParams {
    pub process_id: ProcessId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_bytes: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessesGetResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process: Option<ProcessDescriptor>,
    #[serde(default)]
    pub output: Vec<ProcessOutput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessesStopParams {
    pub process_id: ProcessId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessesStopResult {
    pub result: ProcessStopResult,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProcessesStopAllParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessesStopAllResult {
    pub results: Vec<ProcessStopResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessesSubscribeResult {
    pub subscribed: bool,
    pub event_kinds: Vec<String>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_turn_start_params_accept_desktop_input_shape() {
        let params: TurnStartParams = serde_json::from_value(serde_json::json!({
            "threadId": "thread-1",
            "input": [
                { "type": "text", "text": "hello" },
                { "type": "image", "imageUrl": "data:image/png;base64,YWJj" }
            ]
        }))
        .unwrap();

        assert_eq!(params.thread_id, "thread-1");
        assert_eq!(params.input[0].kind, "text");
        assert_eq!(params.input[0].text.as_deref(), Some("hello"));
        assert_eq!(params.input[1].kind, "image");
        assert_eq!(
            params.input[1].image_url.as_deref(),
            Some("data:image/png;base64,YWJj")
        );
        assert!(!params.task_ledger_required);
    }

    #[test]
    fn desktop_turn_start_params_accept_task_ledger_required() {
        let params: TurnStartParams = serde_json::from_value(serde_json::json!({
            "threadId": "thread-1",
            "taskLedgerRequired": true
        }))
        .unwrap();

        assert!(params.task_ledger_required);
    }

    #[test]
    fn desktop_notifications_serialize_with_json_rpc_method_and_camel_case_params() {
        let notification = JsonRpcNotification {
            jsonrpc: "2.0".to_string(),
            method: "item/agentMessage/delta".to_string(),
            params: serde_json::to_value(AgentMessageDeltaNotification {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                item_id: "turn-1-assistant".to_string(),
                delta: "hello".to_string(),
                phase: Some("final_answer".to_string()),
            })
            .unwrap(),
        };

        let value = serde_json::to_value(notification).unwrap();
        assert_eq!(value["jsonrpc"], "2.0");
        assert_eq!(value["method"], "item/agentMessage/delta");
        assert_eq!(value["params"]["threadId"], "thread-1");
        assert_eq!(value["params"]["turnId"], "turn-1");
        assert_eq!(value["params"]["itemId"], "turn-1-assistant");
        assert_eq!(value["params"]["delta"], "hello");
        assert_eq!(value["params"]["phase"], "final_answer");
    }

    #[test]
    fn verification_notifications_use_camel_case_fields() {
        let value = serde_json::to_value(VerificationRequiredNotification {
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            reason: "code_changes_without_verification".to_string(),
            changed_files: vec!["src/lib.rs".to_string()],
            tool_evidence: vec!["write_file: wrote src/lib.rs".to_string()],
            tests_run: vec!["cargo test".to_string()],
            open_gaps: Vec::new(),
        })
        .unwrap();

        assert_eq!(value["threadId"], "thread-1");
        assert_eq!(value["turnId"], "turn-1");
        assert_eq!(value["changedFiles"][0], "src/lib.rs");
        assert_eq!(value["toolEvidence"][0], "write_file: wrote src/lib.rs");
        assert_eq!(value["testsRun"][0], "cargo test");
        assert!(value.get("changed_files").is_none());
    }

    #[test]
    fn search_index_status_protocol_uses_camel_case_fields() {
        let value = serde_json::to_value(SearchIndexStatusNotification {
            status: SearchIndexStatus {
                state: SearchIndexStatusState::Ready,
                enabled: true,
                workspace: "/tmp/workspace".to_string(),
                store_dir: "/tmp/home/.roder/indexes/abc".to_string(),
                index_version: Some("fastregex-v1".to_string()),
                document_count: Some(7),
                index_bytes: Some(128),
                build_time_ms: Some(4),
                stale: false,
                message: None,
            },
        })
        .unwrap();

        assert_eq!(value["status"]["state"], "ready");
        assert_eq!(value["status"]["storeDir"], "/tmp/home/.roder/indexes/abc");
        assert_eq!(value["status"]["indexVersion"], "fastregex-v1");
        assert_eq!(value["status"]["documentCount"], 7);
        assert_eq!(value["status"]["buildTimeMs"], 4);
        assert!(value["status"].get("store_dir").is_none());
        assert!(value["status"].get("document_count").is_none());
    }

    #[test]
    fn code_index_status_protocol_uses_camel_case_fields() {
        let value = serde_json::to_value(CodeIndexStatusNotification {
            status: CodeIndexStatusView {
                status: CodeIndexStatus::Ready,
                workspace: "/tmp/workspace".to_string(),
                store_path: "/tmp/home/.roder/code-index/abc/code-index.sqlite3".to_string(),
                generation_id: Some("gen-1".to_string()),
                root_hash: Some("root-hash".to_string()),
                stale: false,
                stats: roder_api::code_index::CodeIndexStats {
                    file_count: 2,
                    chunk_count: 3,
                    embedded_chunk_count: 3,
                    cached_embedding_count: 1,
                    index_bytes: 256,
                },
                message: None,
            },
        })
        .unwrap();

        assert_eq!(value["status"]["status"], "ready");
        assert_eq!(
            value["status"]["storePath"],
            "/tmp/home/.roder/code-index/abc/code-index.sqlite3"
        );
        assert_eq!(value["status"]["generationId"], "gen-1");
        assert_eq!(value["status"]["stats"]["chunkCount"], 3);
        assert_eq!(value["status"]["stats"]["cachedEmbeddingCount"], 1);
        assert!(value["status"].get("store_path").is_none());
        assert!(value["status"]["stats"].get("chunk_count").is_none());
    }

    #[test]
    fn team_start_params_round_trip_camel_case_display_mode() {
        let params: TeamStartParams = serde_json::from_value(serde_json::json!({
            "leadThreadId": "lead-thread",
            "displayMode": "tmux",
            "members": [{
                "name": "reviewer",
                "modelProvider": "mock",
                "model": "mock"
            }]
        }))
        .unwrap();

        assert_eq!(params.lead_thread_id.as_deref(), Some("lead-thread"));
        assert_eq!(params.display_mode, Some(AgentTeamDisplayMode::Tmux));
        assert_eq!(params.members[0].model_provider.as_deref(), Some("mock"));

        let value = serde_json::to_value(params).unwrap();
        assert_eq!(value["leadThreadId"], "lead-thread");
        assert_eq!(value["displayMode"], "tmux");
        assert_eq!(value["members"][0]["modelProvider"], "mock");
    }

    #[test]
    fn subagent_trace_protocol_structs_use_camel_case_fields() {
        let list_params: SubagentTracesListParams = serde_json::from_value(serde_json::json!({
            "threadId": "thread-1",
            "turnId": "turn-1"
        }))
        .unwrap();
        assert_eq!(list_params.thread_id, "thread-1");
        assert_eq!(list_params.turn_id, "turn-1");

        let read_params: SubagentTraceReadParams = serde_json::from_value(serde_json::json!({
            "threadId": "thread-1",
            "traceId": "trace-1",
            "offset": 10,
            "limit": 20
        }))
        .unwrap();
        assert_eq!(read_params.thread_id, "thread-1");
        assert_eq!(read_params.trace_id, "trace-1");
        assert_eq!(read_params.offset, 10);
        assert_eq!(read_params.limit, Some(20));

        let result = SubagentTraceReadResult {
            trace_id: "trace-1".to_string(),
            events: Vec::new(),
            next_offset: Some(30),
        };
        let value = serde_json::to_value(result).unwrap();
        assert_eq!(value["traceId"], "trace-1");
        assert_eq!(value["nextOffset"], 30);
    }

    #[test]
    fn artifacts_protocol_structs_use_camel_case_fields() {
        let params: ArtifactReadParams = serde_json::from_value(serde_json::json!({
            "threadId": "thread-1",
            "artifactId": "artifact-1",
            "startLine": 2,
            "limit": 10
        }))
        .unwrap();

        assert_eq!(params.thread_id, "thread-1");
        assert_eq!(params.artifact_id, "artifact-1");
        assert_eq!(params.start_line, Some(2));

        let command = serde_json::to_value(CommandExecResponse {
            exit_code: 0,
            stdout: "short".to_string(),
            stderr: String::new(),
            stdout_artifact: None,
            stderr_artifact: None,
        })
        .unwrap();

        assert_eq!(command["exitCode"], 0);
        assert!(command.get("stdoutArtifact").is_none());
    }

    #[test]
    fn discovery_protocol_structs_use_camel_case_fields() {
        let params: DiscoveryReadParams = serde_json::from_value(serde_json::json!({
            "itemId": "tool:builtin/grep",
            "startLine": 2,
            "limit": 10,
            "threadId": "thread-1",
            "turnId": "turn-1"
        }))
        .unwrap();

        assert_eq!(params.item_id, "tool:builtin/grep");
        assert_eq!(params.start_line, Some(2));
        assert_eq!(params.thread_id.as_deref(), Some("thread-1"));

        let clear = serde_json::to_value(DiscoveryPromotedClearResult { cleared: 2 }).unwrap();
        assert_eq!(clear["cleared"], 2);
    }

    #[test]
    fn retrieval_protocol_structs_use_camel_case_fields() {
        let params: RetrievalTurnParams = serde_json::from_value(serde_json::json!({
            "threadId": "thread-1",
            "turnId": "turn-1",
            "limit": 5
        }))
        .unwrap();
        assert_eq!(params.thread_id, "thread-1");
        assert_eq!(params.turn_id, "turn-1");
        assert_eq!(params.limit, Some(5));

        let result = RetrievalRecommendationsResult {
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            plans: Vec::new(),
            summary: RetrievalDebugSummary {
                text: "no route recommendations recorded".to_string(),
                notes: vec!["router did not emit retrieval/routePlanned".to_string()],
                truncated: false,
            },
        };
        let value = serde_json::to_value(result).unwrap();
        assert_eq!(value["threadId"], "thread-1");
        assert_eq!(value["turnId"], "turn-1");
        assert_eq!(value["summary"]["truncated"], false);

        let state = RetrievalPromotedCapabilityState {
            item_id: "tool:builtin/grep".to_string(),
            route_id: Some("route-1".to_string()),
            state: "skipped".to_string(),
            cache_status: None,
            reason: Some("already warm".to_string()),
            thread_id: "thread-1".to_string(),
            turn_id: Some("turn-1".to_string()),
            timestamp: OffsetDateTime::UNIX_EPOCH,
        };
        let value = serde_json::to_value(state).unwrap();
        assert_eq!(value["itemId"], "tool:builtin/grep");
        assert_eq!(value["routeId"], "route-1");
        assert!(value.get("cacheStatus").is_none());
    }

    #[test]
    fn plan_review_and_hunk_protocol_structs_use_camel_case_fields() {
        let comment_params: PlanReviewCommentParams = serde_json::from_value(serde_json::json!({
            "threadId": "thread-1",
            "reviewId": "review-1",
            "anchor": { "step": { "stepId": "step-1" } },
            "body": "Use the smaller patch."
        }))
        .unwrap();
        assert_eq!(comment_params.thread_id, "thread-1");
        assert_eq!(comment_params.review_id, "review-1");

        let hunk_params: HunkReadParams = serde_json::from_value(serde_json::json!({
            "threadId": "thread-1",
            "hunkId": "hunk-1",
            "offset": 4,
            "limit": 20
        }))
        .unwrap();
        assert_eq!(hunk_params.hunk_id, "hunk-1");
        assert_eq!(hunk_params.limit, Some(20));

        let result = HunkRollbackResult {
            rolled_back: false,
            error: Some("checkpoint data is unavailable".to_string()),
        };
        let value = serde_json::to_value(result).unwrap();
        assert_eq!(value["rolledBack"], false);
        assert_eq!(value["error"], "checkpoint data is unavailable");
    }

    #[test]
    fn workflow_import_protocol_structs_use_camel_case_fields() {
        let scan_params: WorkflowScanParams = serde_json::from_value(serde_json::json!({
            "workspace": "/tmp/repo",
            "includeUser": true
        }))
        .unwrap();
        assert_eq!(scan_params.workspace.as_deref(), Some("/tmp/repo"));
        assert!(scan_params.include_user);

        let enable_params: WorkflowEnableParams = serde_json::from_value(serde_json::json!({
            "workspace": "/tmp/repo",
            "itemId": "workflow-1",
            "approveSideEffects": true
        }))
        .unwrap();
        assert_eq!(enable_params.item_id, "workflow-1");
        assert!(enable_params.approve_side_effects);

        let remove = WorkflowRemoveResult {
            item_id: "workflow-1".to_string(),
            state: WorkflowImportState::Removed,
            decision: WorkflowImportDecision {
                item_id: "workflow-1".to_string(),
                decision: roder_api::workflow::WorkflowImportDecisionKind::Remove,
                source_hash: "hash".to_string(),
                approved_side_effects: false,
                decided_at: OffsetDateTime::UNIX_EPOCH,
            },
        };
        let value = serde_json::to_value(remove).unwrap();
        assert_eq!(value["itemId"], "workflow-1");
        assert_eq!(value["state"], "removed");
        assert_eq!(value["decision"]["sourceHash"], "hash");
    }

    #[test]
    fn marketplace_protocol_structs_use_camel_case_fields() {
        let params: MarketplacesInstallDefaultParams = serde_json::from_value(serde_json::json!({
            "selection": "all"
        }))
        .unwrap();
        assert_eq!(params.selection, DefaultMarketplaceSelection::All);

        let add: MarketplacesAddParams = serde_json::from_value(serde_json::json!({
            "id": "local-cursor",
            "kind": "cursor",
            "displayName": "Local Cursor",
            "source": {
                "kind": "localPath",
                "path": "/tmp/cursor"
            }
        }))
        .unwrap();
        assert_eq!(add.id, "local-cursor");
        assert_eq!(add.kind, Some(MarketplaceKind::Cursor));
        assert_eq!(
            add.source,
            MarketplaceSource::LocalPath {
                path: "/tmp/cursor".to_string()
            }
        );

        let remove = MarketplacesRemoveParams {
            marketplace_id: "local-cursor".to_string(),
        };
        let value = serde_json::to_value(remove).unwrap();
        assert_eq!(value["marketplaceId"], "local-cursor");

        let disable = PluginDisableParams {
            variant_key: "codex-plugins:superpowers".to_string(),
        };
        let value = serde_json::to_value(disable).unwrap();
        assert_eq!(value["variantKey"], "codex-plugins:superpowers");

        let all = PluginInstallAllVariantsParams {
            marketplace_id: "codex-plugins".to_string(),
            plugin_id: "superpowers".to_string(),
        };
        let value = serde_json::to_value(all).unwrap();
        assert_eq!(value["marketplaceId"], "codex-plugins");

        let uninstall = PluginUninstallParams {
            variant_key: "codex-plugins:superpowers".to_string(),
        };
        let value = serde_json::to_value(uninstall).unwrap();
        assert_eq!(value["variantKey"], "codex-plugins:superpowers");
    }

    #[test]
    fn skills_protocol_structs_use_camel_case_fields() {
        let params: SkillsSetExposureParams = serde_json::from_value(serde_json::json!({
            "selector": { "name": { "name": "commit" } },
            "exposure": "direct_only"
        }))
        .unwrap();
        assert_eq!(
            params.selector,
            SkillSelector::Name {
                name: "commit".to_string()
            }
        );
        assert_eq!(params.exposure, SkillExposure::DirectOnly);

        let enabled = SkillsSetEnabledParams {
            selector: SkillSelector::Path {
                path: "roder-builtin://commit/SKILL.md".to_string(),
            },
            enabled: false,
        };
        let value = serde_json::to_value(enabled).unwrap();
        assert_eq!(
            value["selector"]["path"]["path"],
            "roder-builtin://commit/SKILL.md"
        );
        assert_eq!(value["enabled"], false);
    }

    #[test]
    fn automations_protocol_structs_use_camel_case_fields() {
        let create: AutomationsCreateParams = serde_json::from_value(serde_json::json!({
            "name": "Nightly cleanup",
            "project": { "cwd": "/repo", "displayName": "repo" },
            "schedule": { "interval": { "seconds": 300 } },
            "prompt": "summarize status",
            "modelProvider": "codex",
            "model": "gpt-5.5",
            "policyMode": "plan",
            "catchUp": { "runLatestOnly": null },
            "concurrency": "forbid"
        }))
        .unwrap();

        assert_eq!(create.name, "Nightly cleanup");
        assert_eq!(create.project.display_name.as_deref(), Some("repo"));
        assert_eq!(create.model_provider.as_deref(), Some("codex"));
        assert_eq!(
            create.policy_mode,
            Some(roder_api::policy_mode::PolicyMode::Plan)
        );
        assert_eq!(create.catch_up, CatchUpPolicy::RunLatestOnly);

        let update = AutomationsUpdateParams {
            automation_id: "automation-1".to_string(),
            patch: AutomationsUpdatePatch {
                model_provider: Some("codex".to_string()),
                catch_up: Some(CatchUpPolicy::SkipExpired { grace_seconds: 60 }),
                ..AutomationsUpdatePatch::default()
            },
        };
        let value = serde_json::to_value(update).unwrap();
        assert_eq!(value["automationId"], "automation-1");
        assert_eq!(value["patch"]["modelProvider"], "codex");
        assert_eq!(value["patch"]["catchUp"]["skipExpired"]["graceSeconds"], 60);
        assert!(value.get("automation_id").is_none());
    }

    #[test]
    fn processes_protocol_structs_cover_list_detail_stop_and_subscribe() {
        let descriptor = ProcessDescriptor {
            process_id: "process-1".to_string(),
            origin: roder_api::processes::ProcessOrigin::CommandExec,
            state: roder_api::processes::ProcessState::Running,
            command: vec!["sleep".to_string(), "10".to_string()],
            command_summary: "sleep 10".to_string(),
            cwd: Some("/repo".to_string()),
            pid: Some(1234),
            task_id: Some("task-1".to_string()),
            thread_id: Some("thread-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            runner_destination_id: None,
            runner_session_id: None,
            stoppable: true,
            started_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
            stdout_tail: Some("ready\n".to_string()),
            stderr_tail: None,
        };

        let list_params: ProcessesListParams = serde_json::from_value(serde_json::json!({
            "includeCompleted": true
        }))
        .unwrap();
        assert!(list_params.include_completed);

        let list = serde_json::to_value(ProcessesListResult {
            processes: vec![descriptor.clone()],
        })
        .unwrap();
        assert_eq!(list["processes"][0]["processId"], "process-1");
        assert_eq!(list["processes"][0]["pid"], 1234);
        assert!(list["processes"][0].get("process_id").is_none());

        let get: ProcessesGetParams = serde_json::from_value(serde_json::json!({
            "processId": "process-1",
            "outputBytes": 1024
        }))
        .unwrap();
        assert_eq!(get.process_id, "process-1");
        assert_eq!(get.output_bytes, Some(1024));

        let stop: ProcessesStopParams = serde_json::from_value(serde_json::json!({
            "processId": "process-1",
            "reason": "user requested stop"
        }))
        .unwrap();
        assert_eq!(stop.process_id, "process-1");
        assert_eq!(stop.reason.as_deref(), Some("user requested stop"));

        let stop_all: ProcessesStopAllResult = serde_json::from_value(serde_json::json!({
            "results": [{
                "processId": "process-1",
                "stopped": true,
                "process": list["processes"][0].clone()
            }]
        }))
        .unwrap();
        assert_eq!(stop_all.results[0].process_id, "process-1");
        assert!(stop_all.results[0].stopped);

        let subscribe = ProcessesSubscribeResult {
            subscribed: true,
            event_kinds: vec!["process.started".to_string(), "process.output".to_string()],
        };
        let value = serde_json::to_value(subscribe).unwrap();
        assert_eq!(value["subscribed"], true);
        assert_eq!(value["eventKinds"][0], "process.started");
        assert!(value.get("event_kinds").is_none());
    }

    #[test]
    fn eval_report_protocol_structs_use_camel_case_fields() {
        let list: EvalReportsListParams = serde_json::from_value(serde_json::json!({
            "limit": 5
        }))
        .unwrap();
        assert_eq!(list.limit, Some(5));

        let read: EvalReportReadParams = serde_json::from_value(serde_json::json!({
            "reportId": "eval-run",
            "maxBytes": 1024
        }))
        .unwrap();
        assert_eq!(read.report_id, "eval-run");
        assert_eq!(read.max_bytes, Some(1024));

        let result = EvalReportReadResult {
            summary: EvalReportSummary {
                id: "eval-run".to_string(),
                suite_id: "tool-calls".to_string(),
                fixture_count: 2,
                passed: 1,
                failed: 1,
                reliability: EvalReliabilitySummary {
                    retry_attempts: 2,
                    retry_recoveries: 1,
                    ..EvalReliabilitySummary::default()
                },
                generated_at: OffsetDateTime::UNIX_EPOCH,
            },
            markdown: "# Report".to_string(),
            truncated: false,
        };
        let value = serde_json::to_value(result).unwrap();
        assert_eq!(value["summary"]["suiteId"], "tool-calls");
        assert_eq!(value["summary"]["fixtureCount"], 2);
        assert_eq!(value["summary"]["reliability"]["retryAttempts"], 2);
        assert_eq!(value["summary"]["reliability"]["retryRecoveries"], 1);
        assert_eq!(value["markdown"], "# Report");
    }

    #[test]
    fn deadline_notifications_use_camel_case_fields() {
        let value = serde_json::to_value(TurnDeadlineExceededNotification {
            thread_id: "thread-a".to_string(),
            turn_id: "turn-a".to_string(),
            deadline: OffsetDateTime::UNIX_EPOCH,
            partial_result: "partial evidence".to_string(),
        })
        .unwrap();
        assert_eq!(value["threadId"], "thread-a");
        assert_eq!(value["turnId"], "turn-a");
        assert_eq!(value["partialResult"], "partial evidence");

        let value = serde_json::to_value(TurnPartialResultNotification {
            thread_id: "thread-a".to_string(),
            turn_id: "turn-a".to_string(),
            summary: "partial evidence".to_string(),
        })
        .unwrap();
        assert_eq!(value["summary"], "partial evidence");
    }

    #[test]
    fn thread_goal_set_params_preserve_null_budget_clear() {
        let params: ThreadGoalSetParams = serde_json::from_value(serde_json::json!({
            "threadId": "thread-a",
            "tokenBudget": null
        }))
        .unwrap();
        assert_eq!(params.thread_id, "thread-a");
        assert_eq!(params.token_budget, Some(None));

        let params: ThreadGoalSetParams = serde_json::from_value(serde_json::json!({
            "threadId": "thread-a"
        }))
        .unwrap();
        assert_eq!(params.token_budget, None);
    }

    #[test]
    fn media_protocol_structs_use_camel_case_fields() {
        let read: MediaReadParams = serde_json::from_value(serde_json::json!({
            "artifactId": "media-1",
            "maxBytes": 1024
        }))
        .unwrap();
        assert_eq!(read.artifact_id, "media-1");
        assert_eq!(read.max_bytes, Some(1024));

        let attach = MediaAttachToTurnResult {
            attachment: MediaAttachment {
                artifact_id: "media-1".to_string(),
                mime_type: "image/png".to_string(),
                data_url: "data:image/png;base64,YWJj".to_string(),
            },
            image: Some(InputImage {
                image_url: "data:image/png;base64,YWJj".to_string(),
            }),
        };
        let value = serde_json::to_value(attach).unwrap();
        assert_eq!(value["attachment"]["artifactId"], "media-1");
        assert_eq!(value["attachment"]["mimeType"], "image/png");
        assert_eq!(value["image"]["image_url"], "data:image/png;base64,YWJj");
    }

    #[test]
    fn transcript_json_rpc_boundary_values_round_trip() {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(7)),
            method: "processes/list".to_string(),
            params: Some(serde_json::json!({"includeCompleted": true})),
        };
        let response = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(7)),
            result: Some(serde_json::json!({"processes": []})),
            error: None,
        };
        let notification = JsonRpcNotification {
            jsonrpc: "2.0".to_string(),
            method: "processes/changed".to_string(),
            params: serde_json::json!({"processId": "proc-a"}),
        };

        assert_eq!(
            serde_json::to_value(&request).unwrap()["method"],
            "processes/list"
        );
        assert_eq!(
            serde_json::to_value(&response).unwrap()["result"]["processes"],
            serde_json::json!([])
        );
        assert_eq!(
            serde_json::to_value(&notification).unwrap()["method"],
            "processes/changed"
        );
    }
}
