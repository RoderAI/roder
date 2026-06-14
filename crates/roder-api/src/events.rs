use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::artifacts::{ContextArtifact, ContextArtifactId};
use crate::automations::{
    AutomationCompleted, AutomationCreated, AutomationDeleted, AutomationDue, AutomationFailed,
    AutomationLeaseExpired, AutomationLeased, AutomationQueued, AutomationSkipped,
    AutomationStarted, AutomationUpdated,
};
use crate::code_index::{
    CodeIndexChunked, CodeIndexEmbedded, CodeIndexFailed, CodeIndexProofFilteredResultDropped,
    CodeIndexReady, CodeIndexStale, CodeIndexingStarted,
};
use crate::discovery::{
    DiscoveryAuthRequired, DiscoveryCatalogBuilt, DiscoveryItemPromoted, DiscoveryItemRead,
    DiscoveryItemUpdated, DiscoveryPromotionExpired, DiscoveryPromotionReused,
    DiscoveryWarmCacheHit,
};
use crate::dynamic_workflows::{
    WorkflowAgentCompleted, WorkflowAgentFailed, WorkflowAgentQueued, WorkflowAgentStarted,
    WorkflowApprovalRequested, WorkflowCheckpointRecorded, WorkflowOutputRecorded,
    WorkflowPhaseCompleted, WorkflowPhaseStarted, WorkflowRunApproved, WorkflowRunCompleted,
    WorkflowRunDenied, WorkflowRunDrafted, WorkflowRunFailed, WorkflowRunPaused, WorkflowRunQueued,
    WorkflowRunResumed, WorkflowRunStarted, WorkflowRunStopped,
};
use crate::extension::{ExtensionId, InferenceEngineId};
use crate::goals::{ThreadGoalCleared, ThreadGoalUpdated};
use crate::inference::{
    InferenceEvent, ModelSelection, ReasoningConfig, RuntimeProfile, SpeedPolicyDecision,
    TokenUsage,
};
use crate::inference_routing::InferenceRoutingDecision;
use crate::knowledge::{KnowledgeDocId, KnowledgeDocSummary, KnowledgeLinkType};
use crate::media::{MediaArtifact, MediaArtifactId, MediaPreview};
use crate::memory::{MemoryCitation, MemoryId, MemoryProviderSelection, MemoryRecord, MemoryScope};
use crate::plan_review::{
    HunkId, HunkRecord, PlanComment, PlanReview, PlanReviewId, PlanReviewStatus, PlanRewrite,
};
use crate::processes::{
    ProcessExited, ProcessFailed, ProcessOutput, ProcessStarted, ProcessStopped, ProcessStopping,
};
use crate::reliability::{
    ReliabilityFailureRecorded, ReliabilityLimitRecorded, ReliabilityMetricRecorded,
    ReliabilityRetryRecorded,
};
use crate::retrieval::{
    RetrievalDiscoveryItemPromoted, RetrievalPromotionSkipped, RetrievalResultUsed,
    RetrievalRouteAccepted, RetrievalRouteFailed, RetrievalRouteIgnored, RetrievalRoutePlanned,
};
use crate::skills::{
    SkillActivationResolved, SkillAutoActivated, SkillConfigApplied, SkillIndexRendered,
    SkillInvoked, SkillSkipped, SkillsCatalogLoaded,
};
use crate::subagents::SubagentExitReason;
use crate::task_ledger::TaskLedgerItem;
use crate::teams::{
    AgentTeamDisplayMode, TeamId, TeamMemberId, TeamMemberRole, TeamMemberStatus,
    TeamTaskDescriptor,
};
use crate::trace::{
    ParentTurnRef, SubagentTraceDelta, SubagentTraceId, SubagentTraceStatus, SubagentTraceSummary,
};
use crate::transcript::TranscriptItem;
use crate::workflow::{WorkflowImportDecision, WorkflowImportError, WorkflowImportItem};
use crate::workspace_changes::WorkspaceChangeObservation;

pub use crate::policy_mode::{
    PolicyBypassActive, PolicyDecisionRecorded, PolicyExitPlanRequested, PolicyExitPlanResolved,
    PolicyModeChanged,
};
pub use crate::tasks::{TaskCancelled, TaskCompleted, TaskFailed, TaskOutput, TaskStarted};

