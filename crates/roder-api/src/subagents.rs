use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::events::{ThreadId, TurnId};
use crate::extension::SubagentDispatcherId;
use crate::inference::TokenUsage;
use crate::trace::SubagentTraceSink;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubagentRequest {
    pub description: String,
    pub prompt: String,
    pub subagent_type: Option<String>,
    pub model: Option<String>,
    pub tools: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lane: Option<SubagentLane>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrent: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_tools: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_deadline_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inputs: Option<serde_json::Value>,
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SubagentLane {
    Scout,
    Editor,
    Reviewer,
    Runner,
}

impl SubagentLane {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Scout => "scout",
            Self::Editor => "editor",
            Self::Reviewer => "reviewer",
            Self::Runner => "runner",
        }
    }

    pub fn preset(self) -> SubagentLanePreset {
        match self {
            Self::Scout => SubagentLanePreset {
                lane: self,
                description: "Read and search without changing state.",
                max_concurrent: 4,
                timeout_seconds: 120,
                allowed_tools: &[
                    "Read",
                    "Grep",
                    "Glob",
                    "read_file",
                    "grep",
                    "glob",
                    "list_files",
                ],
            },
            Self::Editor => SubagentLanePreset {
                lane: self,
                description: "Make a bounded file-change slice.",
                max_concurrent: 2,
                timeout_seconds: 180,
                allowed_tools: &[
                    "Read",
                    "Grep",
                    "Glob",
                    "read_file",
                    "grep",
                    "glob",
                    "list_files",
                    "write_file",
                    "edit",
                    "multi_edit",
                    "apply_patch",
                ],
            },
            Self::Reviewer => SubagentLanePreset {
                lane: self,
                description: "Review and verify with evidence.",
                max_concurrent: 2,
                timeout_seconds: 120,
                allowed_tools: &[
                    "Read",
                    "Grep",
                    "Glob",
                    "read_file",
                    "grep",
                    "glob",
                    "list_files",
                ],
            },
            Self::Runner => SubagentLanePreset {
                lane: self,
                description: "Run commands or tests when process policy allows it.",
                max_concurrent: 1,
                timeout_seconds: 120,
                allowed_tools: &["Shell", "shell", "exec_command", "run_command"],
            },
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubagentLanePreset {
    pub lane: SubagentLane,
    pub description: &'static str,
    pub max_concurrent: usize,
    pub timeout_seconds: u64,
    pub allowed_tools: &'static [&'static str],
}

pub fn built_in_subagent_lane_presets() -> [SubagentLanePreset; 4] {
    [
        SubagentLane::Scout.preset(),
        SubagentLane::Editor.preset(),
        SubagentLane::Reviewer.preset(),
        SubagentLane::Runner.preset(),
    ]
}

pub const SUBAGENT_SUMMARY_CONTRACT: &str = "Child summary must include these labels: Conclusion, Evidence, Files inspected, Files changed, Remaining uncertainty.";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubagentDefinition {
    pub agent_type: String,
    pub description: String,
    pub tools: Vec<String>,
    pub model: Option<String>,
    pub system_prompt: Option<String>,
    pub permission_mode: SubagentPermissionMode,
    pub max_turns: Option<u32>,
    pub max_result_chars: Option<usize>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubagentPermissionMode {
    ReadOnly,
    #[default]
    Default,
    AutoEdit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubagentResult {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub agent_type: String,
    pub model: Option<String>,
    pub final_message: String,
    pub usage: Option<TokenUsage>,
    pub exit_reason: SubagentExitReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript: Option<serde_json::Value>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubagentExitReason {
    Completed,
    MaxTurns,
    Timeout,
    Cancelled,
    Failed,
}

#[async_trait::async_trait]
pub trait SubagentDispatcher: Send + Sync + 'static {
    fn id(&self) -> SubagentDispatcherId;

    fn definitions(&self) -> Vec<SubagentDefinition>;

    async fn dispatch(
        &self,
        parent_thread_id: ThreadId,
        parent_turn_id: TurnId,
        request: SubagentRequest,
    ) -> anyhow::Result<SubagentResult>;

    async fn dispatch_traced(
        &self,
        parent_thread_id: ThreadId,
        parent_turn_id: TurnId,
        request: SubagentRequest,
        trace_sink: Option<std::sync::Arc<dyn SubagentTraceSink>>,
    ) -> anyhow::Result<SubagentResult> {
        let _ = trace_sink;
        self.dispatch(parent_thread_id, parent_turn_id, request)
            .await
    }

    /// Dispatch a child carrying the parent tool call's execution handles
    /// (workspace, remote workspace, process runner, context artifacts). This
    /// is how a subagent or swarm child operates on the same workspace as the
    /// lead thread instead of failing with "workspace handle is not available".
    ///
    /// The default implementation ignores the handles and falls back to
    /// [`Self::dispatch_traced`], so non-workspace dispatchers (e.g. remote or
    /// fake ones) keep working unchanged.
    async fn dispatch_with_context(
        &self,
        parent_thread_id: ThreadId,
        parent_turn_id: TurnId,
        request: SubagentRequest,
        trace_sink: Option<std::sync::Arc<dyn SubagentTraceSink>>,
        handles: crate::tools::ToolExecutionHandles,
    ) -> anyhow::Result<SubagentResult> {
        let _ = handles;
        self.dispatch_traced(parent_thread_id, parent_turn_id, request, trace_sink)
            .await
    }
}

// ---------------------------------------------------------------------------
// Agent-swarm mode (roadmap phase 104)
//
// Swarm is a Roder-native composition over the canonical subagent dispatch
// surface above. It lets a lead model (or `/agent-swarm` command) launch many
// homogeneous child tasks from one prompt template, resume unfinished children,
// and collect an ordered, machine-readable result. The types here are
// provider-neutral and do not vendor any external implementation.
// ---------------------------------------------------------------------------

/// Canonical model-facing swarm tool name. Single source of truth shared by
/// the tool registration (`roder-ext-subagents`) and the core turn loop's
/// exclusivity enforcement (`roder-core`).
pub const AGENT_SWARM_TOOL_NAME: &str = "agent_swarm";

/// Literal placeholder replaced with each `agent_swarm` item value.
pub const AGENT_SWARM_PROMPT_PLACEHOLDER: &str = "{{item}}";

/// Default upper bound on swarm child count. Config may lower but not exceed it.
pub const AGENT_SWARM_MAX_SUBAGENTS: usize = 128;
/// Default number of children that may start immediately before pacing applies.
pub const AGENT_SWARM_INITIAL_LAUNCH_LIMIT: usize = 5;
/// Default pacing interval between additional child launches, in milliseconds.
pub const AGENT_SWARM_LAUNCH_INTERVAL_MS: u64 = 700;
/// Default number of times a child is retried after a provider rate limit
/// before it is reported as failed.
pub const AGENT_SWARM_RATE_LIMIT_MAX_RETRIES: usize = 4;
/// Default base backoff between rate-limit retries, in milliseconds (doubled
/// each subsequent attempt: 3s, 6s, 12s, ...).
pub const AGENT_SWARM_RATE_LIMIT_BASE_BACKOFF_MS: u64 = 3_000;
/// Hard cap on rate-limit retries so config/env can never make a swarm wait
/// unboundedly.
pub const AGENT_SWARM_RATE_LIMIT_MAX_RETRIES_CAP: usize = 8;
/// Default minimum interval between successive global rate-limit capacity
/// shrinks, in milliseconds. While the provider keeps rate-limiting, the swarm
/// shrinks its global capacity by one at most this often, so it throttles
/// smoothly instead of collapsing on the first burst of 429s.
pub const AGENT_SWARM_RATE_LIMIT_SHRINK_INTERVAL_MS: u64 = 2_000;
/// Default quiet window after which the swarm recovers one unit of global
/// rate-limit capacity, in milliseconds (3 minutes). Any fresh rate limit
/// restarts the window, and recovery happens at most once per quiet window.
pub const AGENT_SWARM_RATE_LIMIT_RECOVERY_INTERVAL_MS: u64 = 180_000;

fn default_rate_limit_max_retries() -> usize {
    AGENT_SWARM_RATE_LIMIT_MAX_RETRIES
}

fn default_rate_limit_base_backoff_ms() -> u64 {
    AGENT_SWARM_RATE_LIMIT_BASE_BACKOFF_MS
}

fn default_rate_limit_shrink_interval_ms() -> u64 {
    AGENT_SWARM_RATE_LIMIT_SHRINK_INTERVAL_MS
}

fn default_rate_limit_recovery_interval_ms() -> u64 {
    AGENT_SWARM_RATE_LIMIT_RECOVERY_INTERVAL_MS
}

/// Canonical swarm-mode reminder injected into a turn's developer instructions
/// (server-side) while agent-swarm mode is active, so the model reaches for the
/// `agent_swarm` fanout tool. Shared by the runtime injection and the TUI label
/// so the wording stays in one place.
pub const AGENT_SWARM_MODE_REMINDER: &str = "Agent-swarm mode is active. When the task splits into \
several similarly-shaped subtasks over different inputs, call the agent_swarm tool exactly once \
with a prompt_template containing {{item}} and an items array (or resume_agent_ids), and make it \
the only tool call in that response. agent_swarm dispatches configured roles only: its \
subagent_type must exactly match a role advertised in the tool schema, and lane names such as \
scout are not role IDs. Do not rely on a lane to add missing tools; for generic repository work \
use spawn_agent when available.";

/// Emitted when agent-swarm mode is toggled on a runtime/thread.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentSwarmModeChanged {
    pub thread_id: ThreadId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    pub enabled: bool,
    pub trigger: AgentSwarmModeTrigger,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: time::OffsetDateTime,
}

/// Emitted when the `agent_swarm` tool begins fanning out children, so any
/// app-server/SDK/TUI client can observe the swarm as a whole. Per-child
/// progress flows through the existing `Subagent*` trace events.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentSwarmStarted {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub tool_id: String,
    /// Children the swarm will launch (item-based spawns plus resumes).
    pub child_count: usize,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: time::OffsetDateTime,
}

/// Emitted when the `agent_swarm` tool finishes, carrying the aggregate child
/// outcome counts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentSwarmCompleted {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub tool_id: String,
    pub completed: usize,
    pub failed: usize,
    pub aborted: usize,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: time::OffsetDateTime,
}

/// A running tally of swarm children as they resolve, used for live progress.
/// `resolved` is `completed + failed + aborted`; clients can render
/// `resolved/total` as a progress bar without tracking individual children.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentSwarmProgressSnapshot {
    pub total: usize,
    pub completed: usize,
    pub failed: usize,
    pub aborted: usize,
}

impl AgentSwarmProgressSnapshot {
    /// Children that have finished (in any outcome).
    pub fn resolved(&self) -> usize {
        self.completed + self.failed + self.aborted
    }
}

/// Emitted each time a swarm child resolves, so a client can render a live
/// "N/total done" progress tick between `AgentSwarmStarted` and
/// `AgentSwarmCompleted` rather than only the final result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentSwarmProgress {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub tool_id: String,
    pub snapshot: AgentSwarmProgressSnapshot,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: time::OffsetDateTime,
}

/// Sink the runtime supplies on a tool-execution context so the `agent_swarm`
/// tool can publish live progress snapshots (carrying the runtime's thread/turn
/// ids and the bus emitter) without the tool depending on `roder-core`.
#[async_trait::async_trait]
pub trait AgentSwarmProgressSink: Send + Sync {
    async fn emit_progress(
        &self,
        thread_id: &str,
        turn_id: &str,
        tool_id: &str,
        snapshot: AgentSwarmProgressSnapshot,
    );
}