pub type ThreadId = String;
pub type TurnId = String;
pub type EventId = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum EventSource {
    Runtime,
    Core,
    Provider,
    Tool,
    AppServer,
    Tui,
    Extension,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeStarted {
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionRegistered {
    pub extension_id: ExtensionId,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadCreated {
    pub thread_id: ThreadId,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

/// A conversation fork into a worktree-backed child thread was requested.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadForkRequested {
    pub parent_thread_id: ThreadId,
    pub name: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

/// A child thread was created with its workspace fork.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadForked {
    pub parent_thread_id: ThreadId,
    pub child_thread_id: ThreadId,
    pub fork: crate::forks::WorkspaceFork,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

/// A requested conversation fork failed (worktree or thread creation).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadForkFailed {
    pub parent_thread_id: ThreadId,
    pub name: String,
    pub message: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

/// A fork's worktree was explicitly removed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadForkRemoved {
    pub thread_id: ThreadId,
    pub fork_id: String,
    pub worktree_path: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

/// A typed event emitted by an extension (e.g. a process-hosted child)
/// through the extension-owned event channel. Payloads are redacted and
/// schema-versioned by the emitter; the host enforces a size cap.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionEventEmitted {
    pub extension_id: String,
    pub event_kind: String,
    pub schema_version: u32,
    pub payload: serde_json::Value,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

/// An event sink failed or timed out while handling an envelope. The
/// message is redacted; sink-failure events are never re-dispatched to
/// sinks, so a broken sink cannot create an event loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventSinkFailed {
    pub sink_id: String,
    pub event_kind: String,
    pub message: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadLoaded {
    pub thread_id: ThreadId,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnStarted {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    #[serde(default)]
    pub runtime_profile: RuntimeProfile,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextAssemblyStarted {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextBlockAdded {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub block_type: String,
    #[serde(default)]
    pub byte_count: u64,
    #[serde(default)]
    pub estimated_tokens: u32,
    #[serde(default)]
    pub priority: i32,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextAssemblyCompleted {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    #[serde(default)]
    pub block_count: u64,
    #[serde(default)]
    pub total_byte_count: u64,
    #[serde(default)]
    pub estimated_tokens: u32,
    #[serde(default)]
    pub prompt_estimated_tokens: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_budget: Option<u32>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextEntrypointCandidatesInjected {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub candidate_count: u64,
    pub block_byte_count: u64,
    pub estimated_tokens: u32,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextCompactionStarted {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub original_item_count: u64,
    pub original_estimated_tokens: u32,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextCompactionRecorded {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub original_item_count: u64,
    pub original_estimated_tokens: u32,
    pub compacted_item_count: u64,
    pub compacted_estimated_tokens: u32,
    pub file_backed: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceStarted {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub engine_id: InferenceEngineId,
    #[serde(default = "default_model_selection")]
    pub model: ModelSelection,
    #[serde(default)]
    pub reasoning: ReasoningConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_policy: Option<SpeedPolicyDecision>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_remaining_seconds: Option<u64>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

fn default_model_selection() -> ModelSelection {
    ModelSelection {
        provider: String::new(),
        model: String::new(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InferenceRoutingDecisionEvent {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    #[serde(default)]
    pub round_index: u32,
    pub default_selection: ModelSelection,
    pub selected_selection: ModelSelection,
    pub decision: InferenceRoutingDecision,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceEventReceived {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub event: InferenceEvent,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRequested {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub tool_id: String,
    pub tool_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_payload: Option<serde_json::Value>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallValidationFailureClass {
    InvalidJson,
    UnknownTool,
    MissingRequired,
    UnexpectedProperty,
    WrongType,
    EmptyRequiredString,
    SchemaRepairApplied,
    SchemaRepairRejected,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallValidationRepairStatus {
    NotNeeded,
    Applied,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallValidationRecorded {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub tool_id: String,
    pub tool_name: String,
    pub failure_class: ToolCallValidationFailureClass,
    pub repair_status: ToolCallValidationRepairStatus,
    pub message: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequested {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub approval_id: String,
    pub tool_id: String,
    pub tool_name: String,
    pub reason: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalResolved {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub approval_id: String,
    pub tool_id: String,
    pub tool_name: String,
    pub approved: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ExternalToolCallOutcome {
    Resolved,
    TimedOut,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalToolCallRequested {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub request_id: String,
    pub tool_id: String,
    pub tool_name: String,
    pub arguments: serde_json::Value,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalToolCallResolved {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub request_id: String,
    pub tool_id: String,
    pub tool_name: String,
    pub outcome: ExternalToolCallOutcome,
    pub is_error: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInputRequested {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub request_id: String,
    pub questions: serde_json::Value,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInputResolved {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub request_id: String,
    pub answers: serde_json::Value,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskLedgerUpdated {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub tasks: Vec<TaskLedgerItem>,
    pub completed_count: u64,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationRequired {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub reason: String,
    pub changed_files: Vec<String>,
    pub tool_evidence: Vec<String>,
    pub tests_run: Vec<String>,
    pub open_gaps: Vec<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationCompleted {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub passed: bool,
    pub changed_files: Vec<String>,
    pub tool_evidence: Vec<String>,
    pub tests_run: Vec<String>,
    pub open_gaps: Vec<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationSkipped {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub reason: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallStarted {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub tool_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_payload: Option<serde_json::Value>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallCompleted {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub tool_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_payload: Option<serde_json::Value>,
    #[serde(default)]
    pub is_error: bool,
    #[serde(default)]
    pub output: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutputTruncated {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub tool_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    pub original_line_count: u64,
    pub original_char_count: u64,
    pub inline_char_count: u64,
    pub artifact_backed: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentStarted {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub parent_thread_id: ThreadId,
    pub parent_turn_id: TurnId,
    pub agent_type: String,
    pub description: String,
    pub model: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentMessage {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub parent_thread_id: ThreadId,
    pub parent_turn_id: TurnId,
    pub agent_type: String,
    pub text: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentToolCall {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub parent_thread_id: ThreadId,
    pub parent_turn_id: TurnId,
    pub agent_type: String,
    pub tool_id: String,
    pub tool_name: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentCompleted {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub parent_thread_id: ThreadId,
    pub parent_turn_id: TurnId,
    pub agent_type: String,
    pub exit_reason: SubagentExitReason,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentFailed {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub parent_thread_id: ThreadId,
    pub parent_turn_id: TurnId,
    pub agent_type: String,
    pub error: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentTraceCreated {
    pub summary: SubagentTraceSummary,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentTraceDeltaEvent {
    pub delta: SubagentTraceDelta,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentTraceStatusChanged {
    pub trace_id: SubagentTraceId,
    pub parent: ParentTurnRef,
    pub status: SubagentTraceStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentTraceCompleted {
    pub summary: SubagentTraceSummary,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentTraceFailed {
    pub summary: SubagentTraceSummary,
    pub error: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanReviewCreated {
    pub review: PlanReview,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanReviewStatusChanged {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub review_id: PlanReviewId,
    pub status: PlanReviewStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanReviewCommentAdded {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub review_id: PlanReviewId,
    pub comment: PlanComment,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanReviewRewritten {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub review_id: PlanReviewId,
    pub rewrite: PlanRewrite,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanReviewApproved {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub review_id: PlanReviewId,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanReviewRejected {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub review_id: PlanReviewId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HunkRecorded {
    pub hunk: HunkRecord,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceChangeObserved {
    pub change: WorkspaceChangeObservation,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HunkRollbackRequested {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub hunk_id: HunkId,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HunkRollbackCompleted {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub hunk_id: HunkId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowImportsDetected {
    pub workspace: String,
    pub items: Vec<WorkflowImportItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<WorkflowImportError>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowImportPreviewed {
    pub item: WorkflowImportItem,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowImportEnabled {
    pub item: WorkflowImportItem,
    pub decision: WorkflowImportDecision,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowImportDisabled {
    pub item_id: String,
    pub decision: WorkflowImportDecision,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowImportStale {
    pub item: WorkflowImportItem,
    pub previous_hash: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowImportFailed {
    pub item_id: Option<String>,
    pub error: WorkflowImportError,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaArtifactCreated {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub artifact: MediaArtifact,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaArtifactUpdated {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub artifact: MediaArtifact,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaArtifactDeleted {
    pub artifact_id: MediaArtifactId,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaPreviewReady {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub preview: MediaPreview,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextArtifactCreated {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub artifact: ContextArtifact,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextArtifactAppended {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub artifact_id: ContextArtifactId,
    pub appended_bytes: u64,
    pub byte_count: u64,
    pub line_count: u64,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextArtifactCapped {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub artifact_id: ContextArtifactId,
    pub inline_byte_count: u64,
    pub original_byte_count: u64,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextArtifactDeleted {
    pub thread_id: ThreadId,
    pub artifact_id: ContextArtifactId,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextArtifactRetentionExpired {
    pub thread_id: ThreadId,
    pub artifact_id: ContextArtifactId,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChanged {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub path: String,
    pub change_type: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChangePreviewReady {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub tool_id: String,
    pub tool_name: String,
    pub path: String,
    pub change_type: String,
    pub before: Option<String>,
    pub after: String,
    pub supports_partial: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptItemAppended {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub item_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_index: Option<usize>,
    /// Full transcript item for runtime appends. `None` means the record carries
    /// append metadata without an embedded transcript item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<TranscriptItem>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnCompleted {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
    /**
     * Normalized stop reason of the turn's terminal inference step; see
     * `crate::inference::finish_reason_from_stop_reason` for the vocabulary.
     */
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnFailed {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub error: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnPartialResult {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub summary: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnDeadlineExceeded {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    #[serde(with = "time::serde::rfc3339")]
    pub deadline: OffsetDateTime,
    pub partial_result: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnInterrupted {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnSteered {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub message: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunnerLifecycle {
    pub destination_id: String,
    pub provider_id: String,
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamStarted {
    pub team_id: TeamId,
    pub lead_thread_id: ThreadId,
    pub display_mode: AgentTeamDisplayMode,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamMemberStarted {
    pub team_id: TeamId,
    pub member_id: TeamMemberId,
    pub member_thread_id: ThreadId,
    pub role: TeamMemberRole,
    pub name: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamMemberStatusChanged {
    pub team_id: TeamId,
    pub member_id: TeamMemberId,
    pub member_thread_id: ThreadId,
    pub status: TeamMemberStatus,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamMemberMessageDelta {
    pub team_id: TeamId,
    pub member_id: TeamMemberId,
    pub member_thread_id: ThreadId,
    pub turn_id: TurnId,
    pub delta: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamMemberCompleted {
    pub team_id: TeamId,
    pub member_id: TeamMemberId,
    pub member_thread_id: ThreadId,
    pub turn_id: Option<TurnId>,
    pub status: TeamMemberStatus,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamDisplayModeChanged {
    pub team_id: TeamId,
    pub display_mode: AgentTeamDisplayMode,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamTaskChanged {
    pub team_id: TeamId,
    pub task: TeamTaskDescriptor,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamCleanupCompleted {
    pub team_id: TeamId,
    pub forced: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySaved {
    pub memory: MemoryRecord,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryUpdated {
    pub memory: MemoryRecord,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryDeleted {
    pub memory_id: MemoryId,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryQueried {
    pub scope: Option<MemoryScope>,
    pub query: String,
    pub result_count: usize,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRecallReady {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub citations: Vec<MemoryCitation>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryReembedQueued {
    pub scope: Option<MemoryScope>,
    pub provider: MemoryProviderSelection,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryProviderChanged {
    pub provider: MemoryProviderSelection,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryObservationRecorded {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub memory_id: MemoryId,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeSaved {
    pub document: KnowledgeDocSummary,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeUpdated {
    pub document: KnowledgeDocSummary,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeArchived {
    pub doc_id: KnowledgeDocId,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeLinked {
    pub from: KnowledgeDocId,
    pub to: KnowledgeDocId,
    pub link_type: KnowledgeLinkType,
    pub removed: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteServerStarted {
    pub listen_addr: String,
    pub connect_urls: Vec<String>,
    pub token_preview: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteServerStopped {
    pub listen_addr: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteAuthFailed {
    pub remote_addr: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteClientConnected {
    pub remote_addr: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteClientDisconnected {
    pub remote_addr: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoadmapChanged {
    pub event_kind: String,
    pub path: String,
    pub task_id: Option<String>,
    pub thread_id: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RoderEvent {
    RuntimeStarted(RuntimeStarted),
    ExtensionRegistered(ExtensionRegistered),
    ExtensionEventEmitted(ExtensionEventEmitted),
    EventSinkFailed(EventSinkFailed),
    ThreadCreated(ThreadCreated),
    ThreadLoaded(ThreadLoaded),
    ThreadForkRequested(ThreadForkRequested),
    ThreadForked(ThreadForked),
    ThreadForkFailed(ThreadForkFailed),
    ThreadForkRemoved(ThreadForkRemoved),
    TurnStarted(TurnStarted),
    ContextAssemblyStarted(ContextAssemblyStarted),
    ContextBlockAdded(ContextBlockAdded),
    ContextAssemblyCompleted(ContextAssemblyCompleted),
    ContextEntrypointCandidatesInjected(ContextEntrypointCandidatesInjected),
    ContextCompactionStarted(ContextCompactionStarted),
    ContextCompactionRecorded(ContextCompactionRecorded),
    InferenceRoutingDecision(InferenceRoutingDecisionEvent),
    InferenceStarted(InferenceStarted),
    InferenceEventReceived(InferenceEventReceived),
    ToolCallRequested(ToolCallRequested),
    ToolCallValidationRecorded(ToolCallValidationRecorded),
    ReliabilityFailureRecorded(ReliabilityFailureRecorded),
    ReliabilityRetryRecorded(ReliabilityRetryRecorded),
    ReliabilityLimitRecorded(ReliabilityLimitRecorded),
    ReliabilityMetricRecorded(ReliabilityMetricRecorded),
    CodeIndexingStarted(CodeIndexingStarted),
    CodeIndexChunked(CodeIndexChunked),
    CodeIndexEmbedded(CodeIndexEmbedded),
    CodeIndexReady(CodeIndexReady),
    CodeIndexStale(CodeIndexStale),
    CodeIndexFailed(CodeIndexFailed),
    CodeIndexProofFilteredResultDropped(CodeIndexProofFilteredResultDropped),
    ApprovalRequested(ApprovalRequested),
    ApprovalResolved(ApprovalResolved),
    ExternalToolCallRequested(ExternalToolCallRequested),
    ExternalToolCallResolved(ExternalToolCallResolved),
    UserInputRequested(UserInputRequested),
    UserInputResolved(UserInputResolved),
    TaskLedgerUpdated(TaskLedgerUpdated),
    VerificationRequired(VerificationRequired),
    VerificationCompleted(VerificationCompleted),
    VerificationSkipped(VerificationSkipped),
    PolicyDecisionRecorded(PolicyDecisionRecorded),
    PolicyBypassActive(PolicyBypassActive),
    PolicyModeChanged(PolicyModeChanged),
    PolicyExitPlanRequested(PolicyExitPlanRequested),
    PolicyExitPlanResolved(PolicyExitPlanResolved),
    ToolCallStarted(ToolCallStarted),
    ToolCallCompleted(ToolCallCompleted),
    ToolOutputTruncated(ToolOutputTruncated),
    SubagentStarted(SubagentStarted),
    SubagentMessage(SubagentMessage),
    SubagentToolCall(SubagentToolCall),
    SubagentCompleted(SubagentCompleted),
    SubagentFailed(SubagentFailed),
    SubagentTraceCreated(SubagentTraceCreated),
    SubagentTraceDelta(SubagentTraceDeltaEvent),
    SubagentTraceStatusChanged(SubagentTraceStatusChanged),
    SubagentTraceCompleted(SubagentTraceCompleted),
    SubagentTraceFailed(SubagentTraceFailed),
    PlanReviewCreated(PlanReviewCreated),
    PlanReviewStatusChanged(PlanReviewStatusChanged),
    PlanReviewCommentAdded(PlanReviewCommentAdded),
    PlanReviewRewritten(PlanReviewRewritten),
    PlanReviewApproved(PlanReviewApproved),
    PlanReviewRejected(PlanReviewRejected),
    HunkRecorded(HunkRecorded),
    WorkspaceChangeObserved(WorkspaceChangeObserved),
    HunkRollbackRequested(HunkRollbackRequested),
    HunkRollbackCompleted(HunkRollbackCompleted),
    WorkflowImportsDetected(WorkflowImportsDetected),
    WorkflowImportPreviewed(WorkflowImportPreviewed),
    WorkflowImportEnabled(WorkflowImportEnabled),
    WorkflowImportDisabled(WorkflowImportDisabled),
    WorkflowImportStale(WorkflowImportStale),
    WorkflowImportFailed(WorkflowImportFailed),
    WorkflowRunDrafted(WorkflowRunDrafted),
    WorkflowApprovalRequested(WorkflowApprovalRequested),
    WorkflowRunApproved(WorkflowRunApproved),
    WorkflowRunDenied(WorkflowRunDenied),
    WorkflowRunQueued(WorkflowRunQueued),
    WorkflowRunStarted(WorkflowRunStarted),
    WorkflowPhaseStarted(WorkflowPhaseStarted),
    WorkflowPhaseCompleted(WorkflowPhaseCompleted),
    WorkflowAgentQueued(WorkflowAgentQueued),
    WorkflowAgentStarted(WorkflowAgentStarted),
    WorkflowAgentCompleted(WorkflowAgentCompleted),
    WorkflowAgentFailed(WorkflowAgentFailed),
    WorkflowOutputRecorded(WorkflowOutputRecorded),
    WorkflowCheckpointRecorded(WorkflowCheckpointRecorded),
    WorkflowRunPaused(WorkflowRunPaused),
    WorkflowRunResumed(WorkflowRunResumed),
    WorkflowRunStopped(WorkflowRunStopped),
    WorkflowRunCompleted(WorkflowRunCompleted),
    WorkflowRunFailed(WorkflowRunFailed),
    MediaArtifactCreated(MediaArtifactCreated),
    MediaArtifactUpdated(MediaArtifactUpdated),
    MediaArtifactDeleted(MediaArtifactDeleted),
    MediaPreviewReady(MediaPreviewReady),
    ContextArtifactCreated(ContextArtifactCreated),
    ContextArtifactAppended(ContextArtifactAppended),
    ContextArtifactCapped(ContextArtifactCapped),
    ContextArtifactDeleted(ContextArtifactDeleted),
    ContextArtifactRetentionExpired(ContextArtifactRetentionExpired),
    DiscoveryCatalogBuilt(DiscoveryCatalogBuilt),
    DiscoveryItemUpdated(DiscoveryItemUpdated),
    DiscoveryAuthRequired(DiscoveryAuthRequired),
    DiscoveryItemRead(DiscoveryItemRead),
    DiscoveryItemPromoted(DiscoveryItemPromoted),
    DiscoveryPromotionReused(DiscoveryPromotionReused),
    DiscoveryWarmCacheHit(DiscoveryWarmCacheHit),
    DiscoveryPromotionExpired(DiscoveryPromotionExpired),
    RetrievalRoutePlanned(RetrievalRoutePlanned),
    RetrievalRouteAccepted(RetrievalRouteAccepted),
    RetrievalRouteIgnored(RetrievalRouteIgnored),
    RetrievalRouteFailed(RetrievalRouteFailed),
    RetrievalResultUsed(RetrievalResultUsed),
    RetrievalDiscoveryItemPromoted(RetrievalDiscoveryItemPromoted),
    RetrievalPromotionSkipped(RetrievalPromotionSkipped),
    MemorySaved(MemorySaved),
    MemoryUpdated(MemoryUpdated),
    MemoryDeleted(MemoryDeleted),
    MemoryQueried(MemoryQueried),
    MemoryRecallReady(MemoryRecallReady),
    MemoryReembedQueued(MemoryReembedQueued),
    MemoryProviderChanged(MemoryProviderChanged),
    MemoryObservationRecorded(MemoryObservationRecorded),
    KnowledgeSaved(KnowledgeSaved),
    KnowledgeUpdated(KnowledgeUpdated),
    KnowledgeArchived(KnowledgeArchived),
    KnowledgeLinked(KnowledgeLinked),
    RemoteServerStarted(RemoteServerStarted),
    RemoteServerStopped(RemoteServerStopped),
    RemoteAuthFailed(RemoteAuthFailed),
    RemoteClientConnected(RemoteClientConnected),
    RemoteClientDisconnected(RemoteClientDisconnected),
    ThreadGoalUpdated(ThreadGoalUpdated),
    ThreadGoalCleared(ThreadGoalCleared),
    RoadmapChanged(RoadmapChanged),
    AutomationCreated(AutomationCreated),
    AutomationUpdated(AutomationUpdated),
    AutomationDeleted(AutomationDeleted),
    AutomationDue(AutomationDue),
    AutomationLeased(AutomationLeased),
    AutomationQueued(AutomationQueued),
    AutomationStarted(AutomationStarted),
    AutomationCompleted(AutomationCompleted),
    AutomationFailed(AutomationFailed),
    AutomationSkipped(AutomationSkipped),
    AutomationLeaseExpired(AutomationLeaseExpired),
    SkillsCatalogLoaded(SkillsCatalogLoaded),
    SkillConfigApplied(SkillConfigApplied),
    SkillActivationResolved(SkillActivationResolved),
    SkillIndexRendered(SkillIndexRendered),
    SkillInvoked(SkillInvoked),
    SkillAutoActivated(SkillAutoActivated),
    SkillSkipped(SkillSkipped),
    TaskStarted(TaskStarted),
    TaskOutput(TaskOutput),
    TaskCompleted(TaskCompleted),
    TaskFailed(TaskFailed),
    TaskCancelled(TaskCancelled),
    ProcessStarted(ProcessStarted),
    ProcessOutput(ProcessOutput),
    ProcessExited(ProcessExited),
    ProcessStopping(ProcessStopping),
    ProcessStopped(ProcessStopped),
    ProcessFailed(ProcessFailed),
    FileChangePreviewReady(FileChangePreviewReady),
    FileChanged(FileChanged),
    TranscriptItemAppended(TranscriptItemAppended),
    TurnCompleted(TurnCompleted),
    TurnFailed(TurnFailed),
    TurnPartialResult(TurnPartialResult),
    TurnDeadlineExceeded(TurnDeadlineExceeded),
    TurnInterrupted(TurnInterrupted),
    TurnSteered(TurnSteered),
    RunnerLifecycle(RunnerLifecycle),
    TeamStarted(TeamStarted),
    TeamMemberStarted(TeamMemberStarted),
    TeamMemberStatusChanged(TeamMemberStatusChanged),
    TeamMemberMessageDelta(TeamMemberMessageDelta),
    TeamMemberCompleted(TeamMemberCompleted),
    TeamDisplayModeChanged(TeamDisplayModeChanged),
    TeamTaskChanged(TeamTaskChanged),
    TeamCleanupCompleted(TeamCleanupCompleted),
}

impl RoderEvent {
    pub fn kind(&self) -> &'static str {
        match self {
            RoderEvent::RuntimeStarted(_) => "runtime.started",
            RoderEvent::ExtensionRegistered(_) => "extension.registered",
            RoderEvent::ExtensionEventEmitted(_) => "extension.event",
            RoderEvent::EventSinkFailed(_) => "extension.event_sink_failed",
            RoderEvent::ThreadCreated(_) => "thread.created",
            RoderEvent::ThreadLoaded(_) => "thread.loaded",
            RoderEvent::ThreadForkRequested(_) => "thread.fork_requested",
            RoderEvent::ThreadForked(_) => "thread.forked",
            RoderEvent::ThreadForkFailed(_) => "thread.fork_failed",
            RoderEvent::ThreadForkRemoved(_) => "thread.fork_removed",
            RoderEvent::TurnStarted(_) => "turn.started",
            RoderEvent::ContextAssemblyStarted(_) => "context.assembly_started",
            RoderEvent::ContextBlockAdded(_) => "context.block_added",
            RoderEvent::ContextAssemblyCompleted(_) => "context.assembly_completed",
            RoderEvent::ContextEntrypointCandidatesInjected(_) => {
                "context.entrypoint_candidates_injected"
            }
            RoderEvent::ContextCompactionStarted(_) => "context.compaction_started",
            RoderEvent::ContextCompactionRecorded(_) => "context.compaction_recorded",
            RoderEvent::InferenceRoutingDecision(_) => "inference.routing_decision",
            RoderEvent::InferenceStarted(_) => "inference.started",
            RoderEvent::InferenceEventReceived(_) => "inference.event_received",
            RoderEvent::ToolCallRequested(_) => "tool.call_requested",
            RoderEvent::ToolCallValidationRecorded(_) => "tool.call_validation",
            RoderEvent::ReliabilityFailureRecorded(_) => "reliability.failure",
            RoderEvent::ReliabilityRetryRecorded(_) => "reliability.retry",
            RoderEvent::ReliabilityLimitRecorded(_) => "reliability.limit",
            RoderEvent::ReliabilityMetricRecorded(_) => "reliability.metric",
            RoderEvent::CodeIndexingStarted(_) => "code_index.started",
            RoderEvent::CodeIndexChunked(_) => "code_index.chunked",
            RoderEvent::CodeIndexEmbedded(_) => "code_index.embedded",
            RoderEvent::CodeIndexReady(_) => "code_index.ready",
            RoderEvent::CodeIndexStale(_) => "code_index.stale",
            RoderEvent::CodeIndexFailed(_) => "code_index.failed",
            RoderEvent::CodeIndexProofFilteredResultDropped(_) => {
                "code_index.proof_filtered_result_dropped"
            }
            RoderEvent::ApprovalRequested(_) => "approval.requested",
            RoderEvent::ApprovalResolved(_) => "approval.resolved",
            RoderEvent::ExternalToolCallRequested(_) => "external_tool.requested",
            RoderEvent::ExternalToolCallResolved(_) => "external_tool.resolved",
            RoderEvent::UserInputRequested(_) => "user_input.requested",
            RoderEvent::UserInputResolved(_) => "user_input.resolved",
            RoderEvent::TaskLedgerUpdated(_) => "task_ledger.updated",
            RoderEvent::VerificationRequired(_) => "verification.required",
            RoderEvent::VerificationCompleted(_) => "verification.completed",
            RoderEvent::VerificationSkipped(_) => "verification.skipped",
            RoderEvent::PolicyDecisionRecorded(_) => "policy.decision",
            RoderEvent::PolicyBypassActive(_) => "policy.bypass_active",
            RoderEvent::PolicyModeChanged(_) => "policy.mode_changed",
            RoderEvent::PolicyExitPlanRequested(_) => "policy.exit_plan_requested",
            RoderEvent::PolicyExitPlanResolved(_) => "policy.exit_plan_resolved",
            RoderEvent::ToolCallStarted(_) => "tool.call_started",
            RoderEvent::ToolCallCompleted(_) => "tool.call_completed",
            RoderEvent::ToolOutputTruncated(_) => "tool.output_truncated",
            RoderEvent::SubagentStarted(_) => "subagent.started",
            RoderEvent::SubagentMessage(_) => "subagent.message",
            RoderEvent::SubagentToolCall(_) => "subagent.tool_call",
            RoderEvent::SubagentCompleted(_) => "subagent.completed",
            RoderEvent::SubagentFailed(_) => "subagent.failed",
            RoderEvent::SubagentTraceCreated(_) => "turn/subagentTraceCreated",
            RoderEvent::SubagentTraceDelta(_) => "turn/subagentTraceDelta",
            RoderEvent::SubagentTraceStatusChanged(_) => "turn/subagentTraceStatusChanged",
            RoderEvent::SubagentTraceCompleted(_) => "turn/subagentTraceCompleted",
            RoderEvent::SubagentTraceFailed(_) => "turn/subagentTraceFailed",
            RoderEvent::PlanReviewCreated(_) => "plan/reviewCreated",
            RoderEvent::PlanReviewStatusChanged(_) => "plan/reviewStatusChanged",
            RoderEvent::PlanReviewCommentAdded(_) => "plan/reviewCommentAdded",
            RoderEvent::PlanReviewRewritten(_) => "plan/reviewRewritten",
            RoderEvent::PlanReviewApproved(_) => "plan/reviewApproved",
            RoderEvent::PlanReviewRejected(_) => "plan/reviewRejected",
            RoderEvent::HunkRecorded(_) => "hunk/recorded",
            RoderEvent::WorkspaceChangeObserved(_) => "workspace/changeObserved",
            RoderEvent::HunkRollbackRequested(_) => "hunk/rollbackRequested",
            RoderEvent::HunkRollbackCompleted(_) => "hunk/rollbackCompleted",
            RoderEvent::WorkflowImportsDetected(_) => "workflow/importsDetected",
            RoderEvent::WorkflowImportPreviewed(_) => "workflow/importPreviewed",
            RoderEvent::WorkflowImportEnabled(_) => "workflow/importEnabled",
            RoderEvent::WorkflowImportDisabled(_) => "workflow/importDisabled",
            RoderEvent::WorkflowImportStale(_) => "workflow/importStale",
            RoderEvent::WorkflowImportFailed(_) => "workflow/importFailed",
            RoderEvent::WorkflowRunDrafted(_) => "workflows/drafted",
            RoderEvent::WorkflowApprovalRequested(_) => "workflows/approvalRequested",
            RoderEvent::WorkflowRunApproved(_) => "workflows/approved",
            RoderEvent::WorkflowRunDenied(_) => "workflows/denied",
            RoderEvent::WorkflowRunQueued(_) => "workflows/queued",
            RoderEvent::WorkflowRunStarted(_) => "workflows/started",
            RoderEvent::WorkflowPhaseStarted(_) => "workflows/phaseStarted",
            RoderEvent::WorkflowPhaseCompleted(_) => "workflows/phaseCompleted",
            RoderEvent::WorkflowAgentQueued(_) => "workflows/agentQueued",
            RoderEvent::WorkflowAgentStarted(_) => "workflows/agentStarted",
            RoderEvent::WorkflowAgentCompleted(_) => "workflows/agentCompleted",
            RoderEvent::WorkflowAgentFailed(_) => "workflows/agentFailed",
            RoderEvent::WorkflowOutputRecorded(_) => "workflows/outputRecorded",
            RoderEvent::WorkflowCheckpointRecorded(_) => "workflows/checkpointRecorded",
            RoderEvent::WorkflowRunPaused(_) => "workflows/paused",
            RoderEvent::WorkflowRunResumed(_) => "workflows/resumed",
            RoderEvent::WorkflowRunStopped(_) => "workflows/stopped",
            RoderEvent::WorkflowRunCompleted(_) => "workflows/completed",
            RoderEvent::WorkflowRunFailed(_) => "workflows/failed",
            RoderEvent::MediaArtifactCreated(_) => "media/artifactCreated",
            RoderEvent::MediaArtifactUpdated(_) => "media/artifactUpdated",
            RoderEvent::MediaArtifactDeleted(_) => "media/artifactDeleted",
            RoderEvent::MediaPreviewReady(_) => "media/previewReady",
            RoderEvent::ContextArtifactCreated(_) => "artifact/created",
            RoderEvent::ContextArtifactAppended(_) => "artifact/appended",
            RoderEvent::ContextArtifactCapped(_) => "artifact/capped",
            RoderEvent::ContextArtifactDeleted(_) => "artifact/deleted",
            RoderEvent::ContextArtifactRetentionExpired(_) => "artifact/retentionExpired",
            RoderEvent::DiscoveryCatalogBuilt(_) => "discovery/catalogBuilt",
            RoderEvent::DiscoveryItemUpdated(_) => "discovery/itemUpdated",
            RoderEvent::DiscoveryAuthRequired(_) => "discovery/authRequired",
            RoderEvent::DiscoveryItemRead(_) => "discovery/itemRead",
            RoderEvent::DiscoveryItemPromoted(_) => "discovery/itemPromoted",
            RoderEvent::DiscoveryPromotionReused(_) => "discovery/promotionReused",
            RoderEvent::DiscoveryWarmCacheHit(_) => "discovery/warmCacheHit",
            RoderEvent::DiscoveryPromotionExpired(_) => "discovery/promotionExpired",
            RoderEvent::RetrievalRoutePlanned(_) => "retrieval/routePlanned",
            RoderEvent::RetrievalRouteAccepted(_) => "retrieval/routeAccepted",
            RoderEvent::RetrievalRouteIgnored(_) => "retrieval/routeIgnored",
            RoderEvent::RetrievalRouteFailed(_) => "retrieval/routeFailed",
            RoderEvent::RetrievalResultUsed(_) => "retrieval/resultUsed",
            RoderEvent::RetrievalDiscoveryItemPromoted(_) => "retrieval/discoveryItemPromoted",
            RoderEvent::RetrievalPromotionSkipped(_) => "retrieval/promotionSkipped",
            RoderEvent::MemorySaved(_) => "memory/saved",
            RoderEvent::MemoryUpdated(_) => "memory/updated",
            RoderEvent::MemoryDeleted(_) => "memory/deleted",
            RoderEvent::MemoryQueried(_) => "memory/queried",
            RoderEvent::MemoryRecallReady(_) => "memory/recallReady",
            RoderEvent::MemoryReembedQueued(_) => "memory/reembedQueued",
            RoderEvent::MemoryProviderChanged(_) => "memory/providerChanged",
            RoderEvent::MemoryObservationRecorded(_) => "memory/observationRecorded",
            RoderEvent::KnowledgeSaved(_) => "knowledge/saved",
            RoderEvent::KnowledgeUpdated(_) => "knowledge/updated",
            RoderEvent::KnowledgeArchived(_) => "knowledge/archived",
            RoderEvent::KnowledgeLinked(_) => "knowledge/linked",
            RoderEvent::RemoteServerStarted(_) => "remote/serverStarted",
            RoderEvent::RemoteServerStopped(_) => "remote/serverStopped",
            RoderEvent::RemoteAuthFailed(_) => "remote/authFailed",
            RoderEvent::RemoteClientConnected(_) => "remote/clientConnected",
            RoderEvent::RemoteClientDisconnected(_) => "remote/clientDisconnected",
            RoderEvent::ThreadGoalUpdated(_) => "thread/goal/updated",
            RoderEvent::ThreadGoalCleared(_) => "thread/goal/cleared",
            RoderEvent::RoadmapChanged(_) => "roadmap.changed",
            RoderEvent::AutomationCreated(_) => "automations/created",
            RoderEvent::AutomationUpdated(_) => "automations/updated",
            RoderEvent::AutomationDeleted(_) => "automations/deleted",
            RoderEvent::AutomationDue(_) => "automations/due",
            RoderEvent::AutomationLeased(_) => "automations/leased",
            RoderEvent::AutomationQueued(_) => "automations/queued",
            RoderEvent::AutomationStarted(_) => "automations/started",
            RoderEvent::AutomationCompleted(_) => "automations/completed",
            RoderEvent::AutomationFailed(_) => "automations/failed",
            RoderEvent::AutomationSkipped(_) => "automations/skipped",
            RoderEvent::AutomationLeaseExpired(_) => "automations/leaseExpired",
            RoderEvent::SkillsCatalogLoaded(_) => "skills/catalogLoaded",
            RoderEvent::SkillConfigApplied(_) => "skills/configApplied",
            RoderEvent::SkillActivationResolved(_) => "skills/activationResolved",
            RoderEvent::SkillIndexRendered(_) => "skills/indexRendered",
            RoderEvent::SkillInvoked(_) => "skills/invoked",
            RoderEvent::SkillAutoActivated(_) => "skills/autoActivated",
            RoderEvent::SkillSkipped(_) => "skills/skipped",
            RoderEvent::TaskStarted(_) => "task.started",
            RoderEvent::TaskOutput(_) => "task.output",
            RoderEvent::TaskCompleted(_) => "task.completed",
            RoderEvent::TaskFailed(_) => "task.failed",
            RoderEvent::TaskCancelled(_) => "task.cancelled",
            RoderEvent::ProcessStarted(_) => "process.started",
            RoderEvent::ProcessOutput(_) => "process.output",
            RoderEvent::ProcessExited(_) => "process.exited",
            RoderEvent::ProcessStopping(_) => "process.stopping",
            RoderEvent::ProcessStopped(_) => "process.stopped",
            RoderEvent::ProcessFailed(_) => "process.failed",
            RoderEvent::FileChangePreviewReady(_) => "file.change_preview_ready",
            RoderEvent::FileChanged(_) => "file.changed",
            RoderEvent::TranscriptItemAppended(_) => "turn.transcript_item_appended",
            RoderEvent::TurnCompleted(_) => "turn.completed",
            RoderEvent::TurnFailed(_) => "turn.failed",
            RoderEvent::TurnPartialResult(_) => "turn.partial_result",
            RoderEvent::TurnDeadlineExceeded(_) => "turn.deadline_exceeded",
            RoderEvent::TurnInterrupted(_) => "turn.interrupted",
            RoderEvent::TurnSteered(_) => "turn.steered",
            RoderEvent::RunnerLifecycle(_) => "runner.lifecycle",
            RoderEvent::TeamStarted(_) => "team.started",
            RoderEvent::TeamMemberStarted(_) => "team.member_started",
            RoderEvent::TeamMemberStatusChanged(_) => "team.member_status_changed",
            RoderEvent::TeamMemberMessageDelta(_) => "team.member_message_delta",
            RoderEvent::TeamMemberCompleted(_) => "team.member_completed",
            RoderEvent::TeamDisplayModeChanged(_) => "team.display_mode_changed",
            RoderEvent::TeamTaskChanged(_) => "team.task_changed",
            RoderEvent::TeamCleanupCompleted(_) => "team.cleanup_completed",
        }
    }

    pub fn source(&self) -> EventSource {
        match self {
            RoderEvent::InferenceEventReceived(_) | RoderEvent::InferenceStarted(_) => {
                EventSource::Provider
            }
            RoderEvent::InferenceRoutingDecision(_) => EventSource::Core,
            RoderEvent::ReliabilityRetryRecorded(_) => EventSource::Provider,
            RoderEvent::ReliabilityFailureRecorded(_)
            | RoderEvent::ReliabilityLimitRecorded(_)
            | RoderEvent::ReliabilityMetricRecorded(_) => EventSource::Core,
            RoderEvent::ToolCallRequested(_)
            | RoderEvent::ToolCallValidationRecorded(_)
            | RoderEvent::ToolCallStarted(_)
            | RoderEvent::ToolCallCompleted(_) => EventSource::Tool,
            RoderEvent::SubagentStarted(_)
            | RoderEvent::SubagentMessage(_)
            | RoderEvent::SubagentToolCall(_)
            | RoderEvent::SubagentCompleted(_)
            | RoderEvent::SubagentFailed(_)
            | RoderEvent::SubagentTraceCreated(_)
            | RoderEvent::SubagentTraceDelta(_)
            | RoderEvent::SubagentTraceStatusChanged(_)
            | RoderEvent::SubagentTraceCompleted(_)
            | RoderEvent::SubagentTraceFailed(_)
            | RoderEvent::PlanReviewCreated(_)
            | RoderEvent::PlanReviewStatusChanged(_)
            | RoderEvent::PlanReviewCommentAdded(_)
            | RoderEvent::PlanReviewRewritten(_)
            | RoderEvent::PlanReviewApproved(_)
            | RoderEvent::PlanReviewRejected(_)
            | RoderEvent::HunkRecorded(_)
            | RoderEvent::WorkspaceChangeObserved(_)
            | RoderEvent::HunkRollbackRequested(_)
            | RoderEvent::HunkRollbackCompleted(_)
            | RoderEvent::WorkflowImportsDetected(_)
            | RoderEvent::WorkflowImportPreviewed(_)
            | RoderEvent::WorkflowImportEnabled(_)
            | RoderEvent::WorkflowImportDisabled(_)
            | RoderEvent::WorkflowImportStale(_)
            | RoderEvent::WorkflowImportFailed(_)
            | RoderEvent::MediaArtifactCreated(_)
            | RoderEvent::MediaArtifactUpdated(_)
            | RoderEvent::MediaArtifactDeleted(_)
            | RoderEvent::MediaPreviewReady(_)
            | RoderEvent::ContextArtifactCreated(_)
            | RoderEvent::ContextArtifactAppended(_)
            | RoderEvent::ContextArtifactCapped(_)
            | RoderEvent::ContextArtifactDeleted(_)
            | RoderEvent::ContextArtifactRetentionExpired(_)
            | RoderEvent::DiscoveryCatalogBuilt(_)
            | RoderEvent::DiscoveryItemUpdated(_)
            | RoderEvent::DiscoveryAuthRequired(_)
            | RoderEvent::DiscoveryItemRead(_)
            | RoderEvent::DiscoveryItemPromoted(_)
            | RoderEvent::DiscoveryPromotionReused(_)
            | RoderEvent::DiscoveryWarmCacheHit(_)
            | RoderEvent::DiscoveryPromotionExpired(_)
            | RoderEvent::RetrievalRoutePlanned(_)
            | RoderEvent::RetrievalRouteAccepted(_)
            | RoderEvent::RetrievalRouteIgnored(_)
            | RoderEvent::RetrievalRouteFailed(_)
            | RoderEvent::RetrievalResultUsed(_)
            | RoderEvent::RetrievalDiscoveryItemPromoted(_)
            | RoderEvent::RetrievalPromotionSkipped(_)
            | RoderEvent::MemorySaved(_)
            | RoderEvent::MemoryUpdated(_)
            | RoderEvent::MemoryDeleted(_)
            | RoderEvent::MemoryQueried(_)
            | RoderEvent::MemoryRecallReady(_)
            | RoderEvent::MemoryReembedQueued(_)
            | RoderEvent::MemoryProviderChanged(_)
            | RoderEvent::MemoryObservationRecorded(_)
            | RoderEvent::TaskStarted(_)
            | RoderEvent::TaskOutput(_)
            | RoderEvent::TaskCompleted(_)
            | RoderEvent::TaskFailed(_)
            | RoderEvent::TaskCancelled(_)
            | RoderEvent::ProcessStarted(_)
            | RoderEvent::ProcessOutput(_)
            | RoderEvent::ProcessExited(_)
            | RoderEvent::ProcessStopping(_)
            | RoderEvent::ProcessStopped(_)
            | RoderEvent::ProcessFailed(_) => EventSource::Extension,
            RoderEvent::RemoteServerStarted(_)
            | RoderEvent::RemoteServerStopped(_)
            | RoderEvent::RemoteAuthFailed(_)
            | RoderEvent::RemoteClientConnected(_)
            | RoderEvent::RemoteClientDisconnected(_) => EventSource::AppServer,
            RoderEvent::RoadmapChanged(_)
            | RoderEvent::ThreadGoalUpdated(_)
            | RoderEvent::ThreadGoalCleared(_)
            | RoderEvent::AutomationCreated(_)
            | RoderEvent::AutomationUpdated(_)
            | RoderEvent::AutomationDeleted(_)
            | RoderEvent::AutomationDue(_)
            | RoderEvent::AutomationLeased(_)
            | RoderEvent::AutomationQueued(_)
            | RoderEvent::AutomationStarted(_)
            | RoderEvent::AutomationCompleted(_)
            | RoderEvent::AutomationFailed(_)
            | RoderEvent::AutomationSkipped(_)
            | RoderEvent::AutomationLeaseExpired(_)
            | RoderEvent::SkillsCatalogLoaded(_)
            | RoderEvent::SkillConfigApplied(_)
            | RoderEvent::SkillActivationResolved(_)
            | RoderEvent::SkillIndexRendered(_)
            | RoderEvent::SkillInvoked(_)
            | RoderEvent::SkillAutoActivated(_)
            | RoderEvent::SkillSkipped(_) => EventSource::Core,
            RoderEvent::FileChangePreviewReady(_) => EventSource::Tool,
            RoderEvent::UserInputRequested(_)
            | RoderEvent::UserInputResolved(_)
            | RoderEvent::TaskLedgerUpdated(_)
            | RoderEvent::TurnPartialResult(_)
            | RoderEvent::TurnDeadlineExceeded(_)
            | RoderEvent::VerificationRequired(_)
            | RoderEvent::VerificationCompleted(_)
            | RoderEvent::VerificationSkipped(_) => EventSource::Core,
            RoderEvent::ExtensionRegistered(_) => EventSource::Extension,
            RoderEvent::RunnerLifecycle(_) => EventSource::Extension,
            RoderEvent::TeamStarted(_)
            | RoderEvent::TeamMemberStarted(_)
            | RoderEvent::TeamMemberStatusChanged(_)
            | RoderEvent::TeamMemberMessageDelta(_)
            | RoderEvent::TeamMemberCompleted(_)
            | RoderEvent::TeamDisplayModeChanged(_)
            | RoderEvent::TeamTaskChanged(_)
            | RoderEvent::TeamCleanupCompleted(_) => EventSource::Core,
            _ => EventSource::Core,
        }
    }

    pub fn thread_id(&self) -> Option<&ThreadId> {
        match self {
            RoderEvent::ExtensionEventEmitted(_) | RoderEvent::EventSinkFailed(_) => None,
            RoderEvent::ThreadCreated(e) => Some(&e.thread_id),
            RoderEvent::ThreadLoaded(e) => Some(&e.thread_id),
            RoderEvent::ThreadForkRequested(e) => Some(&e.parent_thread_id),
            RoderEvent::ThreadForked(e) => Some(&e.child_thread_id),
            RoderEvent::ThreadForkFailed(e) => Some(&e.parent_thread_id),
            RoderEvent::ThreadForkRemoved(e) => Some(&e.thread_id),
            RoderEvent::TurnStarted(e) => Some(&e.thread_id),
            RoderEvent::ContextAssemblyStarted(e) => Some(&e.thread_id),
            RoderEvent::ContextBlockAdded(e) => Some(&e.thread_id),
            RoderEvent::ContextAssemblyCompleted(e) => Some(&e.thread_id),
            RoderEvent::ContextEntrypointCandidatesInjected(e) => Some(&e.thread_id),
            RoderEvent::ContextCompactionStarted(e) => Some(&e.thread_id),
            RoderEvent::ContextCompactionRecorded(e) => Some(&e.thread_id),
            RoderEvent::InferenceRoutingDecision(e) => Some(&e.thread_id),
            RoderEvent::InferenceStarted(e) => Some(&e.thread_id),
            RoderEvent::InferenceEventReceived(e) => Some(&e.thread_id),
            RoderEvent::ToolCallRequested(e) => Some(&e.thread_id),
            RoderEvent::ToolCallValidationRecorded(e) => Some(&e.thread_id),
            RoderEvent::ReliabilityFailureRecorded(e) => Some(&e.context.thread_id),
            RoderEvent::ReliabilityRetryRecorded(e) => Some(&e.context.thread_id),
            RoderEvent::ReliabilityLimitRecorded(e) => Some(&e.context.thread_id),
            RoderEvent::ReliabilityMetricRecorded(e) => Some(&e.context.thread_id),
            RoderEvent::CodeIndexingStarted(e) => e.context.thread_id.as_ref(),
            RoderEvent::CodeIndexChunked(e) => e.context.thread_id.as_ref(),
            RoderEvent::CodeIndexEmbedded(e) => e.context.thread_id.as_ref(),
            RoderEvent::CodeIndexReady(_) => None,
            RoderEvent::CodeIndexStale(e) => e.context.thread_id.as_ref(),
            RoderEvent::CodeIndexFailed(e) => e.context.thread_id.as_ref(),
            RoderEvent::CodeIndexProofFilteredResultDropped(e) => e.context.thread_id.as_ref(),
            RoderEvent::ApprovalRequested(e) => Some(&e.thread_id),
            RoderEvent::ApprovalResolved(e) => Some(&e.thread_id),
            RoderEvent::ExternalToolCallRequested(e) => Some(&e.thread_id),
            RoderEvent::ExternalToolCallResolved(e) => Some(&e.thread_id),
            RoderEvent::UserInputRequested(e) => Some(&e.thread_id),
            RoderEvent::UserInputResolved(e) => Some(&e.thread_id),
            RoderEvent::TaskLedgerUpdated(e) => Some(&e.thread_id),
            RoderEvent::VerificationRequired(e) => Some(&e.thread_id),
            RoderEvent::VerificationCompleted(e) => Some(&e.thread_id),
            RoderEvent::VerificationSkipped(e) => Some(&e.thread_id),
            RoderEvent::PolicyDecisionRecorded(e) => Some(&e.thread_id),
            RoderEvent::PolicyBypassActive(e) => Some(&e.thread_id),
            RoderEvent::PolicyModeChanged(e) => Some(&e.thread_id),
            RoderEvent::PolicyExitPlanRequested(e) => Some(&e.thread_id),
            RoderEvent::PolicyExitPlanResolved(e) => Some(&e.thread_id),
            RoderEvent::ToolCallStarted(e) => Some(&e.thread_id),
            RoderEvent::ToolCallCompleted(e) => Some(&e.thread_id),
            RoderEvent::ToolOutputTruncated(e) => Some(&e.thread_id),
            RoderEvent::SubagentStarted(e) => Some(&e.thread_id),
            RoderEvent::SubagentMessage(e) => Some(&e.thread_id),
            RoderEvent::SubagentToolCall(e) => Some(&e.thread_id),
            RoderEvent::SubagentCompleted(e) => Some(&e.thread_id),
            RoderEvent::SubagentFailed(e) => Some(&e.thread_id),
            RoderEvent::SubagentTraceCreated(e) => Some(&e.summary.parent.thread_id),
            RoderEvent::SubagentTraceDelta(e) => Some(&e.delta.parent.thread_id),
            RoderEvent::SubagentTraceStatusChanged(e) => Some(&e.parent.thread_id),
            RoderEvent::SubagentTraceCompleted(e) => Some(&e.summary.parent.thread_id),
            RoderEvent::SubagentTraceFailed(e) => Some(&e.summary.parent.thread_id),
            RoderEvent::PlanReviewCreated(e) => Some(&e.review.thread_id),
            RoderEvent::PlanReviewStatusChanged(e) => Some(&e.thread_id),
            RoderEvent::PlanReviewCommentAdded(e) => Some(&e.thread_id),
            RoderEvent::PlanReviewRewritten(e) => Some(&e.thread_id),
            RoderEvent::PlanReviewApproved(e) => Some(&e.thread_id),
            RoderEvent::PlanReviewRejected(e) => Some(&e.thread_id),
            RoderEvent::HunkRecorded(e) => Some(&e.hunk.thread_id),
            RoderEvent::WorkspaceChangeObserved(e) => Some(&e.change.thread_id),
            RoderEvent::HunkRollbackRequested(e) => Some(&e.thread_id),
            RoderEvent::HunkRollbackCompleted(e) => Some(&e.thread_id),
            RoderEvent::MediaArtifactCreated(e) => Some(&e.thread_id),
            RoderEvent::MediaArtifactUpdated(e) => Some(&e.thread_id),
            RoderEvent::MediaPreviewReady(e) => Some(&e.thread_id),
            RoderEvent::ContextArtifactCreated(e) => Some(&e.thread_id),
            RoderEvent::ContextArtifactAppended(e) => Some(&e.thread_id),
            RoderEvent::ContextArtifactCapped(e) => Some(&e.thread_id),
            RoderEvent::ContextArtifactDeleted(e) => Some(&e.thread_id),
            RoderEvent::ContextArtifactRetentionExpired(e) => Some(&e.thread_id),
            RoderEvent::DiscoveryItemRead(e) => Some(&e.thread_id),
            RoderEvent::DiscoveryItemPromoted(e) => Some(&e.record.thread_id),
            RoderEvent::DiscoveryPromotionReused(e) => Some(&e.record.thread_id),
            RoderEvent::DiscoveryWarmCacheHit(e) => Some(&e.record.thread_id),
            RoderEvent::DiscoveryPromotionExpired(e) => Some(&e.record.thread_id),
            RoderEvent::RetrievalRoutePlanned(e) => Some(&e.plan.thread_id),
            RoderEvent::RetrievalRouteAccepted(e) => Some(&e.thread_id),
            RoderEvent::RetrievalRouteIgnored(e) => Some(&e.thread_id),
            RoderEvent::RetrievalRouteFailed(e) => Some(&e.thread_id),
            RoderEvent::RetrievalResultUsed(e) => Some(&e.thread_id),
            RoderEvent::RetrievalDiscoveryItemPromoted(e) => Some(&e.thread_id),
            RoderEvent::RetrievalPromotionSkipped(e) => Some(&e.thread_id),
            RoderEvent::MemoryRecallReady(e) => Some(&e.thread_id),
            RoderEvent::MemoryObservationRecorded(e) => Some(&e.thread_id),
            RoderEvent::TaskStarted(e) => e.thread_id.as_ref(),
            RoderEvent::TaskOutput(e) => e.thread_id.as_ref(),
            RoderEvent::TaskCompleted(e) => e.thread_id.as_ref(),
            RoderEvent::TaskFailed(e) => e.thread_id.as_ref(),
            RoderEvent::TaskCancelled(e) => e.thread_id.as_ref(),
            RoderEvent::FileChangePreviewReady(e) => Some(&e.thread_id),
            RoderEvent::FileChanged(e) => Some(&e.thread_id),
            RoderEvent::TranscriptItemAppended(e) => Some(&e.thread_id),
            RoderEvent::TurnCompleted(e) => Some(&e.thread_id),
            RoderEvent::TurnFailed(e) => Some(&e.thread_id),
            RoderEvent::TurnPartialResult(e) => Some(&e.thread_id),
            RoderEvent::TurnDeadlineExceeded(e) => Some(&e.thread_id),
            RoderEvent::TurnInterrupted(e) => Some(&e.thread_id),
            RoderEvent::TurnSteered(e) => Some(&e.thread_id),
            RoderEvent::ThreadGoalUpdated(e) => Some(&e.thread_id),
            RoderEvent::ThreadGoalCleared(e) => Some(&e.thread_id),
            RoderEvent::TeamStarted(e) => Some(&e.lead_thread_id),
            RoderEvent::TeamMemberStarted(e) => Some(&e.member_thread_id),
            RoderEvent::TeamMemberStatusChanged(e) => Some(&e.member_thread_id),
            RoderEvent::TeamMemberMessageDelta(e) => Some(&e.member_thread_id),
            RoderEvent::TeamMemberCompleted(e) => Some(&e.member_thread_id),
            RoderEvent::SkillActivationResolved(e) => Some(&e.thread_id),
            RoderEvent::SkillIndexRendered(e) => Some(&e.thread_id),
            RoderEvent::SkillInvoked(e) => Some(&e.thread_id),
            RoderEvent::SkillAutoActivated(e) => Some(&e.thread_id),
            RoderEvent::SkillSkipped(e) => Some(&e.thread_id),
            RoderEvent::RuntimeStarted(_)
            | RoderEvent::ExtensionRegistered(_)
            | RoderEvent::WorkflowImportsDetected(_)
            | RoderEvent::WorkflowImportPreviewed(_)
            | RoderEvent::WorkflowImportEnabled(_)
            | RoderEvent::WorkflowImportDisabled(_)
            | RoderEvent::WorkflowImportStale(_)
            | RoderEvent::WorkflowImportFailed(_)
            | RoderEvent::MediaArtifactDeleted(_)
            | RoderEvent::DiscoveryCatalogBuilt(_)
            | RoderEvent::DiscoveryItemUpdated(_)
            | RoderEvent::DiscoveryAuthRequired(_)
            | RoderEvent::MemorySaved(_)
            | RoderEvent::MemoryUpdated(_)
            | RoderEvent::MemoryDeleted(_)
            | RoderEvent::MemoryQueried(_)
            | RoderEvent::MemoryReembedQueued(_)
            | RoderEvent::MemoryProviderChanged(_)
            | RoderEvent::KnowledgeSaved(_)
            | RoderEvent::KnowledgeUpdated(_)
            | RoderEvent::KnowledgeArchived(_)
            | RoderEvent::KnowledgeLinked(_)
            | RoderEvent::RemoteServerStarted(_)
            | RoderEvent::RemoteServerStopped(_)
            | RoderEvent::RemoteAuthFailed(_)
            | RoderEvent::RemoteClientConnected(_)
            | RoderEvent::RemoteClientDisconnected(_)
            | RoderEvent::RoadmapChanged(_)
            | RoderEvent::AutomationCreated(_)
            | RoderEvent::AutomationUpdated(_)
            | RoderEvent::AutomationDeleted(_)
            | RoderEvent::AutomationDue(_)
            | RoderEvent::AutomationLeased(_)
            | RoderEvent::AutomationQueued(_)
            | RoderEvent::AutomationSkipped(_)
            | RoderEvent::AutomationLeaseExpired(_)
            | RoderEvent::SkillsCatalogLoaded(_)
            | RoderEvent::SkillConfigApplied(_)
            | RoderEvent::RunnerLifecycle(_)
            | RoderEvent::TeamDisplayModeChanged(_)
            | RoderEvent::TeamTaskChanged(_)
            | RoderEvent::TeamCleanupCompleted(_) => None,
            RoderEvent::ProcessStarted(e) => e.process.thread_id.as_ref(),
            RoderEvent::ProcessOutput(e) => e.thread_id.as_ref(),
            RoderEvent::ProcessExited(e) => e.process.thread_id.as_ref(),
            RoderEvent::ProcessStopping(_) => None,
            RoderEvent::ProcessStopped(e) => e.process.thread_id.as_ref(),
            RoderEvent::ProcessFailed(e) => e.process.thread_id.as_ref(),
            RoderEvent::AutomationStarted(e) => e.run.thread_id.as_ref(),
            RoderEvent::AutomationCompleted(e) => e.run.thread_id.as_ref(),
            RoderEvent::AutomationFailed(e) => e.run.thread_id.as_ref(),
            RoderEvent::WorkflowRunDrafted(e) => e.thread_id.as_ref(),
            RoderEvent::WorkflowApprovalRequested(e) => e.thread_id.as_ref(),
            RoderEvent::WorkflowRunApproved(e) => e.thread_id.as_ref(),
            RoderEvent::WorkflowRunDenied(e) => e.thread_id.as_ref(),
            RoderEvent::WorkflowRunQueued(e) => e.thread_id.as_ref(),
            RoderEvent::WorkflowRunStarted(e) => e.thread_id.as_ref(),
            RoderEvent::WorkflowPhaseStarted(e) => e.thread_id.as_ref(),
            RoderEvent::WorkflowPhaseCompleted(e) => e.thread_id.as_ref(),
            RoderEvent::WorkflowAgentQueued(e) => e.thread_id.as_ref(),
            RoderEvent::WorkflowAgentStarted(e) => e.thread_id.as_ref(),
            RoderEvent::WorkflowAgentCompleted(e) => e.thread_id.as_ref(),
            RoderEvent::WorkflowAgentFailed(e) => e.thread_id.as_ref(),
            RoderEvent::WorkflowOutputRecorded(e) => e.thread_id.as_ref(),
            RoderEvent::WorkflowCheckpointRecorded(e) => e.thread_id.as_ref(),
            RoderEvent::WorkflowRunPaused(e) => e.thread_id.as_ref(),
            RoderEvent::WorkflowRunResumed(e) => e.thread_id.as_ref(),
            RoderEvent::WorkflowRunStopped(e) => e.thread_id.as_ref(),
            RoderEvent::WorkflowRunCompleted(e) => e.thread_id.as_ref(),
            RoderEvent::WorkflowRunFailed(e) => e.thread_id.as_ref(),
        }
    }

    pub fn turn_id(&self) -> Option<&TurnId> {
        match self {
            RoderEvent::ExtensionEventEmitted(_) | RoderEvent::EventSinkFailed(_) => None,
            RoderEvent::ThreadForkRequested(_)
            | RoderEvent::ThreadForked(_)
            | RoderEvent::ThreadForkFailed(_)
            | RoderEvent::ThreadForkRemoved(_) => None,
            RoderEvent::TurnStarted(e) => Some(&e.turn_id),
            RoderEvent::ContextAssemblyStarted(e) => Some(&e.turn_id),
            RoderEvent::ContextBlockAdded(e) => Some(&e.turn_id),
            RoderEvent::ContextAssemblyCompleted(e) => Some(&e.turn_id),
            RoderEvent::ContextEntrypointCandidatesInjected(e) => Some(&e.turn_id),
            RoderEvent::ContextCompactionStarted(e) => Some(&e.turn_id),
            RoderEvent::ContextCompactionRecorded(e) => Some(&e.turn_id),
            RoderEvent::InferenceRoutingDecision(e) => Some(&e.turn_id),
            RoderEvent::InferenceStarted(e) => Some(&e.turn_id),
            RoderEvent::InferenceEventReceived(e) => Some(&e.turn_id),
            RoderEvent::ToolCallRequested(e) => Some(&e.turn_id),
            RoderEvent::ToolCallValidationRecorded(e) => Some(&e.turn_id),
            RoderEvent::ReliabilityFailureRecorded(e) => Some(&e.context.turn_id),
            RoderEvent::ReliabilityRetryRecorded(e) => Some(&e.context.turn_id),
            RoderEvent::ReliabilityLimitRecorded(e) => Some(&e.context.turn_id),
            RoderEvent::ReliabilityMetricRecorded(e) => Some(&e.context.turn_id),
            RoderEvent::CodeIndexingStarted(e) => e.context.turn_id.as_ref(),
            RoderEvent::CodeIndexChunked(e) => e.context.turn_id.as_ref(),
            RoderEvent::CodeIndexEmbedded(e) => e.context.turn_id.as_ref(),
            RoderEvent::CodeIndexReady(_) => None,
            RoderEvent::CodeIndexStale(e) => e.context.turn_id.as_ref(),
            RoderEvent::CodeIndexFailed(e) => e.context.turn_id.as_ref(),
            RoderEvent::CodeIndexProofFilteredResultDropped(e) => e.context.turn_id.as_ref(),
            RoderEvent::ApprovalRequested(e) => Some(&e.turn_id),
            RoderEvent::ApprovalResolved(e) => Some(&e.turn_id),
            RoderEvent::ExternalToolCallRequested(e) => Some(&e.turn_id),
            RoderEvent::ExternalToolCallResolved(e) => Some(&e.turn_id),
            RoderEvent::UserInputRequested(e) => Some(&e.turn_id),
            RoderEvent::UserInputResolved(e) => Some(&e.turn_id),
            RoderEvent::TaskLedgerUpdated(e) => Some(&e.turn_id),
            RoderEvent::VerificationRequired(e) => Some(&e.turn_id),
            RoderEvent::VerificationCompleted(e) => Some(&e.turn_id),
            RoderEvent::VerificationSkipped(e) => Some(&e.turn_id),
            RoderEvent::PolicyDecisionRecorded(e) => Some(&e.turn_id),
            RoderEvent::PolicyBypassActive(e) => Some(&e.turn_id),
            RoderEvent::PolicyModeChanged(e) => e.turn_id.as_ref(),
            RoderEvent::PolicyExitPlanRequested(e) => Some(&e.turn_id),
            RoderEvent::PolicyExitPlanResolved(e) => Some(&e.turn_id),
            RoderEvent::ToolCallStarted(e) => Some(&e.turn_id),
            RoderEvent::ToolCallCompleted(e) => Some(&e.turn_id),
            RoderEvent::ToolOutputTruncated(e) => Some(&e.turn_id),
            RoderEvent::SubagentStarted(e) => Some(&e.turn_id),
            RoderEvent::SubagentMessage(e) => Some(&e.turn_id),
            RoderEvent::SubagentToolCall(e) => Some(&e.turn_id),
            RoderEvent::SubagentCompleted(e) => Some(&e.turn_id),
            RoderEvent::SubagentFailed(e) => Some(&e.turn_id),
            RoderEvent::SubagentTraceCreated(e) => Some(&e.summary.parent.turn_id),
            RoderEvent::SubagentTraceDelta(e) => Some(&e.delta.parent.turn_id),
            RoderEvent::SubagentTraceStatusChanged(e) => Some(&e.parent.turn_id),
            RoderEvent::SubagentTraceCompleted(e) => Some(&e.summary.parent.turn_id),
            RoderEvent::SubagentTraceFailed(e) => Some(&e.summary.parent.turn_id),
            RoderEvent::PlanReviewCreated(e) => Some(&e.review.turn_id),
            RoderEvent::PlanReviewStatusChanged(e) => Some(&e.turn_id),
            RoderEvent::PlanReviewCommentAdded(e) => Some(&e.turn_id),
            RoderEvent::PlanReviewRewritten(e) => Some(&e.turn_id),
            RoderEvent::PlanReviewApproved(e) => Some(&e.turn_id),
            RoderEvent::PlanReviewRejected(e) => Some(&e.turn_id),
            RoderEvent::HunkRecorded(e) => Some(&e.hunk.turn_id),
            RoderEvent::WorkspaceChangeObserved(e) => Some(&e.change.turn_id),
            RoderEvent::HunkRollbackRequested(e) => Some(&e.turn_id),
            RoderEvent::HunkRollbackCompleted(e) => Some(&e.turn_id),
            RoderEvent::MediaArtifactCreated(e) => Some(&e.turn_id),
            RoderEvent::MediaArtifactUpdated(e) => Some(&e.turn_id),
            RoderEvent::MediaPreviewReady(e) => Some(&e.turn_id),
            RoderEvent::ContextArtifactCreated(e) => Some(&e.turn_id),
            RoderEvent::ContextArtifactAppended(e) => Some(&e.turn_id),
            RoderEvent::ContextArtifactCapped(e) => Some(&e.turn_id),
            RoderEvent::DiscoveryItemRead(e) => Some(&e.turn_id),
            RoderEvent::DiscoveryItemPromoted(e) => e.record.turn_id.as_ref(),
            RoderEvent::DiscoveryPromotionReused(e) => e.record.turn_id.as_ref(),
            RoderEvent::DiscoveryWarmCacheHit(e) => e.record.turn_id.as_ref(),
            RoderEvent::DiscoveryPromotionExpired(e) => e.record.turn_id.as_ref(),
            RoderEvent::RetrievalRoutePlanned(e) => Some(&e.plan.turn_id),
            RoderEvent::RetrievalRouteAccepted(e) => Some(&e.turn_id),
            RoderEvent::RetrievalRouteIgnored(e) => Some(&e.turn_id),
            RoderEvent::RetrievalRouteFailed(e) => Some(&e.turn_id),
            RoderEvent::RetrievalResultUsed(e) => Some(&e.turn_id),
            RoderEvent::RetrievalDiscoveryItemPromoted(e) => Some(&e.turn_id),
            RoderEvent::RetrievalPromotionSkipped(e) => Some(&e.turn_id),
            RoderEvent::MemoryRecallReady(e) => Some(&e.turn_id),
            RoderEvent::MemoryObservationRecorded(e) => Some(&e.turn_id),
            RoderEvent::TaskStarted(e) => e.turn_id.as_ref(),
            RoderEvent::TaskOutput(e) => e.turn_id.as_ref(),
            RoderEvent::TaskCompleted(e) => e.turn_id.as_ref(),
            RoderEvent::TaskFailed(e) => e.turn_id.as_ref(),
            RoderEvent::TaskCancelled(e) => e.turn_id.as_ref(),
            RoderEvent::FileChangePreviewReady(e) => Some(&e.turn_id),
            RoderEvent::FileChanged(e) => Some(&e.turn_id),
            RoderEvent::TranscriptItemAppended(e) => Some(&e.turn_id),
            RoderEvent::TurnCompleted(e) => Some(&e.turn_id),
            RoderEvent::TurnFailed(e) => Some(&e.turn_id),
            RoderEvent::TurnPartialResult(e) => Some(&e.turn_id),
            RoderEvent::TurnDeadlineExceeded(e) => Some(&e.turn_id),
            RoderEvent::TurnInterrupted(e) => Some(&e.turn_id),
            RoderEvent::TurnSteered(e) => Some(&e.turn_id),
            RoderEvent::TeamMemberMessageDelta(e) => Some(&e.turn_id),
            RoderEvent::TeamMemberCompleted(e) => e.turn_id.as_ref(),
            RoderEvent::SkillActivationResolved(e) => Some(&e.turn_id),
            RoderEvent::SkillIndexRendered(e) => Some(&e.turn_id),
            RoderEvent::SkillInvoked(e) => Some(&e.turn_id),
            RoderEvent::SkillAutoActivated(e) => Some(&e.turn_id),
            RoderEvent::SkillSkipped(e) => Some(&e.turn_id),
            RoderEvent::RuntimeStarted(_)
            | RoderEvent::ExtensionRegistered(_)
            | RoderEvent::ThreadCreated(_)
            | RoderEvent::ThreadLoaded(_)
            | RoderEvent::WorkflowImportsDetected(_)
            | RoderEvent::WorkflowImportPreviewed(_)
            | RoderEvent::WorkflowImportEnabled(_)
            | RoderEvent::WorkflowImportDisabled(_)
            | RoderEvent::WorkflowImportStale(_)
            | RoderEvent::WorkflowImportFailed(_)
            | RoderEvent::MediaArtifactDeleted(_)
            | RoderEvent::ContextArtifactDeleted(_)
            | RoderEvent::ContextArtifactRetentionExpired(_)
            | RoderEvent::DiscoveryCatalogBuilt(_)
            | RoderEvent::DiscoveryItemUpdated(_)
            | RoderEvent::DiscoveryAuthRequired(_)
            | RoderEvent::MemorySaved(_)
            | RoderEvent::MemoryUpdated(_)
            | RoderEvent::MemoryDeleted(_)
            | RoderEvent::MemoryQueried(_)
            | RoderEvent::MemoryReembedQueued(_)
            | RoderEvent::MemoryProviderChanged(_)
            | RoderEvent::KnowledgeSaved(_)
            | RoderEvent::KnowledgeUpdated(_)
            | RoderEvent::KnowledgeArchived(_)
            | RoderEvent::KnowledgeLinked(_)
            | RoderEvent::RemoteServerStarted(_)
            | RoderEvent::RemoteServerStopped(_)
            | RoderEvent::RemoteAuthFailed(_)
            | RoderEvent::RemoteClientConnected(_)
            | RoderEvent::RemoteClientDisconnected(_)
            | RoderEvent::ThreadGoalUpdated(_)
            | RoderEvent::ThreadGoalCleared(_)
            | RoderEvent::RoadmapChanged(_)
            | RoderEvent::AutomationCreated(_)
            | RoderEvent::AutomationUpdated(_)
            | RoderEvent::AutomationDeleted(_)
            | RoderEvent::AutomationDue(_)
            | RoderEvent::AutomationLeased(_)
            | RoderEvent::AutomationQueued(_)
            | RoderEvent::AutomationSkipped(_)
            | RoderEvent::AutomationLeaseExpired(_)
            | RoderEvent::SkillsCatalogLoaded(_)
            | RoderEvent::SkillConfigApplied(_)
            | RoderEvent::RunnerLifecycle(_)
            | RoderEvent::TeamStarted(_)
            | RoderEvent::TeamMemberStarted(_)
            | RoderEvent::TeamMemberStatusChanged(_)
            | RoderEvent::TeamDisplayModeChanged(_)
            | RoderEvent::TeamTaskChanged(_)
            | RoderEvent::TeamCleanupCompleted(_) => None,
            RoderEvent::ProcessStarted(e) => e.process.turn_id.as_ref(),
            RoderEvent::ProcessOutput(e) => e.turn_id.as_ref(),
            RoderEvent::ProcessExited(e) => e.process.turn_id.as_ref(),
            RoderEvent::ProcessStopping(_) => None,
            RoderEvent::ProcessStopped(e) => e.process.turn_id.as_ref(),
            RoderEvent::ProcessFailed(e) => e.process.turn_id.as_ref(),
            RoderEvent::AutomationStarted(e) => e.run.turn_id.as_ref(),
            RoderEvent::AutomationCompleted(e) => e.run.turn_id.as_ref(),
            RoderEvent::AutomationFailed(e) => e.run.turn_id.as_ref(),
            RoderEvent::WorkflowRunDrafted(e) => e.turn_id.as_ref(),
            RoderEvent::WorkflowApprovalRequested(e) => e.turn_id.as_ref(),
            RoderEvent::WorkflowRunApproved(e) => e.turn_id.as_ref(),
            RoderEvent::WorkflowRunDenied(e) => e.turn_id.as_ref(),
            RoderEvent::WorkflowRunQueued(e) => e.turn_id.as_ref(),
            RoderEvent::WorkflowRunStarted(e) => e.turn_id.as_ref(),
            RoderEvent::WorkflowPhaseStarted(e) => e.turn_id.as_ref(),
            RoderEvent::WorkflowPhaseCompleted(e) => e.turn_id.as_ref(),
            RoderEvent::WorkflowAgentQueued(e) => e.turn_id.as_ref(),
            RoderEvent::WorkflowAgentStarted(e) => e.turn_id.as_ref(),
            RoderEvent::WorkflowAgentCompleted(e) => e.turn_id.as_ref(),
            RoderEvent::WorkflowAgentFailed(e) => e.turn_id.as_ref(),
            RoderEvent::WorkflowOutputRecorded(e) => e.turn_id.as_ref(),
            RoderEvent::WorkflowCheckpointRecorded(e) => e.turn_id.as_ref(),
            RoderEvent::WorkflowRunPaused(e) => e.turn_id.as_ref(),
            RoderEvent::WorkflowRunResumed(e) => e.turn_id.as_ref(),
            RoderEvent::WorkflowRunStopped(e) => e.turn_id.as_ref(),
            RoderEvent::WorkflowRunCompleted(e) => e.turn_id.as_ref(),
            RoderEvent::WorkflowRunFailed(e) => e.turn_id.as_ref(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub event_id: EventId,
    pub seq: u64,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
    pub source: EventSource,
    pub kind: String,
    pub thread_id: Option<ThreadId>,
    pub turn_id: Option<TurnId>,
    pub event: RoderEvent,
}

impl EventEnvelope {
    pub fn matches_filter(&self, filter: &EventFilter) -> bool {
        filter.matches(self)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct EventFilter {
    pub thread_id: Option<ThreadId>,
    pub turn_id: Option<TurnId>,
    pub source: Option<EventSource>,
    pub kinds: Vec<String>,
}

impl EventFilter {
    pub fn for_thread(thread_id: impl Into<ThreadId>) -> Self {
        Self {
            thread_id: Some(thread_id.into()),
            ..Self::default()
        }
    }

    pub fn for_turn(thread_id: impl Into<ThreadId>, turn_id: impl Into<TurnId>) -> Self {
        Self {
            thread_id: Some(thread_id.into()),
            turn_id: Some(turn_id.into()),
            ..Self::default()
        }
    }

    pub fn with_source(mut self, source: EventSource) -> Self {
        self.source = Some(source);
        self
    }

    pub fn with_kind(mut self, kind: impl Into<String>) -> Self {
        self.kinds.push(kind.into());
        self
    }

    pub fn matches(&self, envelope: &EventEnvelope) -> bool {
        if self
            .thread_id
            .as_ref()
            .is_some_and(|thread_id| envelope.thread_id.as_ref() != Some(thread_id))
        {
            return false;
        }
        if self
            .turn_id
            .as_ref()
            .is_some_and(|turn_id| envelope.turn_id.as_ref() != Some(turn_id))
        {
            return false;
        }
        if self
            .source
            .as_ref()
            .is_some_and(|source| &envelope.source != source)
        {
            return false;
        }
        if !self.kinds.is_empty() && !self.kinds.iter().any(|kind| kind == &envelope.kind) {
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(thread_id: Option<&str>, turn_id: Option<&str>, kind: &str) -> EventEnvelope {
        EventEnvelope {
            event_id: "event-1".to_string(),
            seq: 1,
            timestamp: OffsetDateTime::UNIX_EPOCH,
            source: EventSource::Core,
            kind: kind.to_string(),
            thread_id: thread_id.map(str::to_string),
            turn_id: turn_id.map(str::to_string),
            event: RoderEvent::TurnStarted(TurnStarted {
                thread_id: thread_id.unwrap_or("thread-a").to_string(),
                turn_id: turn_id.unwrap_or("turn-a").to_string(),
                runtime_profile: RuntimeProfile::Interactive,
                timestamp: OffsetDateTime::UNIX_EPOCH,
            }),
        }
    }

    #[test]
    fn event_filter_matches_thread_turn_source_and_kind() {
        let envelope = envelope(Some("thread-a"), Some("turn-a"), "turn.started");
        let filter = EventFilter::for_turn("thread-a", "turn-a")
            .with_source(EventSource::Core)
            .with_kind("turn.started");

        assert!(filter.matches(&envelope));
        assert!(envelope.matches_filter(&filter));
        assert!(!EventFilter::for_thread("thread-b").matches(&envelope));
        assert!(!EventFilter::for_turn("thread-a", "turn-b").matches(&envelope));
        assert!(
            !EventFilter::default()
                .with_source(EventSource::Provider)
                .matches(&envelope)
        );
        assert!(
            !EventFilter::default()
                .with_kind("turn.completed")
                .matches(&envelope)
        );
    }

    #[test]
    fn empty_event_filter_matches_everything() {
        assert!(EventFilter::default().matches(&envelope(None, None, "runtime.started")));
    }

    #[test]
    fn code_index_event_kind_and_scope_are_visible() {
        let event = RoderEvent::CodeIndexProofFilteredResultDropped(
            crate::code_index::CodeIndexProofFilteredResultDropped {
                context: crate::code_index::CodeIndexEventContext {
                    workspace_root: std::path::PathBuf::from("/repo"),
                    generation_id: Some("gen-1".to_string()),
                    thread_id: Some("thread-1".to_string()),
                    turn_id: Some("turn-1".to_string()),
                },
                drop: crate::code_index::ProofFilteredDrop {
                    query_id: "query-1".to_string(),
                    path_hash: "path-hash".to_string(),
                    content_hash: "content-hash".to_string(),
                    reason: "proof missing".to_string(),
                },
                timestamp: OffsetDateTime::UNIX_EPOCH,
            },
        );

        assert_eq!(event.kind(), "code_index.proof_filtered_result_dropped");
        assert_eq!(event.thread_id().map(String::as_str), Some("thread-1"));
        assert_eq!(event.turn_id().map(String::as_str), Some("turn-1"));
        assert_eq!(event.source(), EventSource::Core);
    }

    #[test]
    fn workflow_event_kind_scope_and_agent_metadata_are_visible() {
        let agent = crate::dynamic_workflows::WorkflowAgentRun {
            agent_id: "agent-1".to_string(),
            phase_id: "phase-1".to_string(),
            description: "Review findings".to_string(),
            status: crate::dynamic_workflows::WorkflowAgentStatus::Completed,
            lane: Some(crate::subagents::SubagentLane::Reviewer),
            model: Some("mock-model".to_string()),
            thread_id: Some("child-thread".to_string()),
            turn_id: Some("child-turn".to_string()),
            usage: Some(crate::inference::TokenUsage::new(10, 5, 15)),
            exit_reason: None,
            error: None,
            started_at: Some(OffsetDateTime::UNIX_EPOCH),
            completed_at: Some(OffsetDateTime::UNIX_EPOCH),
        };
        let event = RoderEvent::WorkflowAgentCompleted(WorkflowAgentCompleted {
            run_id: "run-1".to_string(),
            thread_id: Some("thread-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            agent,
            timestamp: OffsetDateTime::UNIX_EPOCH,
        });

        assert_eq!(event.kind(), "workflows/agentCompleted");
        assert_eq!(event.source(), EventSource::Core);
        assert_eq!(event.thread_id().map(String::as_str), Some("thread-1"));
        assert_eq!(event.turn_id().map(String::as_str), Some("turn-1"));

        let value = serde_json::to_value(&event).unwrap();
        let event_value = &value["WorkflowAgentCompleted"];
        assert_eq!(event_value["runId"], "run-1");
        assert_eq!(event_value["agent"]["phaseId"], "phase-1");
        assert_eq!(event_value["agent"]["agentId"], "agent-1");
        assert_eq!(event_value["agent"]["status"], "completed");
        assert_eq!(event_value["agent"]["usage"]["total_tokens"], 15);
    }

    #[test]
    fn event_timestamps_serialize_as_rfc3339_strings() {
        let value =
            serde_json::to_value(envelope(Some("thread-a"), Some("turn-a"), "turn.started"))
                .unwrap();

        assert_eq!(value["timestamp"], "1970-01-01T00:00:00Z");
        assert_eq!(
            value["event"]["TurnStarted"]["timestamp"],
            "1970-01-01T00:00:00Z"
        );
    }

    #[test]
    fn subagent_event_envelope_round_trips_parent_ids() {
        let event = RoderEvent::SubagentStarted(SubagentStarted {
            thread_id: "child-thread".to_string(),
            turn_id: "child-turn".to_string(),
            parent_thread_id: "parent-thread".to_string(),
            parent_turn_id: "parent-turn".to_string(),
            agent_type: "explore".to_string(),
            description: "Inspect repository".to_string(),
            model: Some("test-model".to_string()),
            timestamp: OffsetDateTime::UNIX_EPOCH,
        });
        let envelope = EventEnvelope {
            event_id: "event-subagent-started".to_string(),
            seq: 7,
            timestamp: OffsetDateTime::UNIX_EPOCH,
            source: event.source(),
            kind: event.kind().to_string(),
            thread_id: event.thread_id().cloned(),
            turn_id: event.turn_id().cloned(),
            event,
        };

        let serialized = serde_json::to_string(&envelope).unwrap();
        let round_trip: EventEnvelope = serde_json::from_str(&serialized).unwrap();

        assert_eq!(round_trip.kind, "subagent.started");
        assert_eq!(round_trip.source, EventSource::Extension);
        assert_eq!(round_trip.thread_id.as_deref(), Some("child-thread"));
        assert_eq!(round_trip.turn_id.as_deref(), Some("child-turn"));

        match round_trip.event {
            RoderEvent::SubagentStarted(started) => {
                assert_eq!(started.parent_thread_id, "parent-thread");
                assert_eq!(started.parent_turn_id, "parent-turn");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn subagent_trace_event_uses_parent_turn_for_filtering() {
        let summary = SubagentTraceSummary {
            trace_id: "trace-1".to_string(),
            parent: ParentTurnRef {
                thread_id: "parent-thread".to_string(),
                turn_id: "parent-turn".to_string(),
            },
            child_thread_id: "child-thread".to_string(),
            child_turn_id: "child-turn".to_string(),
            title: "Inspect repository".to_string(),
            role: "explorer".to_string(),
            model: Some("test-model".to_string()),
            lane: None,
            status: SubagentTraceStatus::Running,
            elapsed_ms: 10,
            usage: None,
            destination: None,
            latest_activity: None,
            error_summary: None,
            exit_reason: None,
        };
        let event = RoderEvent::SubagentTraceCreated(SubagentTraceCreated {
            summary,
            timestamp: OffsetDateTime::UNIX_EPOCH,
        });

        assert_eq!(event.kind(), "turn/subagentTraceCreated");
        assert_eq!(event.source(), EventSource::Extension);
        assert_eq!(event.thread_id().map(String::as_str), Some("parent-thread"));
        assert_eq!(event.turn_id().map(String::as_str), Some("parent-turn"));
    }

    #[test]
    fn task_events_round_trip_with_replay_ids() {
        let event = RoderEvent::TaskOutput(TaskOutput {
            task_id: "task-1".to_string(),
            stream: crate::tasks::TaskOutputStream::Stdout,
            chunk: "building\n".to_string(),
            dropped_bytes: 0,
            thread_id: Some("thread-a".to_string()),
            turn_id: Some("turn-a".to_string()),
            timestamp: OffsetDateTime::UNIX_EPOCH,
        });
        let envelope = EventEnvelope {
            event_id: "event-task-output".to_string(),
            seq: 9,
            timestamp: OffsetDateTime::UNIX_EPOCH,
            source: event.source(),
            kind: event.kind().to_string(),
            thread_id: event.thread_id().cloned(),
            turn_id: event.turn_id().cloned(),
            event,
        };

        let serialized = serde_json::to_string(&envelope).unwrap();
        let round_trip: EventEnvelope = serde_json::from_str(&serialized).unwrap();

        assert_eq!(round_trip.kind, "task.output");
        assert_eq!(round_trip.source, EventSource::Extension);
        assert_eq!(round_trip.thread_id.as_deref(), Some("thread-a"));
        assert_eq!(round_trip.turn_id.as_deref(), Some("turn-a"));
        match round_trip.event {
            RoderEvent::TaskOutput(output) => {
                assert_eq!(output.task_id, "task-1");
                assert_eq!(output.stream, crate::tasks::TaskOutputStream::Stdout);
                assert_eq!(output.chunk, "building\n");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn automations_event_exposes_kind_source_and_replay_scope() {
        let run = crate::automations::AutomationRunSummary {
            run_id: "run-1".to_string(),
            automation_id: "automation-1".to_string(),
            occurrence_key: "automation-1:1970-01-01T00:00:00Z".to_string(),
            state: crate::automations::AutomationRunState::Completed,
            scheduled_for: OffsetDateTime::UNIX_EPOCH,
            queued_at: Some(OffsetDateTime::UNIX_EPOCH),
            started_at: Some(OffsetDateTime::UNIX_EPOCH),
            finished_at: Some(OffsetDateTime::UNIX_EPOCH),
            thread_id: Some("thread-a".to_string()),
            turn_id: Some("turn-a".to_string()),
            task_id: Some("task-1".to_string()),
            server_id: Some("desktop-main".to_string()),
            server_role: Some("desktop".to_string()),
            exit_code: Some(0),
            error: None,
            skip_reason: None,
        };
        let event = RoderEvent::AutomationCompleted(crate::automations::AutomationCompleted {
            run,
            timestamp: OffsetDateTime::UNIX_EPOCH,
        });

        assert_eq!(event.kind(), "automations/completed");
        assert_eq!(event.source(), EventSource::Core);
        assert_eq!(event.thread_id().map(String::as_str), Some("thread-a"));
        assert_eq!(event.turn_id().map(String::as_str), Some("turn-a"));

        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(
            value["AutomationCompleted"]["run"]["occurrenceKey"],
            "automation-1:1970-01-01T00:00:00Z"
        );
        assert_eq!(
            value["AutomationCompleted"]["run"]["serverId"],
            "desktop-main"
        );
    }

    #[test]
    fn tool_call_completed_round_trips_error_status() {
        let event = RoderEvent::ToolCallCompleted(ToolCallCompleted {
            thread_id: "thread-a".to_string(),
            turn_id: "turn-a".to_string(),
            tool_id: "tool-a".to_string(),
            tool_name: Some("list_files".to_string()),
            display_payload: Some(serde_json::json!({ "path": "." })),
            is_error: true,
            output: Some("tool failed".to_string()),
            timestamp: OffsetDateTime::UNIX_EPOCH,
        });
        let envelope = EventEnvelope {
            event_id: "event-tool-completed".to_string(),
            seq: 10,
            timestamp: OffsetDateTime::UNIX_EPOCH,
            source: event.source(),
            kind: event.kind().to_string(),
            thread_id: event.thread_id().cloned(),
            turn_id: event.turn_id().cloned(),
            event,
        };

        let serialized = serde_json::to_string(&envelope).unwrap();
        let round_trip: EventEnvelope = serde_json::from_str(&serialized).unwrap();

        assert_eq!(round_trip.kind, "tool.call_completed");
        match round_trip.event {
            RoderEvent::ToolCallCompleted(completed) => {
                assert_eq!(completed.tool_id, "tool-a");
                assert!(completed.is_error);
                assert_eq!(completed.output.as_deref(), Some("tool failed"));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn file_change_preview_event_round_trips_public_metadata() {
        let event = RoderEvent::FileChangePreviewReady(FileChangePreviewReady {
            thread_id: "thread-a".to_string(),
            turn_id: "turn-a".to_string(),
            tool_id: "tool-a".to_string(),
            tool_name: "edit".to_string(),
            path: "src/lib.rs".to_string(),
            change_type: "modify".to_string(),
            before: Some("old\n".to_string()),
            after: "new\n".to_string(),
            supports_partial: false,
            timestamp: OffsetDateTime::UNIX_EPOCH,
        });
        let envelope = EventEnvelope {
            event_id: "event-file-preview".to_string(),
            seq: 8,
            timestamp: OffsetDateTime::UNIX_EPOCH,
            source: event.source(),
            kind: event.kind().to_string(),
            thread_id: event.thread_id().cloned(),
            turn_id: event.turn_id().cloned(),
            event,
        };

        let serialized = serde_json::to_string(&envelope).unwrap();
        let round_trip: EventEnvelope = serde_json::from_str(&serialized).unwrap();

        assert_eq!(round_trip.kind, "file.change_preview_ready");
        assert_eq!(round_trip.source, EventSource::Tool);
        assert_eq!(round_trip.thread_id.as_deref(), Some("thread-a"));
        assert_eq!(round_trip.turn_id.as_deref(), Some("turn-a"));
        match round_trip.event {
            RoderEvent::FileChangePreviewReady(preview) => {
                assert_eq!(preview.tool_id, "tool-a");
                assert_eq!(preview.tool_name, "edit");
                assert_eq!(preview.path, "src/lib.rs");
                assert_eq!(preview.change_type, "modify");
                assert_eq!(preview.before.as_deref(), Some("old\n"));
                assert_eq!(preview.after, "new\n");
                assert!(!preview.supports_partial);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn inference_started_deserializes_older_records_without_model_fields() {
        let value = serde_json::json!({
            "InferenceStarted": {
                "thread_id": "thread-a",
                "turn_id": "turn-a",
                "engine_id": "mock",
                "timestamp": "1970-01-01T00:00:00Z"
            }
        });

        let event: RoderEvent = serde_json::from_value(value).unwrap();

        match event {
            RoderEvent::InferenceStarted(started) => {
                assert_eq!(started.model.provider, "");
                assert_eq!(started.model.model, "");
                assert_eq!(started.reasoning, ReasoningConfig::default());
            }
            other => panic!("expected inference started, got {other:?}"),
        }
    }

    #[test]
    fn processes_events_expose_kind_source_scope_and_round_trip() {
        let descriptor = crate::processes::ProcessDescriptor {
            process_id: "process-1".to_string(),
            origin: crate::processes::ProcessOrigin::CommandExec,
            state: crate::processes::ProcessState::Running,
            command: vec!["sleep".to_string(), "10".to_string()],
            command_summary: "sleep 10".to_string(),
            cwd: Some("/repo".to_string()),
            pid: Some(1234),
            task_id: Some("task-1".to_string()),
            thread_id: Some("thread-a".to_string()),
            turn_id: Some("turn-a".to_string()),
            runner_destination_id: None,
            runner_session_id: None,
            stoppable: true,
            started_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
            stdout_tail: None,
            stderr_tail: None,
        };
        let event = RoderEvent::ProcessStarted(crate::processes::ProcessStarted {
            process: descriptor,
            timestamp: OffsetDateTime::UNIX_EPOCH,
        });
        let envelope = EventEnvelope {
            event_id: "event-process-started".to_string(),
            seq: 11,
            timestamp: OffsetDateTime::UNIX_EPOCH,
            source: event.source(),
            kind: event.kind().to_string(),
            thread_id: event.thread_id().cloned(),
            turn_id: event.turn_id().cloned(),
            event,
        };

        let serialized = serde_json::to_string(&envelope).unwrap();
        let round_trip: EventEnvelope = serde_json::from_str(&serialized).unwrap();

        assert_eq!(round_trip.kind, "process.started");
        assert_eq!(round_trip.source, EventSource::Extension);
        assert_eq!(round_trip.thread_id.as_deref(), Some("thread-a"));
        assert_eq!(round_trip.turn_id.as_deref(), Some("turn-a"));
        match round_trip.event {
            RoderEvent::ProcessStarted(started) => {
                assert_eq!(started.process.process_id, "process-1");
                assert_eq!(started.process.pid, Some(1234));
            }
            other => panic!("unexpected event: {other:?}"),
        }

        let output = RoderEvent::ProcessOutput(crate::processes::ProcessOutput {
            process_id: "process-1".to_string(),
            stream: crate::tasks::TaskOutputStream::Stdout,
            chunk: "ready\n".to_string(),
            dropped_bytes: 0,
            thread_id: Some("thread-a".to_string()),
            turn_id: Some("turn-a".to_string()),
            timestamp: OffsetDateTime::UNIX_EPOCH,
        });
        assert_eq!(output.kind(), "process.output");
        assert_eq!(output.thread_id().map(String::as_str), Some("thread-a"));
        assert_eq!(output.turn_id().map(String::as_str), Some("turn-a"));
    }

    #[test]
    fn skill_activation_event_exposes_kind_source_and_turn_scope() {
        let descriptor = crate::skills::SkillDescriptor {
            id: "builtin:commit".to_string(),
            name: "commit".to_string(),
            canonical_path: "roder-builtin://commit/SKILL.md".to_string(),
            source: crate::skills::SkillSource::BuiltIn,
            exposure: crate::skills::SkillExposure::DirectOnly,
            activation: crate::skills::SkillActivationState::Enabled,
            description: "Commit staged changes safely.".to_string(),
            short_description: Some("Commit safely".to_string()),
            experimental: false,
            diagnostics: Vec::new(),
            agent_metadata: None,
        };
        let event = RoderEvent::SkillActivationResolved(crate::skills::SkillActivationResolved {
            thread_id: "thread-a".to_string(),
            turn_id: "turn-a".to_string(),
            selector: crate::skills::SkillSelector::Name {
                name: "commit".to_string(),
            },
            activation_reason: crate::skills::SkillActivationReason::FeatureBinding,
            activated: true,
            descriptor: Some(descriptor),
            diagnostic: None,
            timestamp: OffsetDateTime::UNIX_EPOCH,
        });

        assert_eq!(event.kind(), "skills/activationResolved");
        assert_eq!(event.source(), EventSource::Core);
        assert_eq!(event.thread_id().map(String::as_str), Some("thread-a"));
        assert_eq!(event.turn_id().map(String::as_str), Some("turn-a"));
    }
}