/// Why a batch of tool calls violates the `agent_swarm` exclusivity rule:
/// `agent_swarm` must be the only tool call in a single model response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentSwarmBatchViolation {
    /// More than one `agent_swarm` call appeared in the same response.
    MultipleSwarms {
        /// Whether non-swarm tool calls were also present in the batch.
        has_other_tools: bool,
    },
    /// Exactly one `agent_swarm` call was mixed with other tool calls.
    MixedWithOtherTools,
}

impl AgentSwarmBatchViolation {
    /// Actionable retry text returned to the model for every call in the
    /// denied batch, so it re-issues `agent_swarm` by itself.
    pub fn deny_message(self) -> String {
        match self {
            Self::MultipleSwarms { has_other_tools } => {
                let mut message = String::from(
                    "agent_swarm must be called one swarm at a time. Multiple agent_swarm calls \
                     are not forbidden, but issue them sequentially: call one agent_swarm, wait \
                     for its result, then call the next; or merge the work into a single \
                     agent_swarm when one swarm can cover it.",
                );
                if has_other_tools {
                    message.push_str(
                        " agent_swarm also must not be combined with other tools in the same \
                         response.",
                    );
                }
                message
            }
            Self::MixedWithOtherTools => String::from(
                "agent_swarm must be the only tool call in a model response. Retry with a single \
                 agent_swarm call by itself, then call any other tools after it returns.",
            ),
        }
    }
}

/// Detect whether a batch of tool-call names violates the `agent_swarm`
/// exclusivity rule. Returns `None` when the batch is valid: no `agent_swarm`
/// call, or exactly one `agent_swarm` call by itself.
pub fn agent_swarm_batch_violation<'a>(
    tool_names: impl Iterator<Item = &'a str>,
) -> Option<AgentSwarmBatchViolation> {
    let mut total = 0usize;
    let mut swarm = 0usize;
    for name in tool_names {
        total += 1;
        if name == AGENT_SWARM_TOOL_NAME {
            swarm += 1;
        }
    }
    if swarm == 0 || (swarm == 1 && total == 1) {
        return None;
    }
    if swarm > 1 {
        Some(AgentSwarmBatchViolation::MultipleSwarms {
            has_other_tools: total > swarm,
        })
    } else {
        Some(AgentSwarmBatchViolation::MixedWithOtherTools)
    }
}

/// How a swarm-mode session was entered.
///
/// `manual` is a persistent toggle (`/agent-swarm on`); `task` is a one-shot
/// `/agent-swarm <prompt>`; `tool` is implicit entry from the `agent_swarm`
/// tool call. Only `manual` stays active across turns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentSwarmModeTrigger {
    Manual,
    Task,
    Tool,
}

impl AgentSwarmModeTrigger {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Task => "task",
            Self::Tool => "tool",
        }
    }

    /// One-shot triggers auto-exit at the end of the relevant turn.
    pub fn should_auto_exit(self) -> bool {
        matches!(self, Self::Task | Self::Tool)
    }
}

/// Whether a swarm child is a fresh spawn or a resume of an existing agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentSwarmChildKind {
    Spawn,
    Resume,
}

impl AgentSwarmChildKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Spawn => "spawn",
            Self::Resume => "resume",
        }
    }
}

/// Final outcome of a single swarm child.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentSwarmChildOutcome {
    Completed,
    Failed,
    Aborted,
}

impl AgentSwarmChildOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Aborted => "aborted",
        }
    }
}

/// Whether an aborted child had started running before cancellation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentSwarmChildState {
    Started,
    NotStarted,
}

impl AgentSwarmChildState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::NotStarted => "not_started",
        }
    }
}

/// Parsed `agent_swarm` tool input, before any child is dispatched.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AgentSwarmRequest {
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_template: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<String>,
    /// Map of existing subagent agent_id to the prompt used to resume it.
    /// Resumed children are dispatched before new item-based spawns.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub resume_agent_ids: BTreeMap<String, String>,
}

/// One ordered child to dispatch as part of a swarm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSwarmChildSpec {
    /// 1-based ordering index, stable across the whole swarm.
    pub index: usize,
    pub kind: AgentSwarmChildKind,
    /// The item value (for spawns) used to render the prompt, when present.
    pub item: Option<String>,
    /// The fully-rendered prompt for this child.
    pub prompt: String,
    /// For resumes, the existing agent id to continue.
    pub resume_agent_id: Option<String>,
}

/// Tunable scheduler/bounds for swarm fanout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSwarmConfig {
    /// Hard upper bound on children per swarm. Never exceeds
    /// [`AGENT_SWARM_MAX_SUBAGENTS`].
    pub max_subagents: usize,
    /// Children allowed to start immediately before pacing applies.
    pub initial_launch_limit: usize,
    /// Pacing interval between additional launches, in milliseconds.
    pub launch_interval_ms: u64,
    /// Optional cap on simultaneously-active children (normal phase).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrency: Option<usize>,
    /// Optional per-child timeout override, in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_timeout_seconds: Option<u64>,
    /// Times a child is retried after a provider rate limit before failing.
    #[serde(default = "default_rate_limit_max_retries")]
    pub rate_limit_max_retries: usize,
    /// Base backoff between rate-limit retries, in milliseconds (doubled each
    /// subsequent attempt).
    #[serde(default = "default_rate_limit_base_backoff_ms")]
    pub rate_limit_base_backoff_ms: u64,
    /// Minimum interval between successive global rate-limit capacity shrinks,
    /// in milliseconds. Sustained rate limits shrink global capacity by one at
    /// most this often.
    #[serde(default = "default_rate_limit_shrink_interval_ms")]
    pub rate_limit_shrink_interval_ms: u64,
    /// Quiet window with no rate limit after which the swarm recovers one unit
    /// of global rate-limit capacity, in milliseconds. A fresh rate limit
    /// restarts the window.
    #[serde(default = "default_rate_limit_recovery_interval_ms")]
    pub rate_limit_recovery_interval_ms: u64,
}

impl Default for AgentSwarmConfig {
    fn default() -> Self {
        Self {
            max_subagents: AGENT_SWARM_MAX_SUBAGENTS,
            initial_launch_limit: AGENT_SWARM_INITIAL_LAUNCH_LIMIT,
            launch_interval_ms: AGENT_SWARM_LAUNCH_INTERVAL_MS,
            max_concurrency: None,
            child_timeout_seconds: None,
            rate_limit_max_retries: AGENT_SWARM_RATE_LIMIT_MAX_RETRIES,
            rate_limit_base_backoff_ms: AGENT_SWARM_RATE_LIMIT_BASE_BACKOFF_MS,
            rate_limit_shrink_interval_ms: AGENT_SWARM_RATE_LIMIT_SHRINK_INTERVAL_MS,
            rate_limit_recovery_interval_ms: AGENT_SWARM_RATE_LIMIT_RECOVERY_INTERVAL_MS,
        }
    }
}

impl AgentSwarmConfig {
    /// Clamp config into a bounded, deterministic range so config (or env) can
    /// never request unbounded fanout, a zero-sized ramp, or an unbounded
    /// rate-limit wait.
    pub fn clamped(mut self) -> Self {
        self.max_subagents = self.max_subagents.clamp(1, AGENT_SWARM_MAX_SUBAGENTS);
        self.initial_launch_limit = self.initial_launch_limit.max(1);
        if let Some(cap) = self.max_concurrency {
            self.max_concurrency = Some(cap.max(1));
        }
        if let Some(timeout) = self.child_timeout_seconds {
            self.child_timeout_seconds = Some(timeout.max(1));
        }
        self.rate_limit_max_retries = self
            .rate_limit_max_retries
            .min(AGENT_SWARM_RATE_LIMIT_MAX_RETRIES_CAP);
        self
    }
}

/// Outcome of a single resolved swarm child entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentSwarmChildResult {
    pub index: usize,
    pub kind: AgentSwarmChildKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub outcome: AgentSwarmChildOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<AgentSwarmChildState>,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
}

/// Aggregated, ordered swarm result returned to the lead model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentSwarmResult {
    pub completed: usize,
    pub failed: usize,
    pub aborted: usize,
    pub children: Vec<AgentSwarmChildResult>,
}

impl AgentSwarmResult {
    pub fn from_children(children: Vec<AgentSwarmChildResult>) -> Self {
        let mut completed = 0;
        let mut failed = 0;
        let mut aborted = 0;
        for child in &children {
            match child.outcome {
                AgentSwarmChildOutcome::Completed => completed += 1,
                AgentSwarmChildOutcome::Failed => failed += 1,
                AgentSwarmChildOutcome::Aborted => aborted += 1,
            }
        }
        Self {
            completed,
            failed,
            aborted,
            children,
        }
    }

    /// `completed: 2, failed: 1` style summary; omits zero buckets.
    pub fn summary_line(&self) -> String {
        let mut parts = Vec::new();
        if self.completed > 0 {
            parts.push(format!("completed: {}", self.completed));
        }
        if self.failed > 0 {
            parts.push(format!("failed: {}", self.failed));
        }
        if self.aborted > 0 {
            parts.push(format!("aborted: {}", self.aborted));
        }
        if parts.is_empty() {
            "completed: 0".to_string()
        } else {
            parts.join(", ")
        }
    }

    /// A resume hint is useful when unfinished children carry agent ids.
    pub fn needs_resume_hint(&self) -> bool {
        self.children.iter().any(|child| {
            child.outcome != AgentSwarmChildOutcome::Completed && child.agent_id.is_some()
        })
    }

    /// Render the durable, transcript-safe `<agent_swarm_result>` text block.
    pub fn render_text(&self) -> String {
        let mut lines = vec![
            "<agent_swarm_result>".to_string(),
            format!("<summary>{}</summary>", self.summary_line()),
        ];
        if self.needs_resume_hint() {
            lines.push(
                "<resume_hint>Call agent_swarm with resume_agent_ids using the agent_id values in this result to continue unfinished work.</resume_hint>"
                    .to_string(),
            );
        }
        for child in &self.children {
            let mode = if child.kind == AgentSwarmChildKind::Resume {
                " mode=\"resume\"".to_string()
            } else {
                String::new()
            };
            let agent_id = child
                .agent_id
                .as_deref()
                .map(|id| format!(" agent_id=\"{}\"", escape_xml_attr(id)))
                .unwrap_or_default();
            let item = child
                .item
                .as_deref()
                .map(|item| format!(" item=\"{}\"", escape_xml_attr(item)))
                .unwrap_or_default();
            let state = child
                .state
                .map(|state| format!(" state=\"{}\"", state.as_str()))
                .unwrap_or_default();
            lines.push(format!(
                "<subagent{mode}{agent_id}{item}{state} outcome=\"{}\">{}</subagent>",
                child.outcome.as_str(),
                escape_xml_text(&child.body)
            ));
        }
        lines.push("</agent_swarm_result>".to_string());
        lines.join("\n")
    }
}

fn escape_xml_attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_xml_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Validate an [`AgentSwarmRequest`] and build the ordered child specs.
///
/// Mirrors the documented contract: at least two `items` unless
/// `resume_agent_ids` is present; `prompt_template` is required whenever
/// `items` are present and must contain the `{{item}}` placeholder; filled
/// prompts must be distinct; resumed children are ordered before spawns; and
/// the total child count must not exceed `config.max_subagents`. No child is
/// dispatched if validation fails.
pub fn build_agent_swarm_specs(
    request: &AgentSwarmRequest,
    config: &AgentSwarmConfig,
) -> Result<Vec<AgentSwarmChildSpec>, AgentSwarmValidationError> {
    if request.description.trim().is_empty() {
        return Err(AgentSwarmValidationError::EmptyDescription);
    }

    let items: Vec<String> = request
        .items
        .iter()
        .map(|item| item.trim().to_string())
        .collect();
    if items.iter().any(|item| item.is_empty()) {
        return Err(AgentSwarmValidationError::EmptyItem);
    }

    let resume_entries: Vec<(String, String)> = request
        .resume_agent_ids
        .iter()
        .map(|(id, prompt)| (id.trim().to_string(), prompt.trim().to_string()))
        .collect();
    if resume_entries
        .iter()
        .any(|(id, prompt)| id.is_empty() || prompt.is_empty())
    {
        return Err(AgentSwarmValidationError::EmptyResumeEntry);
    }

    let item_count = items.len();
    let resume_count = resume_entries.len();
    let total = item_count + resume_count;

    if resume_count == 0 && item_count < 2 {
        return Err(AgentSwarmValidationError::TooFewItems);
    }
    let max = config.max_subagents.clamp(1, AGENT_SWARM_MAX_SUBAGENTS);
    if total > max {
        return Err(AgentSwarmValidationError::TooManySubagents { total, max });
    }

    let prompt_template = request
        .prompt_template
        .as_ref()
        .map(|template| template.trim().to_string())
        .filter(|template| !template.is_empty());

    if item_count > 0 {
        let Some(template) = prompt_template.as_ref() else {
            return Err(AgentSwarmValidationError::MissingPromptTemplate);
        };
        if !template.contains(AGENT_SWARM_PROMPT_PLACEHOLDER) {
            return Err(AgentSwarmValidationError::MissingPlaceholder);
        }
    }

    let mut specs = Vec::with_capacity(total);
    for (agent_id, prompt) in &resume_entries {
        specs.push(AgentSwarmChildSpec {
            index: specs.len() + 1,
            kind: AgentSwarmChildKind::Resume,
            item: None,
            prompt: prompt.clone(),
            resume_agent_id: Some(agent_id.clone()),
        });
    }

    if item_count > 0 {
        let template = prompt_template.expect("prompt template validated above");
        let mut seen: BTreeMap<String, usize> = BTreeMap::new();
        for (offset, item) in items.iter().enumerate() {
            let prompt = template.replace(AGENT_SWARM_PROMPT_PLACEHOLDER, item);
            if let Some(previous) = seen.get(&prompt) {
                return Err(AgentSwarmValidationError::DuplicatePrompt {
                    first: *previous,
                    second: offset + 1,
                });
            }
            seen.insert(prompt.clone(), offset + 1);
            specs.push(AgentSwarmChildSpec {
                index: specs.len() + 1,
                kind: AgentSwarmChildKind::Spawn,
                item: Some(item.clone()),
                prompt,
                resume_agent_id: None,
            });
        }
    }

    Ok(specs)
}

/// Reasons an `agent_swarm` request is rejected before any child starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentSwarmValidationError {
    EmptyDescription,
    EmptyItem,
    EmptyResumeEntry,
    TooFewItems,
    TooManySubagents { total: usize, max: usize },
    MissingPromptTemplate,
    MissingPlaceholder,
    DuplicatePrompt { first: usize, second: usize },
}

impl std::fmt::Display for AgentSwarmValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyDescription => write!(f, "description must not be empty"),
            Self::EmptyItem => write!(f, "items must not contain empty values"),
            Self::EmptyResumeEntry => {
                write!(
                    f,
                    "resume_agent_ids entries must have non-empty ids and prompts"
                )
            }
            Self::TooFewItems => write!(
                f,
                "agent_swarm requires at least 2 items unless resume_agent_ids is provided"
            ),
            Self::TooManySubagents { total, max } => {
                write!(
                    f,
                    "agent_swarm supports at most {max} subagents (got {total})"
                )
            }
            Self::MissingPromptTemplate => {
                write!(f, "prompt_template is required when items are provided")
            }
            Self::MissingPlaceholder => write!(
                f,
                "prompt_template must include the {AGENT_SWARM_PROMPT_PLACEHOLDER} placeholder"
            ),
            Self::DuplicatePrompt { first, second } => write!(
                f,
                "duplicate subagent prompts from items {first} and {second}; agent_swarm requires distinct subagents"
            ),
        }
    }
}

impl std::error::Error for AgentSwarmValidationError {}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    struct NoopDispatcher;

    #[async_trait::async_trait]
    impl SubagentDispatcher for NoopDispatcher {
        fn id(&self) -> SubagentDispatcherId {
            "noop".to_string()
        }

        fn definitions(&self) -> Vec<SubagentDefinition> {
            vec![SubagentDefinition {
                agent_type: "explore".to_string(),
                description: "Explore the workspace".to_string(),
                tools: vec!["Read".to_string()],
                model: Some("test-model".to_string()),
                system_prompt: Some("Report findings only".to_string()),
                permission_mode: SubagentPermissionMode::ReadOnly,
                max_turns: Some(4),
                max_result_chars: Some(4000),
            }]
        }

        async fn dispatch(
            &self,
            _parent_thread_id: ThreadId,
            _parent_turn_id: TurnId,
            request: SubagentRequest,
        ) -> anyhow::Result<SubagentResult> {
            Ok(SubagentResult {
                thread_id: "child-thread".to_string(),
                turn_id: "child-turn".to_string(),
                agent_type: request
                    .subagent_type
                    .unwrap_or_else(|| "explore".to_string()),
                model: request.model,
                final_message: "done".to_string(),
                usage: None,
                exit_reason: SubagentExitReason::Completed,
                transcript: None,
                metadata: serde_json::json!({}),
            })
        }
    }

    #[tokio::test]
    async fn subagent_dispatcher_trait_is_object_safe() {
        let dispatcher: Arc<dyn SubagentDispatcher> = Arc::new(NoopDispatcher);

        assert_eq!(dispatcher.id(), "noop");
        assert_eq!(dispatcher.definitions()[0].agent_type, "explore");

        let result = dispatcher
            .dispatch(
                "parent-thread".to_string(),
                "parent-turn".to_string(),
                SubagentRequest {
                    description: "Check files".to_string(),
                    prompt: "Find the API entrypoint".to_string(),
                    subagent_type: Some("explore".to_string()),
                    model: Some("test-model".to_string()),
                    tools: Some(vec!["Read".to_string()]),
                    lane: None,
                    max_concurrent: None,
                    allowed_tools: None,
                    parent_deadline_seconds: None,
                    inputs: None,
                    timeout_seconds: Some(10),
                },
            )
            .await
            .unwrap();

        assert_eq!(result.thread_id, "child-thread");
        assert_eq!(result.exit_reason, SubagentExitReason::Completed);
    }

    fn swarm_request(items: &[&str]) -> AgentSwarmRequest {
        AgentSwarmRequest {
            description: "inspect files".to_string(),
            subagent_type: Some("explore".to_string()),
            prompt_template: Some("Read {{item}} and report.".to_string()),
            items: items.iter().map(|item| item.to_string()).collect(),
            resume_agent_ids: BTreeMap::new(),
        }
    }

    #[test]
    fn agent_swarm_config_clamps_into_bounds() {
        let clamped = AgentSwarmConfig {
            max_subagents: 9001,
            initial_launch_limit: 0,
            launch_interval_ms: 700,
            max_concurrency: Some(0),
            child_timeout_seconds: Some(0),
            rate_limit_max_retries: 9001,
            ..AgentSwarmConfig::default()
        }
        .clamped();
        assert_eq!(clamped.max_subagents, AGENT_SWARM_MAX_SUBAGENTS);
        assert_eq!(clamped.initial_launch_limit, 1);
        assert_eq!(clamped.max_concurrency, Some(1));
        assert_eq!(clamped.child_timeout_seconds, Some(1));
        assert_eq!(
            clamped.rate_limit_max_retries,
            AGENT_SWARM_RATE_LIMIT_MAX_RETRIES_CAP
        );
    }

    #[test]
    fn build_specs_expands_items_in_order() {
        let specs = build_agent_swarm_specs(
            &swarm_request(&["a.rs", "b.rs"]),
            &AgentSwarmConfig::default(),
        )
        .unwrap();
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].index, 1);
        assert_eq!(specs[0].kind, AgentSwarmChildKind::Spawn);
        assert_eq!(specs[0].prompt, "Read a.rs and report.");
        assert_eq!(specs[1].prompt, "Read b.rs and report.");
    }

    #[test]
    fn build_specs_orders_resumes_before_spawns() {
        let mut request = swarm_request(&["a.rs"]);
        request
            .resume_agent_ids
            .insert("agent-9".to_string(), "continue".to_string());
        let specs = build_agent_swarm_specs(&request, &AgentSwarmConfig::default()).unwrap();
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].kind, AgentSwarmChildKind::Resume);
        assert_eq!(specs[0].resume_agent_id.as_deref(), Some("agent-9"));
        assert_eq!(specs[1].kind, AgentSwarmChildKind::Spawn);
        assert_eq!(specs[1].index, 2);
    }

    #[test]
    fn build_specs_rejects_single_item_without_resume() {
        let err =
            build_agent_swarm_specs(&swarm_request(&["only.rs"]), &AgentSwarmConfig::default())
                .unwrap_err();
        assert_eq!(err, AgentSwarmValidationError::TooFewItems);
    }

    #[test]
    fn build_specs_rejects_missing_placeholder() {
        let mut request = swarm_request(&["a.rs", "b.rs"]);
        request.prompt_template = Some("no placeholder here".to_string());
        let err = build_agent_swarm_specs(&request, &AgentSwarmConfig::default()).unwrap_err();
        assert_eq!(err, AgentSwarmValidationError::MissingPlaceholder);
    }

    #[test]
    fn build_specs_rejects_duplicate_prompts() {
        let request = swarm_request(&["dup", "dup"]);
        let err = build_agent_swarm_specs(&request, &AgentSwarmConfig::default()).unwrap_err();
        assert!(matches!(
            err,
            AgentSwarmValidationError::DuplicatePrompt { .. }
        ));
    }

    #[test]
    fn build_specs_enforces_max_subagents() {
        let config = AgentSwarmConfig {
            max_subagents: 2,
            ..AgentSwarmConfig::default()
        };
        let err = build_agent_swarm_specs(&swarm_request(&["a", "b", "c"]), &config).unwrap_err();
        assert_eq!(
            err,
            AgentSwarmValidationError::TooManySubagents { total: 3, max: 2 }
        );
    }

    #[test]
    fn agent_swarm_result_renders_summary_and_resume_hint() {
        let result = AgentSwarmResult::from_children(vec![
            AgentSwarmChildResult {
                index: 1,
                kind: AgentSwarmChildKind::Spawn,
                item: Some("a.rs".to_string()),
                agent_id: Some("agent-1".to_string()),
                outcome: AgentSwarmChildOutcome::Completed,
                state: None,
                body: "ok".to_string(),
                usage: None,
            },
            AgentSwarmChildResult {
                index: 2,
                kind: AgentSwarmChildKind::Spawn,
                item: Some("b & c.rs".to_string()),
                agent_id: Some("agent-2".to_string()),
                outcome: AgentSwarmChildOutcome::Failed,
                state: Some(AgentSwarmChildState::Started),
                body: "boom <fatal>".to_string(),
                usage: None,
            },
        ]);
        assert_eq!(result.completed, 1);
        assert_eq!(result.failed, 1);
        assert!(result.needs_resume_hint());
        let text = result.render_text();
        assert!(text.contains("<summary>completed: 1, failed: 1</summary>"));
        assert!(text.contains("<resume_hint>"));
        assert!(text.contains("item=\"b &amp; c.rs\""));
        assert!(text.contains("boom &lt;fatal&gt;"));
        assert!(text.contains("outcome=\"failed\""));
    }

    #[test]
    fn agent_swarm_dtos_round_trip_json() {
        let result = AgentSwarmResult::from_children(vec![AgentSwarmChildResult {
            index: 1,
            kind: AgentSwarmChildKind::Resume,
            item: None,
            agent_id: Some("agent-1".to_string()),
            outcome: AgentSwarmChildOutcome::Aborted,
            state: Some(AgentSwarmChildState::NotStarted),
            body: "cancelled".to_string(),
            usage: None,
        }]);
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["aborted"], 1);
        assert_eq!(json["children"][0]["kind"], "resume");
        assert_eq!(json["children"][0]["outcome"], "aborted");
        assert_eq!(json["children"][0]["state"], "not_started");
        let round: AgentSwarmResult = serde_json::from_value(json).unwrap();
        assert_eq!(round, result);
    }

    #[test]
    fn agent_swarm_trigger_auto_exit_rules() {
        assert!(!AgentSwarmModeTrigger::Manual.should_auto_exit());
        assert!(AgentSwarmModeTrigger::Task.should_auto_exit());
        assert!(AgentSwarmModeTrigger::Tool.should_auto_exit());
    }

    #[test]
    fn batch_violation_allows_single_swarm_alone() {
        assert_eq!(
            agent_swarm_batch_violation(["agent_swarm"].into_iter()),
            None
        );
    }

    #[test]
    fn batch_violation_allows_batches_without_swarm() {
        assert_eq!(
            agent_swarm_batch_violation(["read_file", "write_file"].into_iter()),
            None
        );
    }

    #[test]
    fn batch_violation_flags_swarm_mixed_with_other_tools() {
        assert_eq!(
            agent_swarm_batch_violation(["agent_swarm", "read_file"].into_iter()),
            Some(AgentSwarmBatchViolation::MixedWithOtherTools)
        );
        let message = AgentSwarmBatchViolation::MixedWithOtherTools.deny_message();
        assert!(message.contains("only tool call"));
    }

    #[test]
    fn batch_violation_flags_multiple_swarms() {
        assert_eq!(
            agent_swarm_batch_violation(["agent_swarm", "agent_swarm"].into_iter()),
            Some(AgentSwarmBatchViolation::MultipleSwarms {
                has_other_tools: false
            })
        );
        assert_eq!(
            agent_swarm_batch_violation(["agent_swarm", "agent_swarm", "read_file"].into_iter()),
            Some(AgentSwarmBatchViolation::MultipleSwarms {
                has_other_tools: true
            })
        );
        let message = AgentSwarmBatchViolation::MultipleSwarms {
            has_other_tools: true,
        }
        .deny_message();
        assert!(message.contains("one swarm at a time"));
        assert!(message.contains("combined with other tools"));
    }
}
