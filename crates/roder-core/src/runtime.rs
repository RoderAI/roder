use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use futures::StreamExt;
use futures::future::{AbortHandle, Abortable, BoxFuture, try_join_all};
use roder_api::catalog::{
    EDIT_TOOL_EDIT, EDIT_TOOL_PATCH, PROVIDER_GEMINI, REASONING_NONE, REASONING_ULTRA,
    built_in_model_profile, built_in_model_profile_for_provider, lookup_model,
    model_supports_reasoning_effort,
};
use roder_api::context::PolicyGate;
use roder_api::events::*;
use roder_api::extension::ExtensionRegistry;
use roder_api::inference::{
    AgentInferenceRequest, HostedWebSearchConfig, HostedWebSearchMode, InferenceEngine,
    InferenceEvent, InferenceTurnContext, InstructionBundle, ModelHarnessProfile,
    ModelSchemaPolicy, ModelSelection, OutputConfig, ReasoningConfig, RuntimeHints, RuntimeProfile,
    TokenUsage, ToolCallCompleted, ToolSearchConfig, ToolSearchConfigOverlay,
    finish_reason_from_stop_reason,
};
use roder_api::inference_routing::{InferenceRoutingOutcome, ModelSelectionMode};
use roder_api::policy_mode::{PolicyDecision, PolicyMode};
use roder_api::reliability::{
    ReliabilityContext, ReliabilityDetails, ReliabilityErrorClass, ReliabilityLimitDecision,
    ReliabilityLimitRecorded, ReliabilityRequestPolicy, ReliabilityRetryDecision,
    ReliabilityRetryRecorded, provider_retry_delay_ms,
};
use roder_api::remote_runner::{
    RemoteRunnerProvider, RemoteRunnerSession, RemoteWorkspace, RunnerDestination,
    RunnerSessionState, ThreadRunnerBinding,
};
use roder_api::subagents::SubagentDefinition;
use roder_api::teams::{
    TeamId, TeamMailboxMessage, TeamMailboxMessageKind, TeamMemberDescriptor, TeamMemberRole,
    TeamMemberStatus,
};
use roder_api::thread::{
    ThreadItemEvent, ThreadItemEventKind, ThreadMetadata, ThreadSnapshot, ThreadStore,
    ThreadUsageMetadata, is_synthetic_event_thread_id, validate_thread_workspace,
};
use roder_api::tools::{ToolCall, ToolChoice, ToolExecutionContext, ToolRegistry, ToolResult};
use roder_api::transcript::{
    AssistantMessage, ErrorRecord, InputImage, ReasoningSummary, ToolCallRecord, ToolResultRecord,
    TranscriptItem, UserMessage,
};
use roder_sandbox::ScopedFilesystem;
use roder_sandbox::process::LocalProcessRunner;
use roder_skills::{SkillRegistry, SkillRegistryOptions};
use time::{Duration, OffsetDateTime};
use tokio::sync::{Mutex, RwLock, oneshot};

mod codex_v2;

use crate::artifacts::{
    ContextArtifactStore as FilesystemContextArtifactStore, default_context_artifact_dir,
};
use crate::bus::EventBus;
use crate::dynamic_workflows::{
    DynamicWorkflowEffortProfile, RuntimeDynamicWorkflowConfig, WorkflowTriggerDecision,
    classify_workflow_trigger, ultracode_reasoning_level_for_model,
};
use crate::fake_provider::FakeInferenceEngine;
use crate::goals::RuntimeGoalController;
use crate::inference_routing::{
    InferenceRoutingRequest, RuntimeInferenceRouterConfig, collect_inference_routing_candidates,
    route_inference_selection, transcript_failure_count_since,
};
use crate::instructions::{
    apply_agent_swarm_mode, apply_codex_multi_agent_mode, apply_model_instruction_overlay,
    apply_plan_mode, apply_runtime_profile, apply_task_ledger_required,
    apply_thread_developer_instructions, apply_turn_developer_context,
};
use crate::policy_gate::DefaultPolicyGate;
use crate::reliability::{
    ReliabilityLimitHit, RuntimeReliabilityConfig, TurnReliabilityState,
    provider_stream_retry_cause,
};
pub use crate::speed_policy::RuntimeSpeedPolicyConfig;
use crate::speed_policy::{SpeedPolicyState, reasoning_from_decision};
use crate::subagent_traces::{RuntimeAgentSwarmProgressSink, RuntimeSubagentTraceSink};
use crate::teams::{TeamManager, TeamMemberStartRequest, TeamStartRequest, TeamState};
use crate::thread_item_cache::{ThreadItemCache, ThreadItemCacheEntry};
use crate::verification_gate::VerificationGateState;

const MAX_TOOL_ROUNDS_PER_TURN: usize = 1024;
/// Injected when `continue_on_failure_limit` turns a consecutive-tool-failure
/// limit into a continuation, or as the empty-tool-call persistence nudge, so
/// the model keeps working instead of ending the turn early.
const RELIABILITY_CONTINUATION_PROMPT: &str = "Verify the task is fully complete. If any part remains unfinished or unverified, keep working using the available tools — try an alternative approach if one is failing. Only stop once the solution is complete and verified; if it is already complete, restate your final answer.";
/// Capacity of the runtime event broadcast ring buffer. The TUI drains this on
/// its render loop, which during an active turn only wakes at the ~6 FPS status
/// animation cadence (backend events do not wake the input poll), so up to
/// ~166ms of events buffer between drains. A reasoning-high provider that runs
/// its whole tool loop inside one turn (e.g. claude-code) streams a firehose of
/// `InferenceEventReceived` deltas plus per-step tool/thinking events; the old
/// 1024-slot buffer overflowed in those windows and silently dropped tool and
/// thinking rows from the live view. Sized for generous headroom across bursts
/// and brief render stalls.
const EVENT_BUS_CAPACITY: usize = 16_384;
const FINAL_ANSWER_PHASE: &str = "final_answer";
pub(crate) const TASK_LEDGER_TOOL_NAME: &str = "task_ledger.update";
const TASK_LEDGER_COMPLETION_REMINDER_LIMIT: u8 = 2;
const TASK_LEDGER_SCOREABLE_CHECKPOINT_SECONDS: u64 = 180;
const TASK_LEDGER_SCOREABLE_CHECKPOINT_LIMIT: u8 = 1;
pub(crate) const MIN_CHILD_DEADLINE_SECONDS: u64 = 2;
const MODEL_PROFILE_TRACE_KIND: &str = "model_profile_segment";
const MODEL_SWITCH_SUMMARY_PREFIX: &str = "Model switch summary:";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InferenceTimeoutAction {
    ScoreableCheckpoint,
    Finalization,
}

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub default_provider: String,
    pub default_model: String,
    pub reasoning: Option<String>,
    pub auto_compact_token_limit: Option<u32>,
    pub file_backed_dynamic_context: bool,
    pub hosted_web_search: HostedWebSearchConfig,
    pub tool_search: ToolSearchConfig,
    pub provider_tool_search: HashMap<String, ToolSearchConfigOverlay>,
    pub model_tool_search: HashMap<String, ToolSearchConfigOverlay>,
    pub model_edit_tools: HashMap<String, String>,
    pub model_parallel_tool_calls: HashMap<String, bool>,
    pub model_profiles: HashMap<String, ModelHarnessProfile>,
    pub tool_allowlist: Vec<String>,
    /// Seconds a host-executed external tool call may stay unresolved before it fails with a timeout error.
    pub external_tool_timeout_seconds: u64,
    pub command_shell: String,
    pub workspace: Option<String>,
    pub policy_mode: PolicyMode,
    /// Whether agent-swarm mode is active (roadmap 104). When on, the runtime
    /// injects the swarm reminder into each turn's developer instructions so any
    /// client benefits, not just the TUI.
    pub agent_swarm_mode: bool,
    pub runtime_profile: RuntimeProfile,
    pub inference_router: RuntimeInferenceRouterConfig,
    pub speed_policy: RuntimeSpeedPolicyConfig,
    pub dynamic_workflows: RuntimeDynamicWorkflowConfig,
    pub reliability: RuntimeReliabilityConfig,
    pub turn_deadline_seconds: Option<u64>,
    pub remote_runner_destination: Option<RunnerDestination>,
    pub team_data_dir: Option<PathBuf>,
    pub roadmap_data_dir: Option<PathBuf>,
    pub media_generation: crate::media_generation::RuntimeMediaGenerationConfig,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            default_provider: roder_api::catalog::PROVIDER_MOCK.to_string(),
            default_model: "mock".to_string(),
            reasoning: None,
            auto_compact_token_limit: None,
            file_backed_dynamic_context: true,
            hosted_web_search: HostedWebSearchConfig::cached(),
            tool_search: ToolSearchConfig::default(),
            provider_tool_search: HashMap::new(),
            model_tool_search: HashMap::new(),
            model_edit_tools: HashMap::new(),
            model_parallel_tool_calls: HashMap::new(),
            model_profiles: HashMap::new(),
            tool_allowlist: Vec::new(),
            external_tool_timeout_seconds: DEFAULT_EXTERNAL_TOOL_TIMEOUT_SECONDS,
            command_shell: roder_api::command_shell::default_command_shell(),
            workspace: None,
            policy_mode: PolicyMode::Default,
            agent_swarm_mode: false,
            runtime_profile: RuntimeProfile::Interactive,
            inference_router: RuntimeInferenceRouterConfig::default(),
            speed_policy: RuntimeSpeedPolicyConfig::default(),
            dynamic_workflows: RuntimeDynamicWorkflowConfig::default(),
            reliability: RuntimeReliabilityConfig::default(),
            turn_deadline_seconds: None,
            remote_runner_destination: None,
            team_data_dir: None,
            roadmap_data_dir: None,
            media_generation: crate::media_generation::RuntimeMediaGenerationConfig::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct StartTurnRequest {
    pub thread_id: ThreadId,
    pub message: String,
    pub images: Vec<InputImage>,
    pub provider_override: Option<String>,
    pub model_override: Option<String>,
    pub reasoning_override: Option<String>,
    pub workspace: String,
    pub instructions: InstructionBundle,
    /**
     * Per-turn developer-authority context for this turn's InstructionBundle.
     * Applies to every inference round of the turn, is never written to
     * thread state, and does not carry over to later turns.
     */
    pub developer_context: Option<String>,
    pub task_ledger_required: bool,
}

#[derive(Debug, Clone)]
pub struct CreateThreadRequest {
    pub title: Option<String>,
    pub workspace: String,
    pub workspace_id: Option<String>,
    pub root_id: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub selection_mode: Option<ModelSelectionMode>,
    /// Per-thread tool filter applied on top of the runtime allowlist. Empty = no filtering.
    pub tool_allowlist: Vec<String>,
    /// Host-supplied instructions added to the developer slot of every turn's inference request.
    pub developer_instructions: Option<String>,
    /// Host-executed tool specs advertised to the model on every turn of this thread.
    pub external_tools: Vec<roder_api::tools::ToolSpec>,
    /// Explicit remote-runner binding for the thread's native coding tools.
    pub runner: Option<ThreadRunnerSelection>,
}

/**
 * Thread-level remote-runner selection. The destination config is persisted
 * with the thread, so secrets must reach the provider through its
 * environment, not this config.
 */
#[derive(Debug, Clone)]
pub struct ThreadRunnerSelection {
    pub provider_id: String,
    pub config: serde_json::Value,
    /// Absolute path on the runner used as the thread's coding-tool workspace root.
    pub workspace: String,
    /**
     * Extra absolute runner paths file reads may resolve under, beyond
     * `workspace`. Writes and the working directory stay confined to
     * `workspace`.
     */
    pub read_roots: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingPlanExit {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub request_id: String,
    pub target_mode: PolicyMode,
    pub plan_summary: Option<String>,
    pub next_steps: Vec<String>,
    pub requested_at: OffsetDateTime,
    pub expires_at: Option<OffsetDateTime>,
}

pub(crate) struct PendingToolApproval {
    pub(crate) thread_id: ThreadId,
    pub(crate) turn_id: TurnId,
    pub(crate) tool_id: String,
    pub(crate) tool_name: String,
    pub(crate) call: roder_api::tools::ToolCall,
    pub(crate) tx: oneshot::Sender<bool>,
}

pub(crate) struct PendingUserInput {
    pub(crate) thread_id: ThreadId,
    pub(crate) turn_id: TurnId,
    pub(crate) tx: oneshot::Sender<serde_json::Value>,
}

/// Host answer to an external tool call delivered via `tools/resolve`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalToolResolution {
    pub output: String,
    pub is_error: bool,
}

pub(crate) struct PendingExternalToolCall {
    pub(crate) thread_id: ThreadId,
    pub(crate) turn_id: TurnId,
    pub(crate) tool_id: String,
    pub(crate) tool_name: String,
    pub(crate) tx: oneshot::Sender<ExternalToolResolution>,
}

#[derive(Clone)]
struct ActiveTurnHandle {
    thread_id: ThreadId,
    abort: AbortHandle,
    steers: Arc<Mutex<Vec<QueuedTurnSteer>>>,
}

#[derive(Clone)]
struct QueuedTurnSteer {
    message: UserMessage,
    mailbox_ack: Option<MailboxDeliveryAck>,
}

#[derive(Clone)]
struct MailboxDeliveryAck {
    team_id: TeamId,
    message_ids: Vec<String>,
}

#[derive(Clone)]
struct InheritedTurnContext {
    workspace: String,
    instructions: InstructionBundle,
    developer_context: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThreadActivity {
    pub active_turn_id: Option<TurnId>,
    pub active_flags: Vec<String>,
}

/// Per-thread settings persisted at thread creation and applied to every turn.
#[derive(Debug, Clone, Default)]
pub(crate) struct ThreadTurnOverrides {
    pub(crate) tool_allowlist: Vec<String>,
    pub(crate) developer_instructions: Option<String>,
    pub(crate) external_tools: Vec<roder_api::tools::ToolSpec>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TurnRunOutcome {
    Completed,
    Stopped,
}

impl PendingPlanExit {
    pub fn new(
        thread_id: ThreadId,
        turn_id: TurnId,
        request_id: String,
        target_mode: PolicyMode,
        plan_summary: Option<String>,
        next_steps: Vec<String>,
    ) -> Self {
        let requested_at = OffsetDateTime::now_utc();
        Self {
            thread_id,
            turn_id,
            request_id,
            target_mode,
            plan_summary,
            next_steps,
            requested_at,
            expires_at: Some(requested_at + default_plan_exit_timeout()),
        }
    }

    pub fn is_expired(&self, now: OffsetDateTime) -> bool {
        self.expires_at.is_some_and(|expires_at| now >= expires_at)
    }
}

pub fn default_plan_exit_timeout() -> Duration {
    Duration::minutes(10)
}

pub const DEFAULT_EXTERNAL_TOOL_TIMEOUT_SECONDS: u64 = 300;

fn format_mailbox_messages(team: &TeamState, messages: &[TeamMailboxMessage]) -> String {
    messages
        .iter()
        .map(|message| {
            let recipient = team
                .members
                .iter()
                .find(|member| member.id == message.to_member_id)
                .map(canonical_team_member_path)
                .unwrap_or_else(|| format!("/root/{}", message.to_member_id));
            let sender = message
                .from_member_id
                .as_deref()
                .and_then(|id| team.members.iter().find(|member| member.id == id))
                .map(canonical_team_member_path)
                .unwrap_or_else(|| "/root".to_string());
            let message_type = match message.kind {
                TeamMailboxMessageKind::Message => "MESSAGE",
                TeamMailboxMessageKind::NewTask => "NEW_TASK",
                TeamMailboxMessageKind::FinalAnswer => "FINAL_ANSWER",
            };
            format!(
                "Message Type: {message_type}\nTask name: {recipient}\nSender: {sender}\nPayload:\n{}",
                message.text
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn canonical_team_member_path(member: &TeamMemberDescriptor) -> String {
    if let Some(agent_path) = member
        .agent_path
        .as_deref()
        .filter(|path| *path == "/root" || path.starts_with("/root/"))
    {
        return agent_path.to_string();
    }
    if member.role == TeamMemberRole::Lead {
        return "/root".to_string();
    }
    member
        .task_name
        .as_deref()
        .filter(|name| !name.trim().is_empty())
        .map(|name| format!("/root/{}", name.trim().trim_matches('/')))
        .unwrap_or_else(|| format!("/root/{}", member.id))
}

fn is_codex_v2_team(team: &TeamState) -> bool {
    team.members.iter().any(|member| {
        member
            .model
            .as_deref()
            .is_some_and(|model| model_supports_reasoning_effort(model, REASONING_ULTRA))
    })
}

fn runtime_local_team_view(
    mut team: TeamState,
    active_thread_ids: &std::collections::HashSet<ThreadId>,
) -> TeamState {
    for member in &mut team.members {
        if member.status == TeamMemberStatus::Running
            && !active_thread_ids.contains(&member.thread_id)
        {
            member.status = TeamMemberStatus::Interrupted;
            member.current_turn_id = None;
        }
    }
    team
}

pub struct Runtime {
    pub bus: EventBus,
    pub registry: ExtensionRegistry,
    config: RwLock<RuntimeConfig>,
    pending_plan_exit: RwLock<Option<PendingPlanExit>>,
    pub(crate) pending_tool_approvals: Mutex<HashMap<String, PendingToolApproval>>,
    pub(crate) pending_user_inputs: Mutex<HashMap<String, PendingUserInput>>,
    pub(crate) pending_external_tool_calls: Mutex<HashMap<String, PendingExternalToolCall>>,
    active_turns: RwLock<HashMap<TurnId, ActiveTurnHandle>>,
    active_turn_selections: RwLock<HashMap<TurnId, ModelSelectionMode>>,
    active_turn_contexts: RwLock<HashMap<TurnId, InheritedTurnContext>>,
    // Spawn-time live authority retained for every reusable turn of a long-lived teammate.
    // This stays process-local because developer_context is explicitly volatile and must never
    // be persisted to thread state.
    team_member_turn_contexts: Mutex<HashMap<ThreadId, InheritedTurnContext>>,
    workspace: PathBuf,
    teams: TeamManager,
    agent_team_spawn_lock: Mutex<()>,
    pub(crate) roadmaps: Mutex<roder_roadmap::RoadmapRuntime>,
    pub(crate) goals: Arc<RuntimeGoalController>,
    context_artifacts: roder_api::artifacts::ContextArtifactStore,
    pub(crate) thread_store: Option<Arc<dyn ThreadStore>>,
    thread_item_cache: Mutex<ThreadItemCache>,
    pub(crate) tool_registry: ToolRegistry,
    media_generation: Arc<crate::media_generation::MediaGenerationService>,
    pub(crate) skills: RwLock<SkillRegistry>,
    /// Lazily-started bounded dispatch of emitted events to registry
    /// `EventSink`s (process extensions etc.); see `event_sink_dispatch`.
    event_sink_dispatcher: tokio::sync::OnceCell<crate::event_sink_dispatch::EventSinkDispatcher>,
    pub(crate) compaction_hysteresis: std::sync::Mutex<HashMap<ThreadId, u32>>,
    /// Per-thread agent-swarm mode overrides (roadmap 104). A thread present in
    /// this map uses its stored value; absent threads fall back to the
    /// runtime-global `RuntimeConfig.agent_swarm_mode` default. This mirrors the
    /// team per-member policy-mode override idiom so swarm mode is per-thread
    /// like the other `thread/*` operations, without a separate runtime.
    agent_swarm_modes: RwLock<HashMap<ThreadId, bool>>,
}

impl Runtime {
    pub fn new(registry: ExtensionRegistry, config: RuntimeConfig) -> anyhow::Result<Self> {
        if registry.inference_engines.is_empty() {
            anyhow::bail!("at least one inference engine must be registered");
        }
        validate_runtime_config_reasoning(&config)?;
        validate_runtime_inference_router_config(&registry, &config)?;

        let bus = EventBus::new(EVENT_BUS_CAPACITY);
        let thread_store = registry
            .thread_stores
            .first()
            .map(|factory| factory.create());
        let mut tool_registry = ToolRegistry::default();
        for contributor in &registry.tools {
            contributor
                .contribute(&mut tool_registry)
                .with_context(|| format!("tool contributor {} failed", contributor.id()))?;
        }
        crate::agent_control_tools::contribute_agent_control_tools(&mut tool_registry)?;

        let media_generation = Arc::new(crate::media_generation::MediaGenerationService::new(
            registry.media_generator_providers.clone(),
            config.media_generation.clone(),
        ));
        tool_registry.replace(Arc::new(
            crate::media_generation::MediaGenerateImageTool::new(media_generation.clone()),
        ));

        let team_data_dir = config.team_data_dir.clone();
        let workspace = config
            .workspace
            .clone()
            .map(PathBuf::from)
            .unwrap_or(std::env::current_dir()?);
        let roadmap_data_dir = config
            .roadmap_data_dir
            .clone()
            .unwrap_or_else(|| workspace.join(".roder"));
        let context_artifacts = thread_store
            .as_ref()
            .and_then(|store| store.context_artifact_store())
            .or_else(|| {
                thread_store
                    .as_ref()
                    .and_then(|store| store.local_thread_root())
                    .map(FilesystemContextArtifactStore::shared_thread_scoped)
            })
            .unwrap_or_else(|| {
                FilesystemContextArtifactStore::shared_legacy(default_context_artifact_dir())
            });
        let goals = Arc::new(RuntimeGoalController::new(
            bus.clone(),
            thread_store.clone(),
        ));
        let runtime = Self {
            bus,
            registry,
            config: RwLock::new(config),
            pending_plan_exit: RwLock::new(None),
            pending_tool_approvals: Mutex::new(HashMap::new()),
            pending_user_inputs: Mutex::new(HashMap::new()),
            pending_external_tool_calls: Mutex::new(HashMap::new()),
            active_turns: RwLock::new(HashMap::new()),
            active_turn_selections: RwLock::new(HashMap::new()),
            active_turn_contexts: RwLock::new(HashMap::new()),
            team_member_turn_contexts: Mutex::new(HashMap::new()),
            workspace: workspace.clone(),
            teams: TeamManager::new(
                team_data_dir.unwrap_or_else(crate::teams::default_team_data_dir),
            ),
            agent_team_spawn_lock: Mutex::new(()),
            roadmaps: Mutex::new(roder_roadmap::RoadmapRuntime::new(
                workspace,
                roadmap_data_dir,
            )),
            goals,
            context_artifacts,
            thread_store,
            thread_item_cache: Mutex::new(ThreadItemCache::default()),
            tool_registry,
            media_generation,
            skills: RwLock::new(SkillRegistry::load(SkillRegistryOptions::new(
                PathBuf::new(),
            ))),
            event_sink_dispatcher: tokio::sync::OnceCell::new(),
            compaction_hysteresis: crate::compaction_runtime::compaction_hysteresis_state(),
            agent_swarm_modes: RwLock::new(HashMap::new()),
        };
        runtime.bus.emit(RoderEvent::RuntimeStarted(RuntimeStarted {
            timestamp: OffsetDateTime::now_utc(),
        }));
        for manifest in &runtime.registry.manifests {
            runtime
                .bus
                .emit(RoderEvent::ExtensionRegistered(ExtensionRegistered {
                    extension_id: manifest.id.clone(),
                    timestamp: OffsetDateTime::now_utc(),
                }));
        }
        Ok(runtime)
    }

    pub fn from_engine(engine: Arc<dyn InferenceEngine>) -> anyhow::Result<Self> {
        let mut builder = roder_api::extension::ExtensionRegistryBuilder::new();
        builder.inference_engine(engine);
        Self::new(builder.build()?, RuntimeConfig::default())
    }

    pub fn fake() -> anyhow::Result<Self> {
        Self::from_engine(Arc::new(FakeInferenceEngine))
    }

    pub fn subscribe_events(&self) -> tokio::sync::broadcast::Receiver<EventEnvelope> {
        self.bus.subscribe()
    }

    pub fn registry(&self) -> &ExtensionRegistry {
        &self.registry
    }

    pub fn media_generation(&self) -> Arc<crate::media_generation::MediaGenerationService> {
        self.media_generation.clone()
    }

    pub fn context_artifacts(&self) -> roder_api::artifacts::ContextArtifactStore {
        self.context_artifacts.clone()
    }

    pub async fn execute_workflow_tool(
        &self,
        thread_id: ThreadId,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> anyhow::Result<ToolResult> {
        let Some(executor) = self.tool_registry.get(tool_name) else {
            anyhow::bail!("tool not found: {tool_name}");
        };
        let tool_call = ToolCall {
            id: format!("slash-{tool_name}"),
            name: tool_name.to_string(),
            raw_arguments: serde_json::to_string(&arguments)?,
            arguments,
            thread_id: thread_id.clone(),
            turn_id: "slash-command".to_string(),
        };
        let runtime_config = self.status().await;
        let ctx = self.tool_execution_context(
            thread_id,
            "slash-command".to_string(),
            runtime_config.policy_mode,
            runtime_config.workspace.as_deref(),
            Some(&runtime_config.command_shell),
        );
        executor.execute(ctx, tool_call).await
    }

    pub(crate) fn tool_execution_context(
        &self,
        thread_id: ThreadId,
        turn_id: TurnId,
        mode: PolicyMode,
        workspace: Option<&str>,
        command_shell: Option<&str>,
    ) -> ToolExecutionContext {
        let mut ctx = ToolExecutionContext::new(thread_id, turn_id, mode)
            .with_command_shell(command_shell.unwrap_or_default())
            .with_process_runner(Arc::new(LocalProcessRunner))
            .with_context_artifacts(self.context_artifacts.backend())
            .with_goal_controller(self.goals.clone())
            .with_subagent_trace_sink(Arc::new(RuntimeSubagentTraceSink::new(
                self.bus.clone(),
                self.thread_store.clone(),
            )))
            .with_swarm_progress_sink(Arc::new(RuntimeAgentSwarmProgressSink::new(
                self.bus.clone(),
                self.thread_store.clone(),
            )));
        if let Some(workspace) = workspace {
            ctx = ctx.with_workspace_handle(Arc::new(ScopedFilesystem::new(workspace)));
        }
        ctx
    }

    pub async fn status(&self) -> RuntimeConfig {
        self.config.read().await.clone()
    }

    pub async fn set_skills(&self, skills: SkillRegistry) {
        *self.skills.write().await = skills;
    }

    pub async fn skills_snapshot(&self) -> SkillRegistry {
        self.skills.read().await.clone()
    }

    pub fn workspace(&self) -> PathBuf {
        self.workspace.clone()
    }

    pub async fn set_remote_runner_destination(&self, destination: Option<RunnerDestination>) {
        let lifecycle = destination.as_ref().map(|destination| RunnerLifecycle {
            destination_id: destination.id.clone(),
            provider_id: destination.provider_id.clone(),
            state: "configured".to_string(),
            session_id: None,
            timestamp: OffsetDateTime::now_utc(),
        });
        self.config.write().await.remote_runner_destination = destination;
        if let Some(lifecycle) = lifecycle {
            self.emit(RoderEvent::RunnerLifecycle(lifecycle)).await;
        } else {
            self.emit(RoderEvent::RunnerLifecycle(RunnerLifecycle {
                destination_id: "local".to_string(),
                provider_id: "local".to_string(),
                state: "local_fallback".to_string(),
                session_id: None,
                timestamp: OffsetDateTime::now_utc(),
            }))
            .await;
        }
    }

    pub async fn set_file_backed_dynamic_context(&self, enabled: bool) -> RuntimeConfig {
        let mut cfg = self.config.write().await;
        cfg.file_backed_dynamic_context = enabled;
        cfg.clone()
    }

    pub async fn set_command_shell(&self, shell: String) -> RuntimeConfig {
        let mut cfg = self.config.write().await;
        cfg.command_shell = shell;
        cfg.clone()
    }

    pub async fn pending_plan_exit(&self) -> Option<PendingPlanExit> {
        let mut pending = self.pending_plan_exit.write().await;
        let current = pending.clone()?;
        if !current.is_expired(OffsetDateTime::now_utc()) {
            return Some(current);
        }
        *pending = None;
        drop(pending);
        self.emit_plan_exit_resolved(&current, false, self.status().await.policy_mode)
            .await;
        None
    }

    pub async fn set_policy_mode(
        &self,
        mode: PolicyMode,
        reason: Option<String>,
    ) -> anyhow::Result<RuntimeConfig> {
        let mut cfg = self.config.write().await;
        let previous_mode = cfg.policy_mode;
        cfg.policy_mode = mode;
        let next = cfg.clone();
        drop(cfg);
        self.emit(RoderEvent::PolicyModeChanged(PolicyModeChanged {
            thread_id: "runtime".to_string(),
            turn_id: None,
            previous_mode,
            new_mode: mode,
            reason,
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        self.auto_resolve_pending_tool_approvals_for_mode(mode)
            .await;
        Ok(next)
    }

    /// Toggle agent-swarm mode for the runtime (roadmap 104). When enabled, the
    /// swarm reminder is injected into each turn's developer instructions so
    /// every app-server/SDK client gets it, not only the TUI.
    pub async fn set_agent_swarm_mode(
        &self,
        enabled: bool,
        trigger: roder_api::subagents::AgentSwarmModeTrigger,
    ) -> anyhow::Result<RuntimeConfig> {
        let mut cfg = self.config.write().await;
        cfg.agent_swarm_mode = enabled;
        let next = cfg.clone();
        drop(cfg);
        self.emit(RoderEvent::AgentSwarmModeChanged(
            roder_api::subagents::AgentSwarmModeChanged {
                thread_id: "runtime".to_string(),
                turn_id: None,
                enabled,
                trigger,
                timestamp: OffsetDateTime::now_utc(),
            },
        ))
        .await;
        Ok(next)
    }

    /// Toggle agent-swarm mode for a single thread (roadmap 104). The override is
    /// stored per thread so toggling swarm mode on one thread does not leak the
    /// reminder into other threads sharing the runtime; absent threads fall back
    /// to the runtime-global default. The emitted `AgentSwarmModeChanged` event
    /// carries the real `thread_id`.
    pub async fn set_agent_swarm_mode_for_thread(
        &self,
        thread_id: &str,
        enabled: bool,
        trigger: roder_api::subagents::AgentSwarmModeTrigger,
    ) -> bool {
        {
            let mut modes = self.agent_swarm_modes.write().await;
            modes.insert(thread_id.to_string(), enabled);
        }
        self.emit(RoderEvent::AgentSwarmModeChanged(
            roder_api::subagents::AgentSwarmModeChanged {
                thread_id: thread_id.to_string(),
                turn_id: None,
                enabled,
                trigger,
                timestamp: OffsetDateTime::now_utc(),
            },
        ))
        .await;
        enabled
    }

    /// Resolve whether agent-swarm mode is active for `thread_id`: the per-thread
    /// override when present, otherwise the runtime-global default. The turn loop
    /// uses this to decide whether to inject the swarm reminder.
    pub async fn effective_agent_swarm_mode_for_thread(&self, thread_id: &str) -> bool {
        if let Some(enabled) = self.agent_swarm_modes.read().await.get(thread_id).copied() {
            return enabled;
        }
        self.status().await.agent_swarm_mode
    }

    pub async fn set_hosted_web_search(
        &self,
        mode: HostedWebSearchMode,
    ) -> anyhow::Result<RuntimeConfig> {
        let mut cfg = self.config.write().await;
        cfg.hosted_web_search = HostedWebSearchConfig { mode };
        Ok(cfg.clone())
    }

    async fn auto_resolve_pending_tool_approvals_for_mode(&self, mode: PolicyMode) {
        let gate = DefaultPolicyGate::new();
        let mut pending = self.pending_tool_approvals.lock().await;
        let approval_ids = pending
            .iter()
            .filter_map(|(approval_id, approval)| {
                let ctx = ToolExecutionContext::new(
                    approval.thread_id.clone(),
                    approval.turn_id.clone(),
                    mode,
                );
                matches!(
                    gate.decide(&approval.call, mode, &ctx),
                    PolicyDecision::AutoApproved { .. }
                )
                .then_some(approval_id.clone())
            })
            .collect::<Vec<_>>();
        let approvals = approval_ids
            .into_iter()
            .filter_map(|approval_id| {
                pending
                    .remove(&approval_id)
                    .map(|approval| (approval_id, approval))
            })
            .collect::<Vec<_>>();
        drop(pending);

        for (approval_id, approval) in approvals {
            let ctx = ToolExecutionContext::new(
                approval.thread_id.clone(),
                approval.turn_id.clone(),
                mode,
            );
            let decision = gate.decide(&approval.call, mode, &ctx);
            self.emit(RoderEvent::PolicyDecisionRecorded(PolicyDecisionRecorded {
                thread_id: approval.thread_id.clone(),
                turn_id: approval.turn_id.clone(),
                tool_id: approval.tool_id.clone(),
                tool_name: approval.tool_name.clone(),
                mode,
                decision,
                timestamp: OffsetDateTime::now_utc(),
            }))
            .await;
            if mode == PolicyMode::Bypass {
                self.emit(RoderEvent::PolicyBypassActive(PolicyBypassActive {
                    thread_id: approval.thread_id.clone(),
                    turn_id: approval.turn_id.clone(),
                    tool_id: approval.tool_id.clone(),
                    tool_name: approval.tool_name.clone(),
                    timestamp: OffsetDateTime::now_utc(),
                }))
                .await;
            }
            self.emit(RoderEvent::ApprovalResolved(ApprovalResolved {
                thread_id: approval.thread_id,
                turn_id: approval.turn_id,
                approval_id,
                tool_id: approval.tool_id,
                tool_name: approval.tool_name,
                approved: true,
                timestamp: OffsetDateTime::now_utc(),
            }))
            .await;
            let _ = approval.tx.send(true);
        }
    }

    pub async fn record_pending_plan_exit(&self, pending: PendingPlanExit) {
        *self.pending_plan_exit.write().await = Some(pending.clone());
        self.emit(RoderEvent::PolicyExitPlanRequested(
            PolicyExitPlanRequested {
                thread_id: pending.thread_id,
                turn_id: pending.turn_id,
                request_id: pending.request_id,
                target_mode: pending.target_mode,
                plan_summary: pending.plan_summary,
                next_steps: pending.next_steps,
                timestamp: OffsetDateTime::now_utc(),
            },
        ))
        .await;
    }

    pub async fn resolve_pending_plan_exit(
        &self,
        request_id: &str,
        approved: bool,
    ) -> anyhow::Result<Option<PendingPlanExit>> {
        let mut pending = self.pending_plan_exit.write().await;
        let Some(current) = pending.clone() else {
            return Ok(None);
        };
        if current.request_id != request_id {
            anyhow::bail!("pending plan exit request {request_id:?} was not found");
        }
        *pending = None;
        drop(pending);

        let approved = approved && !current.is_expired(OffsetDateTime::now_utc());
        let resolved_mode = if approved {
            let mut cfg = self.config.write().await;
            let previous_mode = cfg.policy_mode;
            cfg.policy_mode = current.target_mode;
            drop(cfg);
            self.emit(RoderEvent::PolicyModeChanged(PolicyModeChanged {
                thread_id: current.thread_id.clone(),
                turn_id: Some(current.turn_id.clone()),
                previous_mode,
                new_mode: current.target_mode,
                reason: Some("approved plan exit".to_string()),
                timestamp: OffsetDateTime::now_utc(),
            }))
            .await;
            self.auto_resolve_pending_tool_approvals_for_mode(current.target_mode)
                .await;
            current.target_mode
        } else {
            self.status().await.policy_mode
        };
        self.emit_plan_exit_resolved(&current, approved, resolved_mode)
            .await;
        Ok(Some(current))
    }

    pub async fn resolve_tool_approval(
        &self,
        approval_id: &str,
        approved: bool,
    ) -> anyhow::Result<bool> {
        let pending = self.pending_tool_approvals.lock().await.remove(approval_id);
        let Some(pending) = pending else {
            return Ok(false);
        };
        self.emit(RoderEvent::ApprovalResolved(ApprovalResolved {
            thread_id: pending.thread_id,
            turn_id: pending.turn_id,
            approval_id: approval_id.to_string(),
            tool_id: pending.tool_id,
            tool_name: pending.tool_name,
            approved,
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        let _ = pending.tx.send(approved);
        Ok(true)
    }

    pub async fn request_app_server_tool_approval(
        &self,
        call: ToolCall,
        reason: Option<String>,
    ) -> anyhow::Result<bool> {
        let approval_id = call.id.clone();
        let (tx, rx) = oneshot::channel();
        self.pending_tool_approvals.lock().await.insert(
            approval_id.clone(),
            PendingToolApproval {
                thread_id: call.thread_id.clone(),
                turn_id: call.turn_id.clone(),
                tool_id: call.id.clone(),
                tool_name: call.name.clone(),
                call: call.clone(),
                tx,
            },
        );
        self.emit(RoderEvent::ApprovalRequested(ApprovalRequested {
            thread_id: call.thread_id.clone(),
            turn_id: call.turn_id.clone(),
            approval_id,
            tool_id: call.id.clone(),
            tool_name: call.name.clone(),
            reason,
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        Ok(rx.await.unwrap_or(false))
    }

    /// Completes a pending host-executed tool call (`tools/resolve`). Returns false when the
    /// request id is unknown, already resolved, timed out, or cancelled by a turn interrupt.
    pub async fn resolve_external_tool_call(
        &self,
        request_id: &str,
        resolution: ExternalToolResolution,
    ) -> anyhow::Result<bool> {
        let pending = self
            .pending_external_tool_calls
            .lock()
            .await
            .remove(request_id);
        let Some(pending) = pending else {
            return Ok(false);
        };
        self.emit(RoderEvent::ExternalToolCallResolved(
            ExternalToolCallResolved {
                thread_id: pending.thread_id,
                turn_id: pending.turn_id,
                request_id: request_id.to_string(),
                tool_id: pending.tool_id,
                tool_name: pending.tool_name,
                outcome: ExternalToolCallOutcome::Resolved,
                is_error: resolution.is_error,
                timestamp: OffsetDateTime::now_utc(),
            },
        ))
        .await;
        let _ = pending.tx.send(resolution);
        Ok(true)
    }

    /// Drops every pending external tool call for the turn and reports them as cancelled so an
    /// interrupt does not leak requests waiting on `tools/resolve`.
    async fn cancel_pending_external_tool_calls_for_turn(&self, turn_id: &TurnId) {
        let cancelled = {
            let mut pending = self.pending_external_tool_calls.lock().await;
            let request_ids = pending
                .iter()
                .filter(|(_, call)| &call.turn_id == turn_id)
                .map(|(request_id, _)| request_id.clone())
                .collect::<Vec<_>>();
            request_ids
                .into_iter()
                .filter_map(|request_id| pending.remove(&request_id).map(|call| (request_id, call)))
                .collect::<Vec<_>>()
        };
        for (request_id, call) in cancelled {
            self.emit(RoderEvent::ExternalToolCallResolved(
                ExternalToolCallResolved {
                    thread_id: call.thread_id,
                    turn_id: call.turn_id,
                    request_id,
                    tool_id: call.tool_id,
                    tool_name: call.tool_name,
                    outcome: ExternalToolCallOutcome::Cancelled,
                    is_error: true,
                    timestamp: OffsetDateTime::now_utc(),
                },
            ))
            .await;
        }
    }

    pub async fn resolve_user_input(
        &self,
        request_id: &str,
        answers: serde_json::Value,
    ) -> anyhow::Result<bool> {
        let pending = self.pending_user_inputs.lock().await.remove(request_id);
        let Some(pending) = pending else {
            return Ok(false);
        };
        self.emit(RoderEvent::UserInputResolved(UserInputResolved {
            thread_id: pending.thread_id,
            turn_id: pending.turn_id,
            request_id: request_id.to_string(),
            answers: answers.clone(),
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        let _ = pending.tx.send(answers);
        Ok(true)
    }

    async fn emit_plan_exit_resolved(
        &self,
        current: &PendingPlanExit,
        approved: bool,
        resolved_mode: PolicyMode,
    ) {
        self.emit(RoderEvent::PolicyExitPlanResolved(PolicyExitPlanResolved {
            thread_id: current.thread_id.clone(),
            turn_id: current.turn_id.clone(),
            request_id: current.request_id.clone(),
            approved,
            target_mode: current.target_mode,
            resolved_mode,
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
    }

    pub async fn select_provider(
        &self,
        provider: String,
        model: Option<String>,
        reasoning: Option<String>,
    ) -> anyhow::Result<RuntimeConfig> {
        let next = self
            .preview_provider_selection(provider, model, reasoning)
            .await?;
        let mut cfg = self.config.write().await;
        *cfg = next;
        Ok(cfg.clone())
    }

    pub async fn preview_provider_selection(
        &self,
        provider: String,
        model: Option<String>,
        reasoning: Option<String>,
    ) -> anyhow::Result<RuntimeConfig> {
        self.engine_for(&provider)?;
        let mut cfg = self.config.read().await.clone();
        cfg.default_provider = provider;
        if let Some(model) = model {
            cfg.default_model = model;
        }
        if let Some(reasoning) = reasoning {
            if reasoning == REASONING_NONE
                && !model_supports_reasoning(&cfg.default_model, &reasoning)
            {
                return Ok(cfg.clone());
            }
            validate_reasoning_effort(&cfg.default_model, &reasoning)?;
            cfg.reasoning = Some(reasoning);
        }
        Ok(cfg)
    }

    pub async fn effective_reasoning(&self) -> String {
        let cfg = self.config.read().await;
        effective_reasoning_for_model(&cfg, &cfg.default_model)
    }

    pub fn effective_reasoning_for_config(cfg: &RuntimeConfig) -> String {
        effective_reasoning_for_model(cfg, &cfg.default_model)
    }

    pub async fn set_dynamic_workflow_effort(
        &self,
        effort_profile: DynamicWorkflowEffortProfile,
    ) -> RuntimeConfig {
        let mut cfg = self.config.write().await;
        cfg.dynamic_workflows.effort_profile = effort_profile;
        cfg.clone()
    }

    pub async fn dynamic_workflow_trigger_decision(
        &self,
        message: &str,
    ) -> WorkflowTriggerDecision {
        let cfg = self.config.read().await;
        classify_workflow_trigger(message, &cfg.dynamic_workflows)
    }

    pub async fn create_thread(&self, title: Option<String>) -> anyhow::Result<ThreadMetadata> {
        self.create_thread_with(CreateThreadRequest {
            title,
            workspace: self.workspace.display().to_string(),
            workspace_id: None,
            root_id: None,
            provider: None,
            model: None,
            selection_mode: None,
            tool_allowlist: Vec::new(),
            developer_instructions: None,
            external_tools: Vec::new(),
            runner: None,
        })
        .await
    }

    /**
     * Resolves an explicit thread-runner selection into a persisted binding.
     * Fails fast at thread creation when the provider is missing or rejects
     * the destination, instead of surfacing the error on the first tool call.
     */
    async fn resolve_thread_runner_binding(
        &self,
        thread_id: &str,
        selection: ThreadRunnerSelection,
    ) -> anyhow::Result<ThreadRunnerBinding> {
        let provider = self
            .registry
            .remote_runner_providers
            .iter()
            .find(|provider| provider.id() == selection.provider_id)
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "remote runner provider {:?} is not installed",
                    selection.provider_id
                )
            })?;
        let workspace = selection.workspace.trim();
        anyhow::ensure!(
            std::path::Path::new(workspace).is_absolute(),
            "runner workspace must be an absolute path on the runner: {workspace:?}"
        );
        let mut read_roots = Vec::with_capacity(selection.read_roots.len());
        for read_root in &selection.read_roots {
            let trimmed = read_root.trim();
            anyhow::ensure!(
                std::path::Path::new(trimmed).is_absolute(),
                "runner read root must be an absolute path on the runner: {trimmed:?}"
            );
            read_roots.push(PathBuf::from(trimmed));
        }
        let destination = RunnerDestination {
            id: format!("thread-{thread_id}"),
            provider_id: selection.provider_id,
            config: selection.config,
            default_manifest: roder_api::remote_runner::RunnerManifest::default(),
        };
        provider.validate_destination(&destination).await?;
        Ok(ThreadRunnerBinding {
            destination,
            workspace: PathBuf::from(workspace),
            read_roots,
        })
    }

    /**
     * Build a per-thread binding from a runtime-level destination (selected via
     * the TUI runner picker or config `default_destination`) when the
     * destination's provider advertises a default workspace. Returns `None` when
     * there is no destination or the provider opts out (no default workspace),
     * preserving the legacy behavior where such threads keep local tools.
     */
    async fn synthesize_runtime_runner_binding(
        &self,
        thread_id: &str,
        destination: Option<RunnerDestination>,
    ) -> anyhow::Result<Option<ThreadRunnerBinding>> {
        let Some(destination) = destination else {
            return Ok(None);
        };
        let Some(provider) = self
            .registry
            .remote_runner_providers
            .iter()
            .find(|provider| provider.id() == destination.provider_id)
        else {
            return Ok(None);
        };
        // An explicit workspace in the destination config wins over the
        // provider default; absence of both means the provider opts out.
        let workspace = destination
            .config
            .get("working_dir")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .or_else(|| provider.default_workspace());
        let Some(workspace) = workspace else {
            return Ok(None);
        };
        let selection = ThreadRunnerSelection {
            provider_id: destination.provider_id.clone(),
            config: destination.config.clone(),
            workspace,
            read_roots: Vec::new(),
        };
        Ok(Some(
            self.resolve_thread_runner_binding(thread_id, selection)
                .await?,
        ))
    }

    /// Validates a runner selection without creating a thread; the placeholder destination id only appears in error messages.
    pub async fn validate_thread_runner_selection(
        &self,
        selection: ThreadRunnerSelection,
    ) -> anyhow::Result<()> {
        self.resolve_thread_runner_binding("validate", selection)
            .await
            .map(|_| ())
    }

    pub async fn create_thread_with(
        &self,
        req: CreateThreadRequest,
    ) -> anyhow::Result<ThreadMetadata> {
        let cfg = self.config.read().await.clone();
        let now = OffsetDateTime::now_utc();
        let workspace = validate_thread_workspace(&req.workspace)?;
        let provider = req.provider.unwrap_or(cfg.default_provider);
        let model = req.model.unwrap_or(cfg.default_model);
        let selection_mode = req
            .selection_mode
            .unwrap_or_else(|| ModelSelectionMode::manual(provider.clone(), model.clone(), None));
        let thread_id = uuid::Uuid::new_v4().to_string();
        let runner_binding = match req.runner {
            Some(selection) => Some(
                self.resolve_thread_runner_binding(&thread_id, selection)
                    .await?,
            ),
            // No explicit per-thread selection: if a runtime-level destination
            // is active and its provider advertises a default workspace, bind
            // the new thread so its coding tools route into the runner. This is
            // what makes a TUI/config-selected runner (e.g. Blaxel) actually
            // execute tools in the sandbox instead of locally. Providers that
            // return no default workspace stay unbound (legacy local-tools).
            None => {
                self.synthesize_runtime_runner_binding(
                    &thread_id,
                    cfg.remote_runner_destination.clone(),
                )
                .await?
            }
        };
        let runner_destination = runner_binding
            .as_ref()
            .map(|binding| binding.destination.clone())
            .or_else(|| cfg.remote_runner_destination.clone());
        let metadata = ThreadMetadata {
            thread_id,
            title: req.title,
            workspace,
            workspace_id: req.workspace_id,
            root_id: req.root_id,
            provider: Some(provider),
            model: Some(model),
            selection_mode: Some(selection_mode),
            tool_allowlist: req.tool_allowlist,
            developer_instructions: req.developer_instructions,
            external_tools: req.external_tools,
            runner_destination,
            runner_state: None,
            runner_binding,
            created_at: now,
            updated_at: now,
            message_count: 0,
            usage: None,
            parent_thread_id: None,
            forked_from_turn_id: None,
            workspace_fork: None,
        };

        let metadata = if let Some(store) = &self.thread_store {
            store.create_thread(metadata).await?
        } else {
            metadata
        };
        self.emit(RoderEvent::ThreadCreated(ThreadCreated {
            thread_id: metadata.thread_id.clone(),
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        Ok(metadata)
    }

    pub async fn list_threads(&self) -> anyhow::Result<Vec<ThreadMetadata>> {
        if let Some(store) = &self.thread_store {
            return store.list_threads().await;
        }
        Ok(Vec::new())
    }

    pub async fn list_threads_page(
        &self,
        options: roder_api::thread::ThreadListOptions,
    ) -> anyhow::Result<roder_api::thread::ThreadListPage> {
        if let Some(store) = &self.thread_store {
            return store.list_threads_page(options).await;
        }
        Ok(roder_api::thread::ThreadListPage::default())
    }

    pub async fn load_thread_metadata(
        &self,
        thread_id: &str,
    ) -> anyhow::Result<Option<roder_api::thread::ThreadMetadata>> {
        if let Some(store) = &self.thread_store {
            return store.load_thread_metadata(&thread_id.to_string()).await;
        }
        Ok(None)
    }

    pub async fn archive_thread(&self, thread_id: &str) -> anyhow::Result<bool> {
        let archived = if let Some(store) = &self.thread_store {
            store.archive_thread(&thread_id.to_string()).await?
        } else {
            false
        };
        if archived {
            self.thread_item_cache
                .lock()
                .await
                .remove_thread(&thread_id.to_string());
        }
        Ok(archived)
    }

    pub async fn start_team(&self, req: TeamStartRequest) -> anyhow::Result<TeamState> {
        let cfg = self.config.read().await.clone();
        let workspace = self.workspace.display().to_string();
        let lead_thread_id = match req.lead_thread_id {
            Some(thread_id) => thread_id,
            None => {
                self.create_thread_with(CreateThreadRequest {
                    title: Some("Team lead".to_string()),
                    workspace: workspace.clone(),
                    workspace_id: None,
                    root_id: None,
                    provider: None,
                    model: None,
                    selection_mode: None,
                    tool_allowlist: Vec::new(),
                    developer_instructions: None,
                    external_tools: Vec::new(),
                    runner: None,
                })
                .await?
                .thread_id
            }
        };
        let team_id = uuid::Uuid::new_v4().to_string();
        let active_lead_turn_id = self.active_turn_for_thread(&lead_thread_id).await;
        let lead_selection = match active_lead_turn_id.as_ref() {
            Some(turn_id) => self
                .active_turn_selections
                .read()
                .await
                .get(turn_id)
                .cloned(),
            None => self.selection_mode_for_thread(&lead_thread_id).await?,
        };
        let lead_concrete_selection = lead_selection
            .as_ref()
            .map(ModelSelectionMode::concrete_selection);
        let mut lead = crate::teams::lead_member(
            lead_thread_id.clone(),
            lead_concrete_selection
                .as_ref()
                .map(|selection| selection.provider.clone())
                .or_else(|| Some(cfg.default_provider.clone())),
            lead_concrete_selection
                .as_ref()
                .map(|selection| selection.model.clone())
                .or_else(|| Some(cfg.default_model.clone())),
            cfg.policy_mode,
        );
        if let Some(turn_id) = active_lead_turn_id {
            lead.current_turn_id = Some(turn_id);
            lead.status = TeamMemberStatus::Running;
        }
        let mut members = vec![lead];

        for (index, member) in req.members.into_iter().enumerate() {
            let thread = self
                .create_thread_with(CreateThreadRequest {
                    title: Some(member.name.clone()),
                    workspace: workspace.clone(),
                    workspace_id: None,
                    root_id: None,
                    provider: member.model_provider.clone(),
                    model: member.model.clone(),
                    selection_mode: None,
                    tool_allowlist: Vec::new(),
                    developer_instructions: None,
                    external_tools: Vec::new(),
                    runner: None,
                })
                .await?;
            let member_id = format!("member-{}", index + 1);
            let descriptor = crate::teams::teammate_member(
                member_id.clone(),
                member.name,
                thread.thread_id.clone(),
                member.model_provider.or(thread.provider),
                member.model.or(thread.model),
                cfg.policy_mode,
            );
            members.push(descriptor);
        }

        let now = OffsetDateTime::now_utc();
        let team = self
            .teams
            .insert(TeamState {
                id: team_id.clone(),
                lead_thread_id: lead_thread_id.clone(),
                display_mode: req.display_mode,
                members,
                mailbox: Vec::new(),
                tasks: Vec::new(),
                created_at: now,
                updated_at: now,
            })
            .await?;
        self.emit(RoderEvent::TeamStarted(TeamStarted {
            team_id: team_id.clone(),
            lead_thread_id,
            display_mode: team.display_mode,
            members: team.members.clone(),
            tasks: team.tasks.clone(),
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        for member in team
            .members
            .iter()
            .filter(|member| member.role != roder_api::teams::TeamMemberRole::Lead)
        {
            self.emit(RoderEvent::TeamMemberStarted(TeamMemberStarted {
                team_id: team_id.clone(),
                member: member.clone(),
                timestamp: OffsetDateTime::now_utc(),
            }))
            .await;
        }
        Ok(team)
    }

    pub async fn list_teams(&self) -> Vec<TeamState> {
        let active_thread_ids = self
            .active_turns
            .read()
            .await
            .values()
            .map(|handle| handle.thread_id.clone())
            .collect::<std::collections::HashSet<_>>();
        self.teams
            .list()
            .await
            .into_iter()
            .map(|team| runtime_local_team_view(team, &active_thread_ids))
            .collect()
    }

    pub async fn read_team(&self, team_id: &str) -> Option<TeamState> {
        let active_thread_ids = self
            .active_turns
            .read()
            .await
            .values()
            .map(|handle| handle.thread_id.clone())
            .collect::<std::collections::HashSet<_>>();
        self.teams
            .get(team_id)
            .await
            .map(|team| runtime_local_team_view(team, &active_thread_ids))
    }

    pub async fn start_team_member(
        &self,
        team_id: &str,
        req: TeamMemberStartRequest,
    ) -> anyhow::Result<TeamState> {
        self.start_team_member_with_selection(team_id, req, None)
            .await
    }

    pub async fn message_team_member(
        self: &Arc<Self>,
        team_id: &str,
        member_id: &str,
        message: String,
    ) -> anyhow::Result<TurnId> {
        let team = self
            .read_team(team_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("unknown team {team_id:?}"))?;
        self.followup_team_member(&team.lead_thread_id, team_id, member_id, message)
            .await
    }

    /// Queue a mailbox message without starting an idle agent. If the target is
    /// already running, the message is delivered at the next inference boundary.
    pub(crate) async fn queue_team_member_message(
        self: &Arc<Self>,
        caller_thread_id: &ThreadId,
        team_id: &str,
        member_id: &str,
        message: String,
    ) -> anyhow::Result<Option<TurnId>> {
        let _delivery_guard = self.agent_team_spawn_lock.lock().await;
        let team = self
            .read_team(team_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("unknown team {team_id:?}"))?;
        let member = team
            .members
            .iter()
            .find(|member| member.id == member_id)
            .ok_or_else(|| anyhow::anyhow!("unknown team member {member_id:?}"))?
            .clone();
        if member.status == TeamMemberStatus::Closed {
            anyhow::bail!("subagent {} is closed", member.name);
        }
        let from_member_id = team
            .members
            .iter()
            .find(|candidate| candidate.thread_id == *caller_thread_id)
            .map(|candidate| candidate.id.clone());
        self.teams
            .append_mailbox_message(
                team_id,
                from_member_id,
                member_id.to_string(),
                TeamMailboxMessageKind::Message,
                message,
            )
            .await?;
        let Some(turn_id) = self.active_turn_for_thread(&member.thread_id).await else {
            return Ok(None);
        };
        self.deliver_pending_team_mailbox(team_id, member_id, member.thread_id, turn_id.clone())
            .await?;
        Ok(Some(turn_id))
    }

    /// Deliver queued messages and start an idle agent, or steer its active turn.
    pub(crate) async fn followup_team_member(
        self: &Arc<Self>,
        caller_thread_id: &ThreadId,
        team_id: &str,
        member_id: &str,
        message: String,
    ) -> anyhow::Result<TurnId> {
        let _spawn_guard = self.agent_team_spawn_lock.lock().await;
        self.followup_team_member_locked(caller_thread_id, team_id, member_id, message)
            .await
    }

    async fn followup_team_member_locked(
        self: &Arc<Self>,
        caller_thread_id: &ThreadId,
        team_id: &str,
        member_id: &str,
        message: String,
    ) -> anyhow::Result<TurnId> {
        let team = self
            .read_team(team_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("unknown team {team_id:?}"))?;
        let member = team
            .members
            .iter()
            .find(|member| member.id == member_id)
            .ok_or_else(|| anyhow::anyhow!("unknown team member {member_id:?}"))?
            .clone();
        if member.status == TeamMemberStatus::Closed {
            anyhow::bail!("subagent {} is closed", member.name);
        }
        let from_member_id = team
            .members
            .iter()
            .find(|candidate| candidate.thread_id == *caller_thread_id)
            .map(|candidate| candidate.id.clone());
        self.teams
            .append_mailbox_message(
                team_id,
                from_member_id,
                member_id.to_string(),
                TeamMailboxMessageKind::NewTask,
                message,
            )
            .await?;
        if let Some(turn_id) = self.active_turn_for_thread(&member.thread_id).await {
            self.deliver_pending_team_mailbox(
                team_id,
                member_id,
                member.thread_id,
                turn_id.clone(),
            )
            .await?;
            return Ok(turn_id);
        }

        self.ensure_codex_v2_team_capacity(&team, &member.id, None)
            .await?;
        let metadata = self.load_thread_metadata(&member.thread_id).await?;
        let inherited_context = self
            .team_member_turn_contexts
            .lock()
            .await
            .get(&member.thread_id)
            .cloned();
        let workspace = inherited_context
            .as_ref()
            .map(|context| context.workspace.clone())
            .or_else(|| metadata.as_ref().map(|metadata| metadata.workspace.clone()))
            .unwrap_or_else(|| self.workspace.display().to_string());
        let member_thread_id = member.thread_id;
        let turn_id = self
            .start_turn(StartTurnRequest {
                thread_id: member_thread_id.clone(),
                message: String::new(),
                images: Vec::new(),
                provider_override: member.model_provider,
                model_override: member.model,
                reasoning_override: metadata
                    .as_ref()
                    .and_then(|metadata| metadata.selection_mode.as_ref())
                    .and_then(ModelSelectionMode::reasoning)
                    .map(str::to_string),
                workspace,
                instructions: inherited_context
                    .as_ref()
                    .map(|context| context.instructions.clone())
                    .unwrap_or_else(crate::default_instructions),
                developer_context: inherited_context
                    .as_ref()
                    .and_then(|context| context.developer_context.clone()),
                task_ledger_required: false,
            })
            .await?;
        Ok(turn_id)
    }

    pub async fn set_team_member_policy_mode(
        &self,
        team_id: &str,
        member_id: &str,
        policy_mode: PolicyMode,
    ) -> anyhow::Result<TeamState> {
        self.teams
            .set_member_policy_mode(team_id, member_id, policy_mode)
            .await
    }

    pub async fn interrupt_team_member(
        &self,
        team_id: &str,
        member_id: &str,
    ) -> anyhow::Result<Option<TurnId>> {
        let _interrupt_guard = self.agent_team_spawn_lock.lock().await;
        let team = self
            .read_team(team_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("unknown team {team_id:?}"))?;
        let member = team
            .members
            .iter()
            .find(|member| member.id == member_id)
            .ok_or_else(|| anyhow::anyhow!("unknown team member {member_id:?}"))?
            .clone();
        if member.status != TeamMemberStatus::Running {
            return Ok(None);
        }
        let Some(turn_id) = member.current_turn_id.clone() else {
            return Ok(None);
        };
        if self
            .active_turn_for_thread(&member.thread_id)
            .await
            .as_ref()
            != Some(&turn_id)
        {
            return Ok(None);
        }
        self.interrupt_turn(member.thread_id.clone(), turn_id.clone())
            .await?;
        // Make queued mailbox work immediately eligible for the replacement turn while the
        // lifecycle lock still prevents a follow-up from starting. Delivery acknowledgements
        // are turn-owned, so a late acknowledgement from the aborted turn cannot steal a
        // reservation acquired by its replacement.
        self.teams
            .release_mailbox_reservations_for_turn(&turn_id)
            .await;
        self.teams
            .update_member(team_id, member_id, |member| {
                member.status = TeamMemberStatus::Interrupted;
                member.current_turn_id = None;
                member.terminal_error = None;
            })
            .await?;
        self.emit(RoderEvent::TeamMemberCompleted(TeamMemberCompleted {
            team_id: team_id.to_string(),
            member_id: member_id.to_string(),
            member_thread_id: member.thread_id,
            turn_id: Some(turn_id.clone()),
            status: TeamMemberStatus::Interrupted,
            final_message: member.final_message,
            error: None,
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        Ok(Some(turn_id))
    }

    pub async fn close_team_member(
        &self,
        team_id: &str,
        member_id: &str,
    ) -> anyhow::Result<roder_api::teams::TeamMemberDescriptor> {
        let team = self
            .read_team(team_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("unknown team {team_id:?}"))?;
        let member = team
            .members
            .iter()
            .find(|member| member.id == member_id)
            .ok_or_else(|| anyhow::anyhow!("unknown team member {member_id:?}"))?
            .clone();
        if member.role == roder_api::teams::TeamMemberRole::Lead {
            anyhow::bail!("team lead cannot be closed as a subagent");
        }
        let interrupted_turn_id = if member.status == TeamMemberStatus::Running {
            if let Some(turn_id) = member.current_turn_id.clone() {
                self.interrupt_turn(member.thread_id.clone(), turn_id.clone())
                    .await?;
                Some(turn_id)
            } else {
                None
            }
        } else {
            member.current_turn_id.clone()
        };
        let updated = self
            .teams
            .update_member(team_id, member_id, |member| {
                member.status = TeamMemberStatus::Closed;
                member.current_turn_id = None;
            })
            .await?;
        let closed = updated
            .members
            .iter()
            .find(|member| member.id == member_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("closed team member disappeared"))?;
        self.team_member_turn_contexts
            .lock()
            .await
            .remove(&closed.thread_id);
        self.emit(RoderEvent::TeamMemberCompleted(TeamMemberCompleted {
            team_id: team_id.to_string(),
            member_id: closed.id.clone(),
            member_thread_id: closed.thread_id.clone(),
            turn_id: interrupted_turn_id,
            status: TeamMemberStatus::Closed,
            final_message: closed.final_message.clone(),
            error: closed.terminal_error.clone(),
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        Ok(closed)
    }

    pub async fn cleanup_team(&self, team_id: &str, force: bool) -> anyhow::Result<bool> {
        let Some(team) = self.read_team(team_id).await else {
            return Ok(false);
        };
        if !force
            && team
                .members
                .iter()
                .any(|member| member.status == TeamMemberStatus::Running)
        {
            anyhow::bail!("team {team_id:?} has active teammates; use forced cleanup");
        }
        if force {
            for member in team.members.iter().filter(|member| {
                member.role != roder_api::teams::TeamMemberRole::Lead
                    && member.status == TeamMemberStatus::Running
            }) {
                if let Some(turn_id) = member
                    .current_turn_id
                    .clone()
                    .or(self.active_turn_for_thread(&member.thread_id).await)
                {
                    let _ = self.interrupt_turn(member.thread_id.clone(), turn_id).await;
                }
            }
        }
        let removed = self.teams.remove(team_id).await?.is_some();
        if removed {
            let member_thread_ids = team
                .members
                .iter()
                .map(|member| member.thread_id.clone())
                .collect::<std::collections::HashSet<_>>();
            self.team_member_turn_contexts
                .lock()
                .await
                .retain(|thread_id, _| !member_thread_ids.contains(thread_id));
            self.emit(RoderEvent::TeamCleanupCompleted(TeamCleanupCompleted {
                team_id: team_id.to_string(),
                forced: force,
                timestamp: OffsetDateTime::now_utc(),
            }))
            .await;
        }
        Ok(removed)
    }

    pub async fn effective_policy_mode_for_thread(&self, thread_id: &str) -> PolicyMode {
        if let Some(mode) = self.teams.policy_mode_for_thread(thread_id).await {
            return mode;
        }
        self.status().await.policy_mode
    }

    async fn complete_team_member_turn_with_result(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        status: TeamMemberStatus,
        final_message: Option<String>,
        terminal_error: Option<String>,
    ) -> anyhow::Result<()> {
        let _delivery_guard = self.agent_team_spawn_lock.lock().await;
        self.active_turns.write().await.remove(turn_id);
        self.active_turn_selections.write().await.remove(turn_id);
        self.active_turn_contexts.write().await.remove(turn_id);
        let Some((team_id, member)) = self
            .teams
            .complete_member_turn(
                thread_id,
                turn_id,
                status,
                final_message.clone(),
                terminal_error.clone(),
            )
            .await?
        else {
            return Ok(());
        };
        if let Some(parent_thread_id) = member.parent_thread_id.as_ref()
            && let Some(team) = self.read_team(&team_id).await
            && let Some(parent) = team
                .members
                .iter()
                .find(|candidate| candidate.thread_id == *parent_thread_id)
        {
            let identity = member
                .agent_path
                .as_deref()
                .or(member.task_name.as_deref())
                .unwrap_or(&member.name);
            let mut report = format!("Agent {identity} finished with status {status:?}.");
            if let Some(message) = final_message.as_deref()
                && !message.trim().is_empty()
            {
                report.push_str("\n\nFinal result:\n");
                report.push_str(message);
            }
            if let Some(error) = terminal_error.as_deref()
                && !error.trim().is_empty()
            {
                report.push_str("\n\nTerminal error:\n");
                report.push_str(error);
            }
            let mailbox_appended = self
                .teams
                .append_mailbox_message(
                    &team_id,
                    Some(member.id.clone()),
                    parent.id.clone(),
                    TeamMailboxMessageKind::FinalAnswer,
                    report,
                )
                .await
                .is_ok();
            if mailbox_appended
                && let Some(parent_turn_id) = self.active_turn_for_thread(parent_thread_id).await
            {
                // The parent may finish between the active-turn lookup and enqueue. Its durable
                // mailbox entry remains pending for the next turn, while terminal observers must
                // still receive TeamMemberCompleted for this child.
                let _ = self
                    .deliver_pending_team_mailbox(
                        &team_id,
                        &parent.id,
                        parent_thread_id.clone(),
                        parent_turn_id,
                    )
                    .await;
            }
        }
        self.emit(RoderEvent::TeamMemberCompleted(TeamMemberCompleted {
            team_id,
            member_id: member.id,
            member_thread_id: member.thread_id,
            turn_id: Some(turn_id.clone()),
            status,
            final_message,
            error: terminal_error,
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        Ok(())
    }

    pub async fn load_thread(
        &self,
        thread_id: &ThreadId,
    ) -> anyhow::Result<Option<ThreadSnapshot>> {
        let loaded = if let Some(store) = &self.thread_store {
            store.load_thread(thread_id).await?
        } else {
            None
        };
        if loaded.is_some() {
            self.emit(RoderEvent::ThreadLoaded(ThreadLoaded {
                thread_id: thread_id.clone(),
                timestamp: OffsetDateTime::now_utc(),
            }))
            .await;
        }
        Ok(loaded)
    }

    pub async fn workspace_for_thread(&self, thread_id: &ThreadId) -> anyhow::Result<String> {
        if let Some(store) = &self.thread_store {
            let snapshot = store
                .load_thread(thread_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"));
            match snapshot {
                Ok(snapshot) => {
                    if let Some(metadata) = snapshot.metadata {
                        // Fork-backed threads fail closed before any write
                        // when their workspace was removed out-of-band.
                        if let Some(fork) = &metadata.workspace_fork
                            && fork.status == roder_api::forks::ForkStatus::Active
                            && !std::path::Path::new(&metadata.workspace).is_dir()
                        {
                            anyhow::bail!(
                                "workspace fork {} is missing its workspace at {}; restore it or \
                                 remove the fork before running turns in this thread",
                                fork.id,
                                metadata.workspace
                            );
                        }
                        return Ok(metadata.workspace);
                    }
                    eprintln!(
                        "thread metadata missing while resolving workspace for {thread_id}; falling back to runtime workspace"
                    );
                }
                Err(err) => {
                    eprintln!(
                        "thread missing while resolving workspace for {thread_id}: {err}; falling back to runtime workspace"
                    );
                }
            }
        }
        Ok(self.workspace.display().to_string())
    }

    async fn selection_mode_for_thread(
        &self,
        thread_id: &ThreadId,
    ) -> anyhow::Result<Option<ModelSelectionMode>> {
        let Some(store) = &self.thread_store else {
            return Ok(None);
        };
        Ok(store
            .load_thread(thread_id)
            .await?
            .and_then(|snapshot| snapshot.metadata)
            .and_then(|metadata| {
                metadata
                    .selection_mode
                    .or_else(|| match (metadata.provider, metadata.model) {
                        (Some(provider), Some(model)) => {
                            Some(ModelSelectionMode::manual(provider, model, None))
                        }
                        _ => None,
                    })
            }))
    }

    /// Per-thread tool allowlist, developer instructions, and external tools persisted at thread creation.
    pub(crate) async fn thread_turn_overrides(
        &self,
        thread_id: &ThreadId,
    ) -> anyhow::Result<ThreadTurnOverrides> {
        let Some(store) = &self.thread_store else {
            return Ok(ThreadTurnOverrides::default());
        };
        Ok(store
            .load_thread_metadata(thread_id)
            .await?
            .map(|metadata| ThreadTurnOverrides {
                tool_allowlist: metadata.tool_allowlist,
                developer_instructions: metadata.developer_instructions,
                external_tools: metadata.external_tools,
            })
            .unwrap_or_default())
    }

    pub async fn set_thread_selection_mode(
        &self,
        thread_id: &ThreadId,
        selection_mode: ModelSelectionMode,
    ) -> anyhow::Result<()> {
        let Some(store) = &self.thread_store else {
            return Ok(());
        };
        let Some(snapshot) = store.load_thread(thread_id).await? else {
            anyhow::bail!("thread not found: {thread_id}");
        };
        let Some(mut metadata) = snapshot.metadata else {
            return Ok(());
        };
        let concrete = selection_mode.concrete_selection();
        metadata.provider = Some(concrete.provider);
        metadata.model = Some(concrete.model);
        metadata.selection_mode = Some(selection_mode);
        metadata.updated_at = OffsetDateTime::now_utc();
        store.update_thread_metadata(metadata).await?;
        Ok(())
    }

    async fn runner_session_for_thread(
        &self,
        thread_id: &ThreadId,
    ) -> anyhow::Result<Option<(RunnerDestination, Arc<dyn RemoteRunnerSession>)>> {
        let metadata = if let Some(store) = &self.thread_store {
            store.load_thread_metadata(thread_id).await?
        } else {
            None
        };
        // An explicit per-thread binding wins over the runtime-level destination.
        let destination = metadata
            .as_ref()
            .and_then(|metadata| metadata.runner_binding.as_ref())
            .map(|binding| binding.destination.clone())
            .or(self.config.read().await.remote_runner_destination.clone());
        let Some(destination) = destination else {
            return Ok(None);
        };
        let provider = self
            .registry
            .remote_runner_providers
            .iter()
            .find(|provider| provider.id() == destination.provider_id)
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "remote runner provider {:?} is not installed",
                    destination.provider_id
                )
            })?;
        let persisted_state = metadata.and_then(|metadata| metadata.runner_state);
        let session = if let Some(state) = persisted_state
            && state.provider_id == destination.provider_id
            && state.destination_id == destination.id
        {
            match provider.resume_session(state).await {
                Ok(session) => session,
                Err(_) => provider.create_session(destination.clone()).await?,
            }
        } else {
            provider.create_session(destination.clone()).await?
        };
        Ok(Some((destination, session)))
    }

    /**
     * Remote workspace for tool execution on a runner-bound thread. `None`
     * for threads without an explicit binding, including threads on a
     * runtime-level `runners/select` destination — those keep local tools.
     */
    pub(crate) async fn remote_workspace_for_thread(
        &self,
        thread_id: &ThreadId,
    ) -> anyhow::Result<Option<Arc<RemoteWorkspace>>> {
        let Some(store) = &self.thread_store else {
            return Ok(None);
        };
        let binding = store
            .load_thread_metadata(thread_id)
            .await?
            .and_then(|metadata| metadata.runner_binding);
        let Some(binding) = binding else {
            return Ok(None);
        };
        let session = self
            .runner_session_for_thread(thread_id)
            .await?
            .map(|(_, session)| session)
            .ok_or_else(|| {
                anyhow::anyhow!("runner-bound thread {thread_id} has no runner session")
            })?;
        Ok(Some(Arc::new(RemoteWorkspace {
            session,
            root: binding.workspace,
            read_roots: binding.read_roots,
        })))
    }

    async fn persist_runner_state(
        &self,
        thread_id: &ThreadId,
        runner: Option<&(RunnerDestination, Arc<dyn RemoteRunnerSession>)>,
    ) -> anyhow::Result<()> {
        let Some((destination, session)) = runner else {
            return Ok(());
        };
        let Some(store) = &self.thread_store else {
            return Ok(());
        };
        let Some(snapshot) = store.load_thread(thread_id).await? else {
            return Ok(());
        };
        let Some(mut metadata) = snapshot.metadata else {
            return Ok(());
        };
        metadata.runner_destination = Some(destination.clone());
        metadata.runner_state = Some(session.state());
        metadata.updated_at = OffsetDateTime::now_utc();
        store.update_thread_metadata(metadata).await?;
        Ok(())
    }

    fn remote_runner_provider_by_id(
        &self,
        provider_id: &str,
    ) -> Option<Arc<dyn RemoteRunnerProvider>> {
        self.registry
            .remote_runner_providers
            .iter()
            .find(|provider| provider.id() == provider_id)
            .cloned()
    }

    /// Resolve the live runner session for a thread along with its provider,
    /// failing clearly when the thread is not runner-bound.
    async fn thread_runner_session(
        &self,
        thread_id: &ThreadId,
    ) -> anyhow::Result<(
        RunnerDestination,
        Arc<dyn RemoteRunnerProvider>,
        Arc<dyn RemoteRunnerSession>,
    )> {
        let Some((destination, session)) = self.runner_session_for_thread(thread_id).await? else {
            anyhow::bail!("thread {thread_id} is not bound to a remote runner");
        };
        let provider = self
            .remote_runner_provider_by_id(&destination.provider_id)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "remote runner provider {:?} is not installed",
                    destination.provider_id
                )
            })?;
        Ok((destination, provider, session))
    }

    /// Pause a runner-bound thread's session toward standby and persist the
    /// post-pause state. Errors if the provider is not pausable.
    pub async fn pause_thread_runner(
        &self,
        thread_id: &ThreadId,
    ) -> anyhow::Result<RunnerSessionState> {
        let (destination, provider, session) = self.thread_runner_session(thread_id).await?;
        anyhow::ensure!(
            provider.capabilities().pausable,
            "remote runner provider {:?} does not support pausing",
            destination.provider_id
        );
        let state = session.pause().await?;
        self.persist_runner_state(thread_id, Some(&(destination, session)))
            .await?;
        Ok(state)
    }

    /// Resume (wake) a runner-bound thread's paused session and persist state.
    pub async fn resume_thread_runner(
        &self,
        thread_id: &ThreadId,
    ) -> anyhow::Result<RunnerSessionState> {
        let (destination, _provider, session) = self.thread_runner_session(thread_id).await?;
        let state = session.resume().await?;
        self.persist_runner_state(thread_id, Some(&(destination, session)))
            .await?;
        Ok(state)
    }

    /// Detach a runner-bound thread's session: persist the durable, rejoinable
    /// state and leave the remote sandbox alive. Errors if not detachable.
    pub async fn detach_thread_runner(
        &self,
        thread_id: &ThreadId,
    ) -> anyhow::Result<RunnerSessionState> {
        let (destination, provider, session) = self.thread_runner_session(thread_id).await?;
        anyhow::ensure!(
            provider.capabilities().detachable,
            "remote runner provider {:?} does not support detaching",
            destination.provider_id
        );
        let state = session.detach().await?;
        // Persist the detached state explicitly so a later turn or process
        // rejoins the same sandbox instead of provisioning a new one.
        if let Some(store) = &self.thread_store
            && let Some(snapshot) = store.load_thread(thread_id).await?
            && let Some(mut metadata) = snapshot.metadata
        {
            metadata.runner_destination = Some(destination);
            metadata.runner_state = Some(state.clone());
            metadata.updated_at = OffsetDateTime::now_utc();
            store.update_thread_metadata(metadata).await?;
        }
        Ok(state)
    }

    /// Rejoin a previously created sandbox from a thread's persisted runner
    /// state without provisioning a new one. An optional `sandbox` overrides the
    /// persisted sandbox name (recovery by name). Persists refreshed state.
    pub async fn rejoin_thread_runner(
        &self,
        thread_id: &ThreadId,
        sandbox: Option<String>,
    ) -> anyhow::Result<RunnerSessionState> {
        let store = self
            .thread_store
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("thread store is required to rejoin a runner"))?;
        let metadata = store
            .load_thread_metadata(thread_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread {thread_id} has no metadata"))?;
        let destination = metadata
            .runner_binding
            .as_ref()
            .map(|binding| binding.destination.clone())
            .or_else(|| metadata.runner_destination.clone())
            .ok_or_else(|| anyhow::anyhow!("thread {thread_id} is not bound to a remote runner"))?;
        let mut state = metadata
            .runner_state
            .clone()
            .ok_or_else(|| anyhow::anyhow!("thread {thread_id} has no persisted runner state"))?;
        if let Some(sandbox) = sandbox
            && let Some(object) = state.metadata.as_object_mut()
        {
            object.insert("sandbox_name".to_string(), serde_json::Value::from(sandbox));
        }
        let provider = self
            .remote_runner_provider_by_id(&destination.provider_id)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "remote runner provider {:?} is not installed",
                    destination.provider_id
                )
            })?;
        let session = provider.rejoin_session(state).await?;
        self.persist_runner_state(thread_id, Some(&(destination, session.clone())))
            .await?;
        Ok(session.state())
    }

    async fn record_thread_usage_metadata(
        &self,
        thread_id: &ThreadId,
        usage: &TokenUsage,
    ) -> anyhow::Result<()> {
        if usage.is_empty() {
            return Ok(());
        }
        let Some(store) = &self.thread_store else {
            return Ok(());
        };
        let Some(snapshot) = store.load_thread(thread_id).await? else {
            return Ok(());
        };
        let Some(mut metadata) = snapshot.metadata else {
            return Ok(());
        };
        metadata
            .usage
            .get_or_insert_with(ThreadUsageMetadata::default)
            .add_token_usage(usage);
        metadata.updated_at = OffsetDateTime::now_utc();
        store.update_thread_metadata(metadata).await?;
        Ok(())
    }

    pub fn start_turn(
        self: &Arc<Self>,
        mut req: StartTurnRequest,
    ) -> BoxFuture<'_, anyhow::Result<TurnId>> {
        Box::pin(async move {
            req.workspace = validate_thread_workspace(&req.workspace)?;
            let team_member = self.teams.member_for_thread(&req.thread_id).await;
            let cfg = self.config.read().await.clone();
            let provider = req
                .provider_override
                .clone()
                .unwrap_or_else(|| cfg.default_provider.clone());
            self.engine_for(&provider)?;
            let turn_id = uuid::Uuid::new_v4().to_string();
            let mut initial_mailbox_ack = None;
            if let Some((team_id, member)) = &team_member {
                let pending = self
                    .teams
                    .reserve_pending_mailbox_messages(team_id, &member.id, &turn_id)
                    .await?;
                if !pending.is_empty()
                    && let Some(team) = self.read_team(team_id).await
                {
                    let mailbox = format_mailbox_messages(&team, &pending);
                    req.message = if req.message.trim().is_empty() {
                        mailbox
                    } else {
                        format!("{mailbox}\n\n[Direct task input]\n{}", req.message)
                    };
                    initial_mailbox_ack = Some(MailboxDeliveryAck {
                        team_id: team_id.clone(),
                        message_ids: pending.iter().map(|message| message.id.clone()).collect(),
                    });
                }
            }
            let (abort_handle, abort_registration) = AbortHandle::new_pair();
            let active = ActiveTurnHandle {
                thread_id: req.thread_id.clone(),
                abort: abort_handle,
                steers: Arc::new(Mutex::new(Vec::new())),
            };
            self.active_turns
                .write()
                .await
                .insert(turn_id.clone(), active);
            self.active_turn_contexts.write().await.insert(
                turn_id.clone(),
                InheritedTurnContext {
                    workspace: req.workspace.clone(),
                    instructions: req.instructions.clone(),
                    developer_context: req.developer_context.clone(),
                },
            );
            if let Some((team_id, member)) = team_member {
                let updated = match self
                    .teams
                    .update_member(&team_id, &member.id, |member| {
                        member.current_turn_id = Some(turn_id.clone());
                        member.status = TeamMemberStatus::Running;
                        member.final_message = None;
                        member.terminal_error = None;
                    })
                    .await
                {
                    Ok(updated) => updated,
                    Err(error) => {
                        self.active_turns.write().await.remove(&turn_id);
                        self.active_turn_contexts.write().await.remove(&turn_id);
                        self.teams
                            .release_mailbox_reservations_for_turn(&turn_id)
                            .await;
                        return Err(error);
                    }
                };
                if let Some(member) = updated
                    .members
                    .into_iter()
                    .find(|candidate| candidate.id == member.id)
                {
                    self.emit(RoderEvent::TeamMemberStatusChanged(
                        TeamMemberStatusChanged {
                            team_id,
                            member_id: member.id,
                            member_thread_id: member.thread_id,
                            status: TeamMemberStatus::Running,
                            timestamp: OffsetDateTime::now_utc(),
                        },
                    ))
                    .await;
                }
            }
            let runtime = Arc::clone(self);
            let turn_req = req;
            let thread_id_for_task = turn_req.thread_id.clone();
            let turn_id_for_task = turn_id.clone();
            tokio::spawn(async move {
                let result = Abortable::new(
                    runtime.run_turn(turn_req, turn_id_for_task.clone(), initial_mailbox_ack),
                    abort_registration,
                )
                .await;
                /*
                 * A failed sibling in a parallel tool batch drops in-flight external tool
                 * futures (`try_join_all` in `route_tool_calls`), stranding their
                 * `pending_external_tool_calls` entries. Sweep before reporting the turn
                 * outcome so every `thread/toolExecutionRequested` gets a terminal
                 * resolution; on clean completion the map holds nothing for this turn.
                 */
                runtime
                    .cancel_pending_external_tool_calls_for_turn(&turn_id_for_task)
                    .await;
                let completed = matches!(&result, Ok(Ok(TurnRunOutcome::Completed)));
                match &result {
                    Ok(Err(err)) => {
                        // run_turn emits failures after the stream starts; this covers setup/startup errors.
                        runtime
                            .emit(RoderEvent::TurnFailed(TurnFailed {
                                thread_id: thread_id_for_task.clone(),
                                turn_id: turn_id_for_task.clone(),
                                error: err.to_string(),
                                error_kind: None,
                                usage: None,
                                timestamp: OffsetDateTime::now_utc(),
                            }))
                            .await;
                        let _ = runtime
                            .complete_team_member_turn_with_result(
                                &thread_id_for_task,
                                &turn_id_for_task,
                                TeamMemberStatus::Failed,
                                None,
                                Some(err.to_string()),
                            )
                            .await;
                    }
                    Ok(Ok(TurnRunOutcome::Stopped)) => {
                        let _ = runtime
                            .complete_team_member_turn_with_result(
                                &thread_id_for_task,
                                &turn_id_for_task,
                                TeamMemberStatus::Failed,
                                None,
                                Some("turn stopped before completion".to_string()),
                            )
                            .await;
                    }
                    Err(_) => {
                        let _ = runtime
                            .complete_team_member_turn_with_result(
                                &thread_id_for_task,
                                &turn_id_for_task,
                                TeamMemberStatus::Interrupted,
                                None,
                                None,
                            )
                            .await;
                    }
                    Ok(Ok(TurnRunOutcome::Completed)) => {}
                }
                runtime
                    .teams
                    .release_mailbox_reservations_for_turn(&turn_id_for_task)
                    .await;
                runtime.active_turns.write().await.remove(&turn_id_for_task);
                runtime
                    .active_turn_selections
                    .write()
                    .await
                    .remove(&turn_id_for_task);
                runtime
                    .active_turn_contexts
                    .write()
                    .await
                    .remove(&turn_id_for_task);
                if completed {
                    let _ = runtime
                        .continue_active_goal_after_turn(thread_id_for_task)
                        .await;
                }
            });
            Ok(turn_id)
        })
    }

    pub(crate) async fn has_active_turn_for_thread(&self, thread_id: &ThreadId) -> bool {
        self.active_turns
            .read()
            .await
            .values()
            .any(|handle| &handle.thread_id == thread_id)
    }

    async fn ensure_codex_v2_team_capacity(
        &self,
        team: &TeamState,
        resuming_member_id: &str,
        candidate_model: Option<&str>,
    ) -> anyhow::Result<()> {
        if !is_codex_v2_team(team)
            && !candidate_model
                .is_some_and(|model| model_supports_reasoning_effort(model, REASONING_ULTRA))
        {
            return Ok(());
        }
        let active_thread_ids = self
            .active_turns
            .read()
            .await
            .values()
            .map(|handle| handle.thread_id.clone())
            .collect::<std::collections::HashSet<_>>();
        let resident_threads = 1 + team
            .members
            .iter()
            .filter(|member| {
                member.role != roder_api::teams::TeamMemberRole::Lead
                    && member.id != resuming_member_id
                    && active_thread_ids.contains(&member.thread_id)
            })
            .count();
        anyhow::ensure!(
            resident_threads < codex_v2::CODEX_V2_MAX_RESIDENT_TEAM_THREADS,
            "agent thread limit reached for this Codex V2 team: maximum {} resident threads (the lead plus 3 running subagents)",
            codex_v2::CODEX_V2_MAX_RESIDENT_TEAM_THREADS
        );
        Ok(())
    }

    /// Number of currently running turns across all threads. Hosted runtime
    /// pools use this to avoid evicting tenants with active work.
    pub async fn active_turn_count(&self) -> usize {
        self.active_turns.read().await.len()
    }

    pub async fn active_turn_for_thread(&self, thread_id: &ThreadId) -> Option<TurnId> {
        self.active_turns
            .read()
            .await
            .iter()
            .find_map(|(turn_id, handle)| (&handle.thread_id == thread_id).then(|| turn_id.clone()))
    }

    pub async fn thread_activity(&self, thread_id: &ThreadId) -> ThreadActivity {
        let Some(active_turn_id) = self.active_turn_for_thread(thread_id).await else {
            return ThreadActivity::default();
        };

        let mut active_flags = Vec::new();
        {
            let pending_approvals = self.pending_tool_approvals.lock().await;
            if pending_approvals
                .values()
                .any(|pending| &pending.thread_id == thread_id && pending.turn_id == active_turn_id)
            {
                active_flags.push("approvalRequired".to_string());
            }
        }
        {
            let pending_inputs = self.pending_user_inputs.lock().await;
            if pending_inputs
                .values()
                .any(|pending| &pending.thread_id == thread_id && pending.turn_id == active_turn_id)
            {
                active_flags.push("userInputRequired".to_string());
            }
        }
        {
            let pending_external = self.pending_external_tool_calls.lock().await;
            if pending_external
                .values()
                .any(|pending| &pending.thread_id == thread_id && pending.turn_id == active_turn_id)
            {
                active_flags.push("externalToolPending".to_string());
            }
        }
        if self.pending_plan_exit().await.is_some_and(|pending| {
            &pending.thread_id == thread_id && pending.turn_id == active_turn_id
        }) {
            active_flags.push("planExitRequired".to_string());
        }

        ThreadActivity {
            active_turn_id: Some(active_turn_id),
            active_flags,
        }
    }

    pub async fn interrupt_turn(&self, thread_id: ThreadId, turn_id: TurnId) -> anyhow::Result<()> {
        if let Some(handle) = self.active_turns.write().await.remove(&turn_id) {
            handle.abort.abort();
        }
        self.active_turn_selections.write().await.remove(&turn_id);
        self.cancel_pending_external_tool_calls_for_turn(&turn_id)
            .await;
        self.emit(RoderEvent::TurnInterrupted(TurnInterrupted {
            thread_id,
            turn_id,
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        Ok(())
    }

    pub async fn steer_turn(
        &self,
        thread_id: ThreadId,
        turn_id: TurnId,
        message: String,
        images: Vec<InputImage>,
    ) -> anyhow::Result<()> {
        self.enqueue_turn_steer(thread_id, turn_id, message, images, None)
            .await
    }

    pub(crate) async fn has_pending_turn_steers(&self, turn_id: &TurnId) -> bool {
        let active = self.active_turns.read().await.get(turn_id).cloned();
        let Some(active) = active else {
            return false;
        };
        let has_pending = !active.steers.lock().await.is_empty();
        has_pending
    }

    async fn steer_turn_with_mailbox_ack(
        &self,
        thread_id: ThreadId,
        turn_id: TurnId,
        message: String,
        message_ids: Vec<String>,
        team_id: TeamId,
    ) -> anyhow::Result<()> {
        self.enqueue_turn_steer(
            thread_id,
            turn_id,
            message,
            Vec::new(),
            Some(MailboxDeliveryAck {
                team_id,
                message_ids,
            }),
        )
        .await
    }

    async fn deliver_pending_team_mailbox(
        &self,
        team_id: &str,
        member_id: &str,
        member_thread_id: ThreadId,
        turn_id: TurnId,
    ) -> anyhow::Result<()> {
        let pending = self
            .teams
            .reserve_pending_mailbox_messages(team_id, member_id, &turn_id)
            .await?;
        if pending.is_empty() {
            return Ok(());
        }
        let message_ids = pending
            .iter()
            .map(|message| message.id.clone())
            .collect::<Vec<_>>();
        let team = self
            .read_team(team_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("unknown team {team_id:?}"))?;
        if let Err(error) = self
            .steer_turn_with_mailbox_ack(
                member_thread_id,
                turn_id.clone(),
                format_mailbox_messages(&team, &pending),
                message_ids.clone(),
                team_id.to_string(),
            )
            .await
        {
            self.teams
                .release_mailbox_reservations(&turn_id, &message_ids)
                .await;
            return Err(error);
        }
        Ok(())
    }

    async fn enqueue_turn_steer(
        &self,
        thread_id: ThreadId,
        turn_id: TurnId,
        message: String,
        images: Vec<InputImage>,
        mailbox_ack: Option<MailboxDeliveryAck>,
    ) -> anyhow::Result<()> {
        let message = message.trim().to_string();
        if message.is_empty() && images.is_empty() {
            return Ok(());
        }

        let Some(active) = self.active_turns.read().await.get(&turn_id).cloned() else {
            anyhow::bail!("no active turn to steer");
        };
        active.steers.lock().await.push(QueuedTurnSteer {
            message: UserMessage::with_images(message.clone(), images),
            mailbox_ack,
        });
        self.emit(RoderEvent::TurnSteered(TurnSteered {
            thread_id,
            turn_id,
            message,
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        Ok(())
    }

    pub async fn tool_specs(&self) -> Vec<roder_api::tools::ToolSpec> {
        let cfg = self.config.read().await;
        let model_profile =
            model_profile_for_provider_model(&cfg, &cfg.default_provider, &cfg.default_model);
        self.filtered_tool_specs(&cfg, &cfg.default_model, model_profile.as_ref(), &[], &[])
    }

    pub fn subagent_definitions(&self) -> Vec<SubagentDefinition> {
        self.registry
            .subagent_dispatchers
            .iter()
            .flat_map(|dispatcher| dispatcher.definitions())
            .collect()
    }

    async fn run_turn(
        self: &Arc<Self>,
        req: StartTurnRequest,
        turn_id: TurnId,
        initial_mailbox_ack: Option<MailboxDeliveryAck>,
    ) -> anyhow::Result<TurnRunOutcome> {
        let turn_started_at = OffsetDateTime::now_utc();
        self.emit(RoderEvent::TurnStarted(TurnStarted {
            thread_id: req.thread_id.clone(),
            turn_id: turn_id.clone(),
            runtime_profile: self.config.read().await.runtime_profile,
            timestamp: turn_started_at,
        }))
        .await;
        self.persist_turn_item(
            &req.thread_id,
            &turn_id,
            &TranscriptItem::UserMessage(UserMessage::with_images(
                req.message.clone(),
                req.images.clone(),
            )),
        )
        .await?;
        if let Some(ack) = initial_mailbox_ack {
            self.teams
                .mark_mailbox_messages_delivered(&ack.team_id, &turn_id, &ack.message_ids)
                .await?;
        }

        let mut cfg = self.config.read().await.clone();
        let runtime_profile = cfg.runtime_profile;
        let turn_deadline = turn_deadline_for_config(&cfg);
        let deadline_finalization_reserve =
            crate::deadline_policy::finalization_reserve_seconds(cfg.turn_deadline_seconds);
        let selection_mode = self.selection_mode_for_thread(&req.thread_id).await?;
        let concrete_selection = selection_mode
            .as_ref()
            .map(ModelSelectionMode::concrete_selection);
        let default_provider = req
            .provider_override
            .clone()
            .or_else(|| {
                concrete_selection
                    .as_ref()
                    .map(|selection| selection.provider.clone())
            })
            .unwrap_or(cfg.default_provider.clone());
        let default_model = req
            .model_override
            .clone()
            .or_else(|| {
                concrete_selection
                    .as_ref()
                    .map(|selection| selection.model.clone())
            })
            .unwrap_or(cfg.default_model.clone());
        if let Some(reasoning) = req.reasoning_override.as_deref().or_else(|| {
            selection_mode
                .as_ref()
                .and_then(ModelSelectionMode::reasoning)
        }) {
            validate_reasoning_effort(&default_model, reasoning)?;
            cfg.reasoning = Some(reasoning.to_string());
        }
        let turn_has_concrete_model_override =
            req.provider_override.is_some() || req.model_override.is_some();
        let (turn_inference_router, turn_inference_router_profile) = match &selection_mode {
            Some(ModelSelectionMode::Auto {
                router_id, profile, ..
            }) if !turn_has_concrete_model_override => (
                RuntimeInferenceRouterConfig {
                    enabled: true,
                    router_id: Some(router_id.clone()),
                },
                profile.clone(),
            ),
            _ => (RuntimeInferenceRouterConfig::disabled(), None),
        };
        let mut provider = default_provider.clone();
        let mut model = default_model.clone();
        let mut model_profile = model_profile_for_provider_model(&cfg, &provider, &model);
        let workspace = req.workspace.clone();
        let mut transcript = self.transcript_for_turn(&req, &turn_id, &model).await?;
        let mut compacted_this_turn = transcript
            .iter()
            .any(crate::compaction::is_compaction_boundary);
        let runner_session = self.runner_session_for_thread(&req.thread_id).await?;
        let effective_policy_mode = self.effective_policy_mode_for_thread(&req.thread_id).await;
        let agent_swarm_mode_active = self
            .effective_agent_swarm_mode_for_thread(&req.thread_id)
            .await;
        let thread_overrides = self.thread_turn_overrides(&req.thread_id).await?;
        let mut final_assistant_text = String::new();
        let mut final_phase_messages = Vec::<AssistantMessage>::new();
        let mut final_reasoning_text = String::new();
        let mut final_provider_metadata = None;
        let mut exhausted_tool_rounds = true;
        let mut verification_gate =
            VerificationGateState::new(req.message.clone(), runtime_profile);
        let mut speed_policy = SpeedPolicyState::default();
        let mut reliability = TurnReliabilityState::default();
        let mut turn_usage = TokenUsage::default();
        // Overwritten on every inference step's Completed event so only the
        // terminal step's stop reason survives (mid-turn tool_use steps must
        // not leak onto turn/completed).
        let mut turn_finish_reason: Option<String> = None;
        let mut deadline_finalization_requested = false;
        let mut deadline_scoreable_completion_requested = false;
        let mut task_ledger_completion_reminders = 0_u8;
        let mut task_ledger_scoreable_checkpoints = 0_u8;
        let mut empty_tool_call_nudges_used = 0_u32;
        let mut provider_stream_retry_attempts = 0_u32;
        let mut routing_candidates = None;
        let routing_transcript_start = transcript.len().saturating_sub(1);
        let mut routing_escalations = 0_u32;
        let mut model_switch_summary_selection = None::<ModelSelection>;

        'tool_rounds: for round_index in 0..MAX_TOOL_ROUNDS_PER_TURN {
            if let Some(deadline) = turn_deadline
                && deadline_expired(deadline)
            {
                self.fail_turn_due_to_deadline(&req.thread_id, &turn_id, deadline, &transcript)
                    .await?;
                return Ok(TurnRunOutcome::Stopped);
            }
            let steers = self.drain_turn_steers(&turn_id).await;
            self.append_steers(&req, &turn_id, &mut transcript, steers)
                .await?;
            if runtime_profile == RuntimeProfile::Eval
                && let Some(remaining) = crate::deadline_policy::should_start_finalization(
                    turn_deadline,
                    deadline_finalization_reserve,
                    deadline_finalization_requested || deadline_scoreable_completion_requested,
                )
            {
                if req.task_ledger_required
                    && task_ledger_completion_reminders < TASK_LEDGER_COMPLETION_REMINDER_LIMIT
                    && let Some(prompt) = task_ledger_completion_prompt(&transcript)
                {
                    task_ledger_completion_reminders += 1;
                    deadline_scoreable_completion_requested = true;
                    let item = TranscriptItem::UserMessage(UserMessage::text(
                        task_ledger_deadline_completion_prompt(
                            remaining,
                            deadline_finalization_reserve,
                            &prompt,
                        ),
                    ));
                    self.persist_turn_item(&req.thread_id, &turn_id, &item)
                        .await?;
                    transcript.push(item);
                    continue 'tool_rounds;
                } else {
                    self.start_deadline_finalization(
                        &req.thread_id,
                        &turn_id,
                        &mut transcript,
                        remaining,
                    )
                    .await?;
                    deadline_finalization_requested = true;
                }
            }
            if runtime_profile == RuntimeProfile::Eval
                && req.task_ledger_required
                && !deadline_finalization_requested
                && task_ledger_scoreable_checkpoints < TASK_LEDGER_SCOREABLE_CHECKPOINT_LIMIT
                && let Some(remaining) = deadline_remaining_seconds(turn_deadline)
                && remaining <= TASK_LEDGER_SCOREABLE_CHECKPOINT_SECONDS
                && remaining > deadline_finalization_reserve
                && let Some(prompt) = task_ledger_completion_prompt(&transcript)
            {
                task_ledger_scoreable_checkpoints += 1;
                let item = TranscriptItem::UserMessage(UserMessage::text(
                    task_ledger_scoreable_checkpoint_prompt(remaining, &prompt),
                ));
                self.persist_turn_item(&req.thread_id, &turn_id, &item)
                    .await?;
                transcript.push(item);
                continue 'tool_rounds;
            }
            if turn_inference_router.is_active() && routing_candidates.is_none() {
                routing_candidates =
                    Some(collect_inference_routing_candidates(&self.registry).await);
            }
            let routing_tools_model = model.clone();
            let routing_tools = self.filtered_tool_specs(
                &cfg,
                &model,
                model_profile.as_ref(),
                &thread_overrides.tool_allowlist,
                &thread_overrides.external_tools,
            );
            let prior_failures =
                transcript_failure_count_since(&transcript, routing_transcript_start)
                    .max(reliability.tool_failure_count())
                    .saturating_add(provider_stream_retry_attempts);
            let routing_selection = route_inference_selection(
                &self.registry,
                &turn_inference_router,
                InferenceRoutingRequest {
                    thread_id: &req.thread_id,
                    turn_id: &turn_id,
                    round_index: round_index as u32,
                    runtime_profile,
                    phase: speed_policy.phase(),
                    profile: turn_inference_router_profile.as_deref(),
                    default_selection: ModelSelection {
                        provider: default_provider.clone(),
                        model: default_model.clone(),
                    },
                    transcript: &transcript,
                    tools: &routing_tools,
                    candidates: routing_candidates.as_deref(),
                    prior_failures,
                    prior_escalations: routing_escalations,
                },
            )
            .await;
            if let Some(decision) = routing_selection.decision.clone() {
                if matches!(decision.outcome, InferenceRoutingOutcome::Escalated) {
                    routing_escalations = routing_escalations.saturating_add(1);
                }
                self.emit(RoderEvent::InferenceRoutingDecision(
                    InferenceRoutingDecisionEvent {
                        thread_id: req.thread_id.clone(),
                        turn_id: turn_id.clone(),
                        round_index: round_index as u32,
                        default_selection: ModelSelection {
                            provider: default_provider.clone(),
                            model: default_model.clone(),
                        },
                        selected_selection: routing_selection.selection.clone(),
                        decision,
                        timestamp: OffsetDateTime::now_utc(),
                    },
                ))
                .await;
            }
            provider = routing_selection.selection.provider.clone();
            model = routing_selection.selection.model.clone();
            let engine = self.engine_for(&provider)?;
            let capabilities = engine.capabilities();
            model_profile = model_profile_for_provider_model(&cfg, &provider, &model);
            let tools = if capabilities.tool_calls {
                if model == routing_tools_model {
                    routing_tools.clone()
                } else {
                    self.filtered_tool_specs(
                        &cfg,
                        &model,
                        model_profile.as_ref(),
                        &thread_overrides.tool_allowlist,
                        &thread_overrides.external_tools,
                    )
                }
            } else {
                Vec::new()
            };
            let parallel_tool_calls = parallel_tool_calls_for_model(&cfg, &model);
            let tool_choice = if tools.is_empty() {
                ToolChoice::None
            } else {
                ToolChoice::Auto
            };
            let summary_selection = ModelSelection {
                provider: provider.clone(),
                model: model.clone(),
            };
            if model_switch_summary_selection.as_ref() != Some(&summary_selection) {
                if let Some(summary) = model_switch_summary(
                    &transcript,
                    model_profile.as_ref(),
                    &provider,
                    &model,
                    &tools,
                ) {
                    let item = TranscriptItem::UserMessage(UserMessage::text(summary));
                    self.persist_turn_item(&req.thread_id, &turn_id, &item)
                        .await?;
                    transcript.push(item);
                }
                model_switch_summary_selection = Some(summary_selection);
            }

            if !capabilities.image_input && transcript_has_images(&transcript) {
                self.fail_turn_with_error(
                    &req.thread_id,
                    &turn_id,
                    format!("provider {provider} does not support image input"),
                )
                .await?;
                return Ok(TurnRunOutcome::Stopped);
            }
            transcript = self
                .compact_transcript_if_needed(
                    &req.thread_id,
                    &turn_id,
                    &provider,
                    &model,
                    transcript,
                    self.compaction_options_for_turn(&req.thread_id, !compacted_this_turn),
                )
                .await?;
            compacted_this_turn = compacted_this_turn
                || transcript
                    .iter()
                    .any(crate::compaction::is_compaction_boundary);

            let speed_policy_decision =
                speed_policy.decision(runtime_profile, &model, &cfg.speed_policy);
            let request_reasoning = reasoning_from_decision(
                speed_policy_decision.as_ref(),
                routing_selection
                    .reasoning
                    .clone()
                    .unwrap_or_else(|| reasoning_for_model(&cfg, &model)),
            );
            self.active_turn_selections.write().await.insert(
                turn_id.clone(),
                ModelSelectionMode::manual(
                    provider.clone(),
                    model.clone(),
                    request_reasoning.level.clone(),
                ),
            );
            if let Some(limit) = reliability.record_model_call(
                &cfg.reliability,
                runtime_profile == RuntimeProfile::Interactive,
            ) {
                self.fail_turn_due_to_reliability_limit(
                    &req.thread_id,
                    &turn_id,
                    &provider,
                    &model,
                    limit,
                    &transcript,
                )
                .await?;
                return Ok(TurnRunOutcome::Stopped);
            }
            self.emit(RoderEvent::InferenceStarted(InferenceStarted {
                thread_id: req.thread_id.clone(),
                turn_id: turn_id.clone(),
                engine_id: engine.id(),
                model: ModelSelection {
                    provider: provider.clone(),
                    model: model.clone(),
                },
                reasoning: request_reasoning.clone(),
                speed_policy: speed_policy_decision.clone(),
                deadline_remaining_seconds: deadline_remaining_seconds(turn_deadline),
                timestamp: OffsetDateTime::now_utc(),
            }))
            .await;

            let mut instructions = req.instructions.clone();
            if let Some(extra) = &thread_overrides.developer_instructions {
                instructions = apply_thread_developer_instructions(instructions, extra);
            }
            if let Some(context) = req.developer_context.as_deref() {
                instructions = apply_turn_developer_context(instructions, context);
            }
            let mut instructions = apply_runtime_profile(instructions, runtime_profile);
            if let Some(profile) = &model_profile {
                instructions = apply_model_instruction_overlay(instructions, profile);
            }
            if req.task_ledger_required
                && runtime_profile == RuntimeProfile::Eval
                && !transcript_has_task_ledger(&transcript)
            {
                instructions = apply_task_ledger_required(instructions);
            }
            if effective_policy_mode == PolicyMode::Plan {
                instructions = apply_plan_mode(instructions);
            }
            if agent_swarm_mode_active {
                instructions = apply_agent_swarm_mode(instructions);
            }
            if model_supports_reasoning_effort(&model, REASONING_ULTRA) {
                instructions = apply_codex_multi_agent_mode(
                    instructions,
                    request_reasoning.level.as_deref() == Some(REASONING_ULTRA),
                );
            }
            instructions = self
                .goals
                .apply_goal_instructions(&req.thread_id, instructions)
                .await?;
            let mut request_metadata = serde_json::json!({});
            if let Some(decision) = &speed_policy_decision {
                request_metadata["speedPolicy"] = serde_json::json!(decision);
            }
            if let Some(decision) = routing_selection.decision.as_ref() {
                request_metadata["inferenceRouting"] = serde_json::json!(decision);
            }
            if let Some(remaining) = deadline_remaining_seconds(turn_deadline) {
                request_metadata["deadlineRemainingSeconds"] = serde_json::json!(remaining);
            }
            if let Some(profile) = &model_profile {
                request_metadata["modelProfile"] = serde_json::json!({
                    "model": profile.model,
                    "providerFamily": profile.provider_family,
                    "editTool": profile.edit_tool,
                    "schemaPolicy": profile.schema_policy,
                    "instructionOverlay": profile.instruction_overlay,
                    "parallelToolCalls": profile.parallel_tool_calls,
                    "autoCompactTokenLimit": profile.auto_compact_token_limit,
                });
            }
            let task_ledger_required_this_round = req.task_ledger_required
                && runtime_profile == RuntimeProfile::Eval
                && !deadline_finalization_requested
                && !transcript_has_task_ledger(&transcript);
            let task_ledger_tools = (capabilities.tool_calls && task_ledger_required_this_round)
                .then(|| self.task_ledger_tool_specs(model_profile.as_ref()))
                .filter(|tools| !tools.is_empty());
            let request_tools = if deadline_finalization_requested {
                Vec::new()
            } else if let Some(ledger_tools) = &task_ledger_tools {
                ledger_tools.clone()
            } else {
                tools.clone()
            };
            let request_tool_choice = if deadline_finalization_requested {
                ToolChoice::None
            } else if task_ledger_tools.is_some() {
                ToolChoice::Specific(TASK_LEDGER_TOOL_NAME.to_string())
            } else {
                tool_choice.clone()
            };
            if deadline_finalization_requested {
                request_metadata["deadlineFinalization"] = serde_json::json!({
                    "reserveSeconds": deadline_finalization_reserve,
                    "remainingSeconds": deadline_remaining_seconds(turn_deadline),
                });
            } else if deadline_scoreable_completion_requested {
                request_metadata["deadlineScoreableCompletion"] = serde_json::json!({
                    "reserveSeconds": deadline_finalization_reserve,
                    "remainingSeconds": deadline_remaining_seconds(turn_deadline),
                });
            }
            let request = AgentInferenceRequest {
                model: ModelSelection {
                    provider: provider.clone(),
                    model: model.clone(),
                },
                instructions,
                transcript: transcript.clone(),
                tools: request_tools,
                tool_choice: request_tool_choice,
                reasoning: request_reasoning,
                output: OutputConfig::default(),
                runtime: RuntimeHints {
                    auto_compact_token_limit: server_side_compaction_threshold(&cfg, &model),
                    profile: runtime_profile,
                    parallel_tool_calls: Some(parallel_tool_calls),
                    hosted_web_search: cfg.hosted_web_search.clone(),
                    tool_search: tool_search_for_provider_model(&cfg, &provider, &model),
                    speed_policy: speed_policy_decision,
                    reliability: Some(cfg.reliability.clone().into()),
                    deadline_remaining_seconds: deadline_remaining_seconds(turn_deadline),
                    ..RuntimeHints::default()
                },
                metadata: request_metadata,
            };

            let ctx = InferenceTurnContext {
                thread_id: &req.thread_id,
                turn_id: &turn_id,
                tool_executor: Some(std::sync::Arc::new(
                    crate::tool_execution::RuntimeTurnToolExecutor {
                        runtime: Arc::clone(self),
                        thread_id: req.thread_id.clone(),
                        turn_id: turn_id.clone(),
                        workspace: Some(workspace.clone()),
                        deadline: turn_deadline,
                    },
                )),
            };
            let stream_future = engine.stream_turn(ctx, request);
            let mut stream = if let Some((deadline, timeout_action)) = inference_timeout_deadline(
                turn_deadline,
                runtime_profile,
                req.task_ledger_required,
                deadline_finalization_reserve,
                deadline_finalization_requested || deadline_scoreable_completion_requested,
                task_ledger_scoreable_checkpoints,
                &transcript,
            ) {
                match tokio::time::timeout_at(deadline_instant(deadline), stream_future).await {
                    Ok(stream) => stream?,
                    Err(_) => {
                        if runtime_profile == RuntimeProfile::Eval
                            && !deadline_finalization_requested
                        {
                            let remaining = deadline_remaining_seconds(turn_deadline).unwrap_or(0);
                            if timeout_action == InferenceTimeoutAction::ScoreableCheckpoint
                                && task_ledger_scoreable_checkpoints
                                    < TASK_LEDGER_SCOREABLE_CHECKPOINT_LIMIT
                                && let Some(prompt) = task_ledger_completion_prompt(&transcript)
                            {
                                task_ledger_scoreable_checkpoints += 1;
                                let item = TranscriptItem::UserMessage(UserMessage::text(
                                    task_ledger_scoreable_checkpoint_prompt(remaining, &prompt),
                                ));
                                self.persist_turn_item(&req.thread_id, &turn_id, &item)
                                    .await?;
                                transcript.push(item);
                                continue 'tool_rounds;
                            }
                            self.start_deadline_finalization(
                                &req.thread_id,
                                &turn_id,
                                &mut transcript,
                                remaining,
                            )
                            .await?;
                            deadline_finalization_requested = true;
                            continue 'tool_rounds;
                        }
                        self.fail_turn_due_to_deadline(
                            &req.thread_id,
                            &turn_id,
                            deadline,
                            &transcript,
                        )
                        .await?;
                        return Ok(TurnRunOutcome::Stopped);
                    }
                }
            } else {
                stream_future.await?
            };
            let mut assistant_text = String::new();
            let mut phase_messages = Vec::<AssistantMessage>::new();
            let mut reasoning_text = String::new();
            let mut tool_calls = Vec::new();
            let mut provider_metadata = None;

            loop {
                let next = if let Some((deadline, timeout_action)) = inference_timeout_deadline(
                    turn_deadline,
                    runtime_profile,
                    req.task_ledger_required,
                    deadline_finalization_reserve,
                    deadline_finalization_requested || deadline_scoreable_completion_requested,
                    task_ledger_scoreable_checkpoints,
                    &transcript,
                ) {
                    match tokio::time::timeout_at(deadline_instant(deadline), stream.next()).await {
                        Ok(next) => next,
                        Err(_) => {
                            if runtime_profile == RuntimeProfile::Eval
                                && !deadline_finalization_requested
                            {
                                let remaining =
                                    deadline_remaining_seconds(turn_deadline).unwrap_or(0);
                                if timeout_action == InferenceTimeoutAction::ScoreableCheckpoint
                                    && task_ledger_scoreable_checkpoints
                                        < TASK_LEDGER_SCOREABLE_CHECKPOINT_LIMIT
                                    && let Some(prompt) = task_ledger_completion_prompt(&transcript)
                                {
                                    task_ledger_scoreable_checkpoints += 1;
                                    let item = TranscriptItem::UserMessage(UserMessage::text(
                                        task_ledger_scoreable_checkpoint_prompt(remaining, &prompt),
                                    ));
                                    self.persist_turn_item(&req.thread_id, &turn_id, &item)
                                        .await?;
                                    transcript.push(item);
                                    continue 'tool_rounds;
                                }
                                self.start_deadline_finalization(
                                    &req.thread_id,
                                    &turn_id,
                                    &mut transcript,
                                    remaining,
                                )
                                .await?;
                                deadline_finalization_requested = true;
                                continue 'tool_rounds;
                            }
                            self.fail_turn_due_to_deadline(
                                &req.thread_id,
                                &turn_id,
                                deadline,
                                &transcript,
                            )
                            .await?;
                            return Ok(TurnRunOutcome::Stopped);
                        }
                    }
                } else {
                    stream.next().await
                };
                let Some(res) = next else {
                    break;
                };
                let event = match res {
                    Ok(event) => event,
                    Err(err) => {
                        let error = err.to_string();
                        if runtime_profile == RuntimeProfile::Eval
                            && !deadline_finalization_requested
                            && let Some(cause) = provider_stream_retry_cause(&error)
                        {
                            let retry_attempt = provider_stream_retry_attempts.saturating_add(1);
                            let policy: ReliabilityRequestPolicy = cfg.reliability.clone().into();
                            if retry_attempt < policy.provider_retry_max_attempts {
                                provider_stream_retry_attempts = retry_attempt;
                                let delay_ms = provider_retry_delay_ms(&policy, retry_attempt);
                                self.emit(RoderEvent::ReliabilityRetryRecorded(
                                    ReliabilityRetryRecorded {
                                        context: ReliabilityContext {
                                            thread_id: req.thread_id.clone(),
                                            turn_id: turn_id.clone(),
                                            provider: Some(provider.clone()),
                                            model: Some(model.clone()),
                                            ..ReliabilityContext::default()
                                        },
                                        error_class: ReliabilityErrorClass::ProviderError,
                                        decision: ReliabilityRetryDecision::Retry,
                                        attempt: retry_attempt,
                                        max_attempts: policy.provider_retry_max_attempts,
                                        delay_ms: Some(delay_ms),
                                        details: ReliabilityDetails::redacted(format!(
                                            "{cause}: {error}"
                                        )),
                                        timestamp: OffsetDateTime::now_utc(),
                                    },
                                ))
                                .await;
                                if delay_ms > 0 {
                                    tokio::time::sleep(std::time::Duration::from_millis(delay_ms))
                                        .await;
                                }
                                continue 'tool_rounds;
                            }
                        }
                        self.emit(RoderEvent::TurnFailed(TurnFailed {
                            thread_id: req.thread_id.clone(),
                            turn_id: turn_id.clone(),
                            error: error.clone(),
                            error_kind: None,
                            usage: None,
                            timestamp: OffsetDateTime::now_utc(),
                        }))
                        .await;
                        self.complete_team_member_turn_with_result(
                            &req.thread_id,
                            &turn_id,
                            TeamMemberStatus::Failed,
                            (!assistant_text.trim().is_empty()).then(|| assistant_text.clone()),
                            Some(error),
                        )
                        .await?;
                        return Err(err);
                    }
                };

                let inference_timestamp = OffsetDateTime::now_utc();
                self.emit(RoderEvent::InferenceEventReceived(InferenceEventReceived {
                    thread_id: req.thread_id.clone(),
                    turn_id: turn_id.clone(),
                    event: event.clone(),
                    timestamp: inference_timestamp,
                }))
                .await;

                match event {
                    InferenceEvent::MessageDelta(delta) => {
                        if let Some((team_id, member)) =
                            self.teams.member_for_thread(&req.thread_id).await
                        {
                            self.emit(RoderEvent::TeamMemberMessageDelta(TeamMemberMessageDelta {
                                team_id,
                                member_id: member.id,
                                member_thread_id: req.thread_id.clone(),
                                turn_id: turn_id.clone(),
                                delta: delta.text.clone(),
                                timestamp: OffsetDateTime::now_utc(),
                            }))
                            .await;
                        }
                        if is_final_answer_phase(delta.phase.as_deref()) {
                            assistant_text.push_str(&delta.text);
                        } else if let Some(last) = phase_messages.last_mut()
                            && last.phase == delta.phase
                        {
                            last.text.push_str(&delta.text);
                        } else {
                            phase_messages.push(AssistantMessage {
                                text: delta.text,
                                phase: delta.phase,
                            });
                        }
                    }
                    InferenceEvent::ReasoningDelta(delta) => reasoning_text.push_str(&delta.text),
                    InferenceEvent::ToolCallCompleted(call) => tool_calls.push(call),
                    InferenceEvent::Failed(failure) => {
                        speed_policy.record_failure();
                        let error = failure.message;
                        self.persist_turn_item(
                            &req.thread_id,
                            &turn_id,
                            &TranscriptItem::Error(ErrorRecord {
                                message: error.clone(),
                            }),
                        )
                        .await?;
                        self.emit(RoderEvent::TurnFailed(TurnFailed {
                            thread_id: req.thread_id.clone(),
                            turn_id: turn_id.clone(),
                            error: error.clone(),
                            error_kind: None,
                            usage: None,
                            timestamp: OffsetDateTime::now_utc(),
                        }))
                        .await;
                        self.complete_team_member_turn_with_result(
                            &req.thread_id,
                            &turn_id,
                            TeamMemberStatus::Failed,
                            (!assistant_text.trim().is_empty()).then(|| assistant_text.clone()),
                            Some(error),
                        )
                        .await?;
                        return Ok(TurnRunOutcome::Stopped);
                    }
                    InferenceEvent::Usage(usage) => {
                        turn_usage.add_assign(&usage);
                    }
                    InferenceEvent::Completed(metadata) => {
                        turn_finish_reason = metadata
                            .stop_reason
                            .as_deref()
                            .map(finish_reason_from_stop_reason);
                    }
                    InferenceEvent::Compaction(_)
                    | InferenceEvent::HostedToolCallStarted(_)
                    | InferenceEvent::HostedToolCallCompleted(_)
                    | InferenceEvent::ToolCallStarted(_)
                    | InferenceEvent::ToolCallDelta(_) => {}
                    InferenceEvent::ProviderMetadata(metadata) => {
                        provider_metadata = Some(metadata);
                    }
                }
            }

            speed_policy.record_model_output(
                !assistant_text.is_empty() || !phase_messages.is_empty(),
                tool_calls.len(),
            );
            if tool_calls.is_empty() {
                let steers = self.drain_turn_steers(&turn_id).await;
                if !steers.is_empty() {
                    for message in phase_messages {
                        let item = TranscriptItem::AssistantMessage(message);
                        self.persist_turn_item(&req.thread_id, &turn_id, &item)
                            .await?;
                        transcript.push(item);
                        self.persist_model_profile_segment(
                            &req.thread_id,
                            &turn_id,
                            model_profile.as_ref(),
                            &provider,
                            &model,
                            "assistant",
                        )
                        .await?;
                    }
                    if !assistant_text.is_empty() {
                        let assistant = TranscriptItem::AssistantMessage(AssistantMessage {
                            text: assistant_text,
                            phase: Some(FINAL_ANSWER_PHASE.to_string()),
                        });
                        self.persist_turn_item(&req.thread_id, &turn_id, &assistant)
                            .await?;
                        transcript.push(assistant);
                        self.persist_model_profile_segment(
                            &req.thread_id,
                            &turn_id,
                            model_profile.as_ref(),
                            &provider,
                            &model,
                            "assistant",
                        )
                        .await?;
                    }
                    if let Some(metadata) = provider_metadata {
                        let had_provider_compaction =
                            crate::compaction::provider_metadata_has_compaction(&metadata);
                        let item = TranscriptItem::ProviderMetadata(metadata);
                        self.persist_turn_item(&req.thread_id, &turn_id, &item)
                            .await?;
                        transcript.push(item);
                        if had_provider_compaction {
                            // Server-side compaction replaced the prior window;
                            // drop pre-boundary items so the next request does
                            // not re-send (and re-compact) the full history.
                            transcript =
                                crate::compaction::trim_to_last_compaction_boundary(transcript);
                            compacted_this_turn = true;
                        }
                    }
                    self.append_steers(&req, &turn_id, &mut transcript, steers)
                        .await?;
                    continue;
                }
                if !deadline_finalization_requested
                    && req.task_ledger_required
                    && runtime_profile == RuntimeProfile::Eval
                    && task_ledger_completion_reminders < TASK_LEDGER_COMPLETION_REMINDER_LIMIT
                    && (!assistant_text.trim().is_empty() || !phase_messages.is_empty())
                    && let Some(prompt) = task_ledger_completion_prompt(&transcript)
                {
                    task_ledger_completion_reminders += 1;
                    let item = TranscriptItem::UserMessage(UserMessage::text(prompt));
                    self.persist_turn_item(&req.thread_id, &turn_id, &item)
                        .await?;
                    transcript.push(item);
                    continue;
                }
                if !deadline_finalization_requested
                    && let Some(prompt) = verification_gate.blocking_prompt()
                {
                    speed_policy.record_verification_required();
                    self.emit(RoderEvent::VerificationRequired(VerificationRequired {
                        thread_id: req.thread_id.clone(),
                        turn_id: turn_id.clone(),
                        reason: verification_gate.reason(),
                        changed_files: verification_gate.changed_files(),
                        tool_evidence: verification_gate.tool_evidence.clone(),
                        tests_run: verification_gate.tests_run.clone(),
                        open_gaps: verification_gate.open_gaps.clone(),
                        timestamp: OffsetDateTime::now_utc(),
                    }))
                    .await;
                    let item = TranscriptItem::UserMessage(UserMessage::text(prompt));
                    self.persist_turn_item(&req.thread_id, &turn_id, &item)
                        .await?;
                    transcript.push(item);
                    continue;
                }
                // Loop persistence nudge: in non-interactive/eval runs, when the
                // model returns a final message with no tool calls, gently prompt
                // it to keep going (bounded by `empty_tool_call_nudges`) before
                // ending the turn. Gated to non-interactive/eval so interactive
                // chat completions are never delayed, and only fires when the
                // model actually produced a final answer to re-examine.
                if !deadline_finalization_requested
                    && runtime_profile != RuntimeProfile::Interactive
                    && cfg.reliability.empty_tool_call_nudges > 0
                    && empty_tool_call_nudges_used < cfg.reliability.empty_tool_call_nudges
                    && (!assistant_text.trim().is_empty() || !phase_messages.is_empty())
                {
                    empty_tool_call_nudges_used += 1;
                    let item = TranscriptItem::UserMessage(UserMessage::text(
                        RELIABILITY_CONTINUATION_PROMPT.to_string(),
                    ));
                    self.persist_turn_item(&req.thread_id, &turn_id, &item)
                        .await?;
                    transcript.push(item);
                    continue;
                }
                if deadline_finalization_requested
                    && assistant_text.trim().is_empty()
                    && phase_messages.is_empty()
                {
                    assistant_text = format!(
                        "Deadline finalization completed without model text. {}",
                        turn_partial_result(&transcript)
                    );
                }
                final_phase_messages = phase_messages;
                final_assistant_text = assistant_text;
                final_reasoning_text = reasoning_text;
                final_provider_metadata = provider_metadata;
                exhausted_tool_rounds = false;
                break;
            }

            for message in phase_messages {
                let item = TranscriptItem::AssistantMessage(message);
                self.persist_turn_item(&req.thread_id, &turn_id, &item)
                    .await?;
                transcript.push(item);
                self.persist_model_profile_segment(
                    &req.thread_id,
                    &turn_id,
                    model_profile.as_ref(),
                    &provider,
                    &model,
                    "assistant",
                )
                .await?;
            }
            if !assistant_text.is_empty() {
                transcript.push(TranscriptItem::AssistantMessage(AssistantMessage {
                    text: assistant_text,
                    phase: Some(FINAL_ANSWER_PHASE.to_string()),
                }));
                self.persist_model_profile_segment(
                    &req.thread_id,
                    &turn_id,
                    model_profile.as_ref(),
                    &provider,
                    &model,
                    "assistant",
                )
                .await?;
            }
            if let Some(metadata) = provider_metadata {
                let had_provider_compaction =
                    crate::compaction::provider_metadata_has_compaction(&metadata);
                let item = TranscriptItem::ProviderMetadata(metadata);
                self.persist_turn_item(&req.thread_id, &turn_id, &item)
                    .await?;
                transcript.push(item);
                if had_provider_compaction {
                    // Server-side compaction replaced the prior window; drop
                    // pre-boundary items so subsequent tool rounds use the
                    // compact window as the next input.
                    transcript = crate::compaction::trim_to_last_compaction_boundary(transcript);
                    compacted_this_turn = true;
                }
            }
            for call in &tool_calls {
                let tool_item = TranscriptItem::ToolCall(ToolCallRecord {
                    id: call.id.clone(),
                    name: call.name.clone(),
                    arguments: call.arguments.clone(),
                });
                self.persist_turn_item(&req.thread_id, &turn_id, &tool_item)
                    .await?;
                transcript.push(tool_item);
                self.persist_model_profile_segment(
                    &req.thread_id,
                    &turn_id,
                    model_profile.as_ref(),
                    &provider,
                    &model,
                    "tool_call",
                )
                .await?;
            }
            if let Some(deadline) = turn_deadline
                && deadline_expired(deadline)
            {
                self.fail_turn_due_to_deadline(&req.thread_id, &turn_id, deadline, &transcript)
                    .await?;
                return Ok(TurnRunOutcome::Stopped);
            }
            let results = self
                .route_tool_calls(
                    &req.thread_id,
                    &turn_id,
                    tool_calls,
                    parallel_tool_calls,
                    Some(workspace.as_str()),
                    turn_deadline,
                )
                .await?;
            let reliability_limit = reliability.record_tool_results(
                &cfg.reliability,
                &results,
                runtime_profile == RuntimeProfile::Interactive,
            );
            for result in results {
                verification_gate.record_tool_result(&result);
                transcript.push(TranscriptItem::ToolResult(result));
                self.persist_model_profile_segment(
                    &req.thread_id,
                    &turn_id,
                    model_profile.as_ref(),
                    &provider,
                    &model,
                    "tool_result",
                )
                .await?;
            }
            if let Some(limit) = reliability_limit {
                if limit.decision == ReliabilityLimitDecision::RequestContinuation {
                    // Loop persistence: rather than ending the turn, reset the
                    // consecutive-failure counter, nudge the model to keep going,
                    // and continue the round loop. Bounded by the per-turn tool
                    // failure ceiling, `max_model_calls_per_turn`, and
                    // `MAX_TOOL_ROUNDS_PER_TURN`.
                    self.record_reliability_limit_continuation(
                        &req.thread_id,
                        &turn_id,
                        &provider,
                        &model,
                        limit,
                    )
                    .await;
                    reliability.reset_consecutive_failures();
                    let nudge = TranscriptItem::UserMessage(UserMessage::text(
                        RELIABILITY_CONTINUATION_PROMPT.to_string(),
                    ));
                    self.persist_turn_item(&req.thread_id, &turn_id, &nudge)
                        .await?;
                    transcript.push(nudge);
                } else {
                    self.fail_turn_due_to_reliability_limit(
                        &req.thread_id,
                        &turn_id,
                        &provider,
                        &model,
                        limit,
                        &transcript,
                    )
                    .await?;
                    return Ok(TurnRunOutcome::Stopped);
                }
            }
            transcript = self
                .compact_transcript_if_needed(
                    &req.thread_id,
                    &turn_id,
                    &provider,
                    &model,
                    transcript,
                    self.compaction_options_for_turn(&req.thread_id, !compacted_this_turn),
                )
                .await?;
            compacted_this_turn = compacted_this_turn
                || transcript
                    .iter()
                    .any(crate::compaction::is_compaction_boundary);
        }

        if exhausted_tool_rounds {
            let message =
                format!("tool call limit reached after {MAX_TOOL_ROUNDS_PER_TURN} rounds");
            self.persist_turn_item(
                &req.thread_id,
                &turn_id,
                &TranscriptItem::Error(ErrorRecord {
                    message: message.clone(),
                }),
            )
            .await?;
            self.emit(RoderEvent::TurnFailed(TurnFailed {
                thread_id: req.thread_id.clone(),
                turn_id: turn_id.clone(),
                error: message.clone(),
                error_kind: None,
                usage: None,
                timestamp: OffsetDateTime::now_utc(),
            }))
            .await;
            self.complete_team_member_turn_with_result(
                &req.thread_id,
                &turn_id,
                TeamMemberStatus::Failed,
                None,
                Some(message),
            )
            .await?;
            return Ok(TurnRunOutcome::Stopped);
        }

        if !final_reasoning_text.is_empty() {
            self.persist_turn_item(
                &req.thread_id,
                &turn_id,
                &TranscriptItem::ReasoningSummary(ReasoningSummary {
                    text: final_reasoning_text,
                }),
            )
            .await?;
        }
        for message in final_phase_messages {
            self.persist_turn_item(
                &req.thread_id,
                &turn_id,
                &TranscriptItem::AssistantMessage(message),
            )
            .await?;
            self.persist_model_profile_segment(
                &req.thread_id,
                &turn_id,
                model_profile.as_ref(),
                &provider,
                &model,
                "assistant",
            )
            .await?;
        }
        if !final_assistant_text.is_empty() {
            self.persist_turn_item(
                &req.thread_id,
                &turn_id,
                &TranscriptItem::AssistantMessage(AssistantMessage {
                    text: final_assistant_text.clone(),
                    phase: Some(FINAL_ANSWER_PHASE.to_string()),
                }),
            )
            .await?;
            self.persist_model_profile_segment(
                &req.thread_id,
                &turn_id,
                model_profile.as_ref(),
                &provider,
                &model,
                "assistant",
            )
            .await?;
        }
        if let Some(metadata) = final_provider_metadata {
            self.persist_turn_item(
                &req.thread_id,
                &turn_id,
                &TranscriptItem::ProviderMetadata(metadata),
            )
            .await?;
        }

        let turn_usage_tokens = turn_usage.total_tokens as i64;
        let completed_usage = (!turn_usage.is_empty()).then_some(turn_usage.clone());
        self.record_thread_usage_metadata(&req.thread_id, &turn_usage)
            .await?;
        self.goals
            .account_turn_usage(
                &req.thread_id,
                turn_usage_tokens,
                OffsetDateTime::now_utc() - turn_started_at,
            )
            .await?;
        self.emit(RoderEvent::TurnCompleted(TurnCompleted {
            thread_id: req.thread_id.clone(),
            turn_id: turn_id.clone(),
            usage: completed_usage,
            finish_reason: turn_finish_reason,
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        self.complete_team_member_turn_with_result(
            &req.thread_id,
            &turn_id,
            TeamMemberStatus::Completed,
            (!final_assistant_text.is_empty()).then_some(final_assistant_text),
            None,
        )
        .await?;
        self.persist_runner_state(&req.thread_id, runner_session.as_ref())
            .await?;
        Ok(TurnRunOutcome::Completed)
    }

    async fn drain_turn_steers(&self, turn_id: &TurnId) -> Vec<QueuedTurnSteer> {
        let Some(active) = self.active_turns.read().await.get(turn_id).cloned() else {
            return Vec::new();
        };
        let mut steers = active.steers.lock().await;
        std::mem::take(&mut *steers)
    }

    async fn route_tool_calls(
        self: &Arc<Self>,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        calls: Vec<ToolCallCompleted>,
        parallel: bool,
        workspace: Option<&str>,
        deadline: Option<OffsetDateTime>,
    ) -> anyhow::Result<Vec<ToolResultRecord>> {
        // Enforce the agent_swarm exclusivity rule (roadmap 104, Task 2):
        // `agent_swarm` must be the only tool call in a model response. A mixed
        // or multi-swarm batch is denied wholesale with actionable retry text so
        // the model re-issues `agent_swarm` by itself, and every tool_call_id
        // still gets a response (keeping the chat-completions transcript valid).
        if let Some(violation) = roder_api::subagents::agent_swarm_batch_violation(
            calls.iter().map(|call| call.name.as_str()),
        ) {
            let message = violation.deny_message();
            return Ok(calls
                .into_iter()
                .map(|call| ToolResultRecord {
                    id: call.id,
                    name: Some(call.name),
                    result: message.clone(),
                    display_payload: None,
                    is_error: true,
                })
                .collect());
        }
        // Emit swarm lifecycle events on the bus (roadmap 104, Task 1) so any
        // app-server/SDK/TUI client can observe a swarm as a whole; per-child
        // progress flows through the existing Subagent* trace events.
        let swarm_call = (calls.len() == 1
            && calls[0].name == roder_api::subagents::AGENT_SWARM_TOOL_NAME)
            .then(|| (calls[0].id.clone(), calls[0].arguments.clone()));
        if let Some((tool_id, args)) = &swarm_call {
            self.emit(RoderEvent::AgentSwarmStarted(
                roder_api::subagents::AgentSwarmStarted {
                    thread_id: thread_id.clone(),
                    turn_id: turn_id.clone(),
                    tool_id: tool_id.clone(),
                    child_count: agent_swarm_child_count(args),
                    timestamp: OffsetDateTime::now_utc(),
                },
            ))
            .await;
        }

        let force_sequential = calls
            .iter()
            .any(|call| crate::agent_control_tools::is_agent_control_tool(&call.name));
        let results =
            if parallel && !force_sequential {
                try_join_all(calls.into_iter().map(|call| {
                    self.route_tool_call(thread_id, turn_id, call, workspace, deadline)
                }))
                .await
            } else {
                let mut results = Vec::with_capacity(calls.len());
                for call in calls {
                    results.push(
                        self.route_tool_call(thread_id, turn_id, call, workspace, deadline)
                            .await?,
                    );
                }
                Ok(results)
            }?;

        if let Some((tool_id, _)) = &swarm_call
            && let Some(result) = results.iter().find(|result| &result.id == tool_id)
            && let Some((completed, failed, aborted)) = parse_swarm_counts(&result.result)
        {
            self.emit(RoderEvent::AgentSwarmCompleted(
                roder_api::subagents::AgentSwarmCompleted {
                    thread_id: thread_id.clone(),
                    turn_id: turn_id.clone(),
                    tool_id: tool_id.clone(),
                    completed,
                    failed,
                    aborted,
                    timestamp: OffsetDateTime::now_utc(),
                },
            ))
            .await;
        }

        Ok(results)
    }

    async fn fail_turn_with_error(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        message: String,
    ) -> anyhow::Result<()> {
        self.persist_turn_item(
            thread_id,
            turn_id,
            &TranscriptItem::Error(ErrorRecord {
                message: message.clone(),
            }),
        )
        .await?;
        self.emit(RoderEvent::TurnFailed(TurnFailed {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            error: message.clone(),
            error_kind: None,
            usage: None,
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        self.complete_team_member_turn_with_result(
            thread_id,
            turn_id,
            TeamMemberStatus::Failed,
            None,
            Some(message),
        )
        .await?;
        Ok(())
    }

    async fn fail_turn_due_to_deadline(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        deadline: OffsetDateTime,
        transcript: &[TranscriptItem],
    ) -> anyhow::Result<()> {
        let partial_result = turn_partial_result(transcript);
        self.emit(RoderEvent::TurnPartialResult(TurnPartialResult {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            summary: partial_result.clone(),
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        self.emit(RoderEvent::TurnDeadlineExceeded(TurnDeadlineExceeded {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            deadline,
            partial_result: partial_result.clone(),
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        let message = "turn deadline expired".to_string();
        self.persist_turn_item(
            thread_id,
            turn_id,
            &TranscriptItem::Error(ErrorRecord {
                message: format!("{message}: {partial_result}"),
            }),
        )
        .await?;
        self.emit(RoderEvent::TurnFailed(TurnFailed {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            error: message.clone(),
            error_kind: Some("deadline_timeout".to_string()),
            usage: None,
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        self.complete_team_member_turn_with_result(
            thread_id,
            turn_id,
            TeamMemberStatus::Failed,
            None,
            Some(format!("{message}: {partial_result}")),
        )
        .await?;
        Ok(())
    }

    async fn start_deadline_finalization(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        transcript: &mut Vec<TranscriptItem>,
        remaining_seconds: u64,
    ) -> anyhow::Result<()> {
        let item = TranscriptItem::UserMessage(crate::deadline_policy::finalization_message(
            remaining_seconds,
        ));
        self.persist_turn_item(thread_id, turn_id, &item).await?;
        transcript.push(item);
        self.emit(RoderEvent::TurnPartialResult(TurnPartialResult {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            summary: turn_partial_result(transcript),
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        Ok(())
    }

    /// Emits the reliability-limit event for a continuation (loop persistence)
    /// without failing the turn. The caller resets the failure counter and
    /// injects a nudge so the round loop keeps going.
    async fn record_reliability_limit_continuation(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        provider: &str,
        model: &str,
        limit: ReliabilityLimitHit,
    ) {
        self.emit(RoderEvent::ReliabilityLimitRecorded(
            ReliabilityLimitRecorded {
                context: ReliabilityContext {
                    thread_id: thread_id.clone(),
                    turn_id: turn_id.clone(),
                    tool_id: None,
                    tool_name: None,
                    provider: Some(provider.to_string()),
                    model: Some(model.to_string()),
                },
                error_class: limit.error_class,
                limit_kind: limit.limit_kind,
                decision: limit.decision,
                current: limit.current,
                limit: limit.limit,
                details: ReliabilityDetails::redacted(&limit.message),
                timestamp: OffsetDateTime::now_utc(),
            },
        ))
        .await;
    }

    async fn fail_turn_due_to_reliability_limit(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        provider: &str,
        model: &str,
        limit: ReliabilityLimitHit,
        transcript: &[TranscriptItem],
    ) -> anyhow::Result<()> {
        self.emit(RoderEvent::ReliabilityLimitRecorded(
            ReliabilityLimitRecorded {
                context: ReliabilityContext {
                    thread_id: thread_id.clone(),
                    turn_id: turn_id.clone(),
                    tool_id: None,
                    tool_name: None,
                    provider: Some(provider.to_string()),
                    model: Some(model.to_string()),
                },
                error_class: limit.error_class,
                limit_kind: limit.limit_kind,
                decision: limit.decision,
                current: limit.current,
                limit: limit.limit,
                details: ReliabilityDetails::redacted(&limit.message),
                timestamp: OffsetDateTime::now_utc(),
            },
        ))
        .await;
        let partial_result = turn_partial_result(transcript);
        self.emit(RoderEvent::TurnPartialResult(TurnPartialResult {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            summary: partial_result.clone(),
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        let message = format!("reliability limit reached: {}", limit.message);
        self.persist_turn_item(
            thread_id,
            turn_id,
            &TranscriptItem::Error(ErrorRecord {
                message: format!("{message}: {partial_result}"),
            }),
        )
        .await?;
        self.emit(RoderEvent::TurnFailed(TurnFailed {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            error: message.clone(),
            error_kind: Some("reliability_limit".to_string()),
            usage: None,
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        self.complete_team_member_turn_with_result(
            thread_id,
            turn_id,
            TeamMemberStatus::Failed,
            None,
            Some(format!("{message}: {partial_result}")),
        )
        .await?;
        Ok(())
    }

    async fn append_steers(
        &self,
        req: &StartTurnRequest,
        turn_id: &TurnId,
        transcript: &mut Vec<TranscriptItem>,
        steers: Vec<QueuedTurnSteer>,
    ) -> anyhow::Result<()> {
        for queued in steers {
            let mut steer = queued.message;
            steer.text = steer.text.trim().to_string();
            if steer.text.is_empty() && steer.images.is_empty() {
                continue;
            }
            let item = TranscriptItem::UserMessage(steer);
            self.persist_turn_item(&req.thread_id, turn_id, &item)
                .await?;
            transcript.push(item);
            if let Some(ack) = queued.mailbox_ack {
                self.teams
                    .mark_mailbox_messages_delivered(&ack.team_id, turn_id, &ack.message_ids)
                    .await?;
            }
        }
        Ok(())
    }

    async fn persist_model_profile_segment(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        profile: Option<&ModelHarnessProfile>,
        provider: &str,
        model: &str,
        segment: &str,
    ) -> anyhow::Result<()> {
        let item = TranscriptItem::ProviderMetadata(model_profile_segment_metadata(
            profile, provider, model, segment,
        ));
        self.persist_turn_item(thread_id, turn_id, &item).await
    }

    /**
     * Allowlists apply to built-in tools only; external tools are advertised with their
     * host-supplied schemas as given. An external tool shadows a built-in with the same name in
     * both advertisement and dispatch (see `route_tool_call`).
     */
    fn filtered_tool_specs(
        &self,
        cfg: &RuntimeConfig,
        model: &str,
        profile: Option<&ModelHarnessProfile>,
        thread_allowlist: &[String],
        external_tools: &[roder_api::tools::ToolSpec],
    ) -> Vec<roder_api::tools::ToolSpec> {
        let mut specs = self
            .tool_registry
            .specs_for_edit_tool_with_schema_policy(
                edit_tool_for_model(cfg, model),
                schema_policy_for_model(profile),
            )
            .into_iter()
            .filter(|spec| {
                allowlist_permits(&cfg.tool_allowlist, &spec.name)
                    && allowlist_permits(thread_allowlist, &spec.name)
                    && !external_tools.iter().any(|tool| tool.name == spec.name)
            })
            .collect::<Vec<_>>();
        specs.extend(external_tools.iter().cloned());
        specs
    }

    fn task_ledger_tool_specs(
        &self,
        profile: Option<&ModelHarnessProfile>,
    ) -> Vec<roder_api::tools::ToolSpec> {
        self.tool_registry
            .get(TASK_LEDGER_TOOL_NAME)
            .map(|tool| {
                tool.spec()
                    .normalized_for_model_profile(schema_policy_for_model(profile))
            })
            .into_iter()
            .collect()
    }

    pub(crate) fn engine_for(&self, provider: &str) -> anyhow::Result<Arc<dyn InferenceEngine>> {
        self.registry
            .inference_engine(provider)
            .or_else(|| {
                self.registry
                    .default_inference_engine()
                    .filter(|engine| provider.is_empty() || engine.id() == provider)
            })
            .ok_or_else(|| anyhow::anyhow!("inference provider {provider:?} is not registered"))
    }

    pub async fn emit(&self, event: RoderEvent) -> EventEnvelope {
        let envelope = self.bus.emit(event);
        if let (Some(store), Some(thread_id)) = (&self.thread_store, envelope.thread_id.as_ref())
            && should_persist_thread_event(thread_id)
        {
            let _ = store.append_event(thread_id, &envelope).await;
        }
        // Registered event sinks (e.g. process extensions) receive the
        // persisted envelope through bounded per-sink queues; a slow sink
        // never blocks emit or turn progress.
        let dispatcher = self
            .event_sink_dispatcher
            .get_or_init(|| async {
                crate::event_sink_dispatch::EventSinkDispatcher::start(
                    &self.registry.event_sinks,
                    self.bus.clone(),
                )
            })
            .await;
        if !dispatcher.is_empty() {
            dispatcher.dispatch(&envelope, &self.bus);
        }
        envelope
    }

    /// Records a projected thread item event and persists it through the configured thread store.
    ///
    /// App-server protocol notification bridges currently call this after translating runtime
    /// events into item events. Headless runtime consumers that subscribe to `RoderEvent` directly
    /// must make the same call if they need the derived item event stream persisted.
    pub async fn record_thread_item_event_kind(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        timestamp: OffsetDateTime,
        kind: ThreadItemEventKind,
    ) -> anyhow::Result<ThreadItemEvent> {
        let seq = self.next_thread_item_event_seq(thread_id).await?;
        let item_event = ThreadItemEvent {
            seq,
            event_id: format!("{turn_id}-item-event-{seq}"),
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            timestamp,
            event: kind,
        };
        if let Some(store) = &self.thread_store {
            store.append_item_event(thread_id, &item_event).await?;
        }
        self.remember_thread_item_event(&item_event).await?;
        Ok(item_event)
    }

    async fn next_thread_item_event_seq(&self, thread_id: &ThreadId) -> anyhow::Result<u64> {
        self.ensure_thread_item_cache(thread_id).await?;
        Ok(self
            .thread_item_cache
            .lock()
            .await
            .next_item_event_seq(thread_id))
    }

    pub async fn thread_item_exists(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        item_id: &str,
    ) -> anyhow::Result<bool> {
        self.ensure_thread_item_cache(thread_id).await?;
        Ok(self
            .thread_item_cache
            .lock()
            .await
            .thread_item_exists(thread_id, turn_id, item_id))
    }

    pub async fn current_reasoning_item_id(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
    ) -> anyhow::Result<Option<String>> {
        self.ensure_thread_item_cache(thread_id).await?;
        Ok(self
            .thread_item_cache
            .lock()
            .await
            .current_reasoning_item_id(thread_id, turn_id))
    }

    async fn remember_thread_item_event(&self, item_event: &ThreadItemEvent) -> anyhow::Result<()> {
        self.ensure_thread_item_cache(&item_event.thread_id).await?;
        self.thread_item_cache
            .lock()
            .await
            .remember_item_event(item_event);
        Ok(())
    }

    pub async fn latest_transcript_item_index(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
    ) -> anyhow::Result<Option<usize>> {
        self.ensure_thread_item_cache(thread_id).await?;
        Ok(self
            .thread_item_cache
            .lock()
            .await
            .latest_transcript_item_index(thread_id, turn_id))
    }

    async fn next_transcript_item_index(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
    ) -> anyhow::Result<usize> {
        self.ensure_thread_item_cache(thread_id).await?;
        Ok(self
            .thread_item_cache
            .lock()
            .await
            .next_transcript_item_index(thread_id, turn_id))
    }

    async fn remember_transcript_item_index(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        item_index: usize,
    ) -> anyhow::Result<()> {
        self.ensure_thread_item_cache(thread_id).await?;
        self.thread_item_cache
            .lock()
            .await
            .remember_transcript_item_index(thread_id, turn_id, item_index);
        Ok(())
    }

    async fn ensure_thread_item_cache(&self, thread_id: &ThreadId) -> anyhow::Result<()> {
        if self
            .thread_item_cache
            .lock()
            .await
            .contains_thread(thread_id)
        {
            return Ok(());
        }

        let snapshot = if let Some(store) = &self.thread_store {
            store.load_thread(thread_id).await?
        } else {
            None
        };
        self.thread_item_cache.lock().await.ensure_thread(
            thread_id,
            ThreadItemCacheEntry::from_snapshot(snapshot.as_ref()),
        );
        Ok(())
    }

    pub(crate) async fn persist_turn_item(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        item: &TranscriptItem,
    ) -> anyhow::Result<()> {
        let item_index = self.next_transcript_item_index(thread_id, turn_id).await?;
        let timestamp = OffsetDateTime::now_utc();
        self.emit(RoderEvent::TranscriptItemAppended(TranscriptItemAppended {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            item_type: match item {
                TranscriptItem::UserMessage(_) => "user_message",
                TranscriptItem::AssistantMessage(_) => "assistant_message",
                TranscriptItem::ReasoningSummary(_) => "reasoning_summary",
                TranscriptItem::ToolCall(_) => "tool_call",
                TranscriptItem::ToolResult(_) => "tool_result",
                TranscriptItem::FileChange(_) => "file_change",
                TranscriptItem::ContextCompaction(_) => "context_compaction",
                TranscriptItem::Error(_) => "error",
                TranscriptItem::ProviderMetadata(_) => "provider_metadata",
            }
            .to_string(),
            item_index: Some(item_index),
            item: Some(item.clone()),
            timestamp,
        }))
        .await;
        self.remember_transcript_item_index(thread_id, turn_id, item_index)
            .await?;
        Ok(())
    }
}

fn transcript_has_images(transcript: &[TranscriptItem]) -> bool {
    transcript.iter().any(|item| {
        matches!(
            item,
            TranscriptItem::UserMessage(message) if !message.images.is_empty()
        )
    })
}

fn transcript_has_task_ledger(transcript: &[TranscriptItem]) -> bool {
    transcript.iter().any(|item| {
        matches!(
            item,
            TranscriptItem::ToolResult(result)
                if result.name.as_deref() == Some(TASK_LEDGER_TOOL_NAME) && !result.is_error
        )
    })
}

fn task_ledger_completion_prompt(transcript: &[TranscriptItem]) -> Option<String> {
    let latest = transcript.iter().rev().find_map(|item| match item {
        TranscriptItem::ToolResult(result)
            if result.name.as_deref() == Some(TASK_LEDGER_TOOL_NAME) && !result.is_error =>
        {
            Some(result.result.as_str())
        }
        _ => None,
    })?;
    if !task_ledger_has_open_items(latest) {
        return None;
    }

    let mut ledger = latest.chars().take(1500).collect::<String>();
    if latest.chars().nth(1500).is_some() {
        ledger.push_str("...");
    }
    Some(format!(
        "Task Ledger Completion Required: the latest task ledger still has pending or in-progress items. Do not provide a final answer yet. Use tools to complete the remaining scoreable work, create or update any required output files, then call `{TASK_LEDGER_TOOL_NAME}` with every task completed and evidence before finalizing.\n\nLatest ledger:\n{ledger}"
    ))
}

fn task_ledger_deadline_completion_prompt(
    remaining_seconds: u64,
    reserve_seconds: u64,
    completion_prompt: &str,
) -> String {
    format!(
        "Eval deadline scoreable completion: {remaining_seconds} seconds remain in the {reserve_seconds}-second finalization reserve. Do not browse, search, or start slow work. Use the available tools now to create or update the required scoreable output files, run only a quick local check if needed, then update the task ledger to completed before finalizing.\n\n{completion_prompt}"
    )
}

fn task_ledger_scoreable_checkpoint_prompt(
    remaining_seconds: u64,
    completion_prompt: &str,
) -> String {
    format!(
        "Scoreable Output Checkpoint: {remaining_seconds} seconds remain before the eval deadline. Before any further research, browsing, or long commands, use tools now to ensure the required output file(s) exist with the best evidence-backed answer, even if provisional. If a scoreable file already exists, read it and preserve that candidate unless you have stronger task-specific evidence for a replacement. Do not overwrite a plausible dated, historical, or local-evidence candidate with a current live-page, partial-coverage, or weaker guess merely to refresh the checkpoint. You may continue refining afterward, but do not apologize or finalize until the scoreable file exists and the task ledger is updated.\n\n{completion_prompt}"
    )
}

fn task_ledger_has_open_items(ledger: &str) -> bool {
    ledger.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with("- pending:") || line.starts_with("- in_progress:")
    })
}

fn turn_deadline_for_config(cfg: &RuntimeConfig) -> Option<OffsetDateTime> {
    if !cfg.runtime_profile.is_non_interactive() {
        return None;
    }
    cfg.turn_deadline_seconds
        .filter(|seconds| *seconds > 0)
        .map(|seconds| OffsetDateTime::now_utc() + Duration::seconds(seconds as i64))
}

fn deadline_expired(deadline: OffsetDateTime) -> bool {
    OffsetDateTime::now_utc() >= deadline
}

pub(crate) fn deadline_remaining_seconds(deadline: Option<OffsetDateTime>) -> Option<u64> {
    let deadline = deadline?;
    if deadline <= OffsetDateTime::now_utc() {
        return Some(0);
    }
    Some(
        (deadline - OffsetDateTime::now_utc())
            .unsigned_abs()
            .as_secs()
            .max(1),
    )
}

fn deadline_instant(deadline: OffsetDateTime) -> tokio::time::Instant {
    let now = OffsetDateTime::now_utc();
    if deadline <= now {
        return tokio::time::Instant::now();
    }
    tokio::time::Instant::now() + (deadline - now).unsigned_abs()
}

fn inference_timeout_deadline(
    deadline: Option<OffsetDateTime>,
    runtime_profile: RuntimeProfile,
    task_ledger_required: bool,
    reserve_seconds: u64,
    finalization_requested: bool,
    task_ledger_scoreable_checkpoints: u8,
    transcript: &[TranscriptItem],
) -> Option<(OffsetDateTime, InferenceTimeoutAction)> {
    let deadline = deadline?;
    if runtime_profile == RuntimeProfile::Eval
        && task_ledger_required
        && !finalization_requested
        && task_ledger_scoreable_checkpoints < TASK_LEDGER_SCOREABLE_CHECKPOINT_LIMIT
        && TASK_LEDGER_SCOREABLE_CHECKPOINT_SECONDS > reserve_seconds
        && task_ledger_completion_prompt(transcript).is_some()
    {
        let checkpoint_deadline =
            deadline - Duration::seconds(TASK_LEDGER_SCOREABLE_CHECKPOINT_SECONDS as i64);
        if checkpoint_deadline > OffsetDateTime::now_utc() {
            return Some((
                checkpoint_deadline,
                InferenceTimeoutAction::ScoreableCheckpoint,
            ));
        }
    }
    if runtime_profile == RuntimeProfile::Eval && !finalization_requested {
        return Some((
            deadline - Duration::seconds(reserve_seconds as i64),
            InferenceTimeoutAction::Finalization,
        ));
    }
    Some((deadline, InferenceTimeoutAction::Finalization))
}

fn turn_partial_result(transcript: &[TranscriptItem]) -> String {
    let tool_results = transcript
        .iter()
        .filter(|item| matches!(item, TranscriptItem::ToolResult(_)))
        .count();
    let assistant_messages = transcript
        .iter()
        .filter(|item| matches!(item, TranscriptItem::AssistantMessage(_)))
        .count();
    format!(
        "partial turn state: {} transcript items, {assistant_messages} assistant messages, {tool_results} tool results",
        transcript.len()
    )
}

fn reasoning_for_model(cfg: &RuntimeConfig, model: &str) -> ReasoningConfig {
    let level = effective_reasoning_for_model(cfg, model);
    match level.as_str() {
        "" | REASONING_NONE => ReasoningConfig::default(),
        level => ReasoningConfig {
            enabled: true,
            level: Some(level.to_string()),
        },
    }
}

fn server_side_compaction_threshold(cfg: &RuntimeConfig, model: &str) -> Option<u32> {
    let entry = lookup_model(model)?;
    if !entry.supports_compaction {
        return None;
    }
    cfg.auto_compact_token_limit
        .or_else(|| {
            model_profile_for_model(cfg, model).and_then(|profile| profile.auto_compact_token_limit)
        })
        .or(Some(entry.auto_compact_token_limit))
        .filter(|threshold| *threshold > 0)
}

pub(crate) fn tool_search_for_provider_model(
    cfg: &RuntimeConfig,
    provider: &str,
    model: &str,
) -> ToolSearchConfig {
    let mut resolved = cfg.tool_search.clone();
    if let Some(provider_config) = cfg.provider_tool_search.get(provider) {
        provider_config.apply_to(&mut resolved);
    }
    if let Some(model_config) = cfg.model_tool_search.get(model) {
        model_config.apply_to(&mut resolved);
    }
    resolved
}

fn parallel_tool_calls_for_model(cfg: &RuntimeConfig, model: &str) -> bool {
    cfg.model_parallel_tool_calls
        .get(model)
        .copied()
        .or_else(|| {
            model_profile_for_model(cfg, model).and_then(|profile| profile.parallel_tool_calls)
        })
        .unwrap_or(true)
}

/// Count the children an `agent_swarm` call will launch from its arguments
/// (item-based spawns plus resumes). Lenient: returns 0 on malformed input.
fn agent_swarm_child_count(arguments: &str) -> usize {
    serde_json::from_str::<roder_api::subagents::AgentSwarmRequest>(arguments)
        .map(|request| request.items.len() + request.resume_agent_ids.len())
        .unwrap_or(0)
}

/// Parse `completed`/`failed`/`aborted` counts from an `<agent_swarm_result>`
/// summary line, e.g. `<summary>completed: 2, failed: 1</summary>`. Omitted
/// buckets default to 0. Returns `None` when the text is not a swarm result.
fn parse_swarm_counts(text: &str) -> Option<(usize, usize, usize)> {
    if !text.contains("<agent_swarm_result>") {
        return None;
    }
    let bucket = |label: &str| -> usize {
        let needle = format!("{label}: ");
        text.find(&needle)
            .map(|start| start + needle.len())
            .map(|start| {
                text[start..]
                    .chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect::<String>()
            })
            .and_then(|digits| digits.parse().ok())
            .unwrap_or(0)
    };
    Some((bucket("completed"), bucket("failed"), bucket("aborted")))
}

fn effective_reasoning_for_model(cfg: &RuntimeConfig, model: &str) -> String {
    let base_reasoning = default_effective_reasoning_for_model(cfg, model);
    if cfg.dynamic_workflows.effort_profile == DynamicWorkflowEffortProfile::Ultracode {
        return ultracode_reasoning_level_for_model(
            model,
            &cfg.speed_policy.ultracode_reasoning,
            &base_reasoning,
        );
    }
    base_reasoning
}

fn default_effective_reasoning_for_model(cfg: &RuntimeConfig, model: &str) -> String {
    let Some(entry) = lookup_model(model) else {
        return cfg
            .reasoning
            .clone()
            .unwrap_or_else(|| REASONING_NONE.to_string());
    };
    if entry.supported_reasoning.is_empty() {
        return REASONING_NONE.to_string();
    }
    cfg.reasoning
        .as_deref()
        .filter(|reasoning| {
            entry
                .supported_reasoning
                .iter()
                .any(|option| option.effort == *reasoning)
        })
        .map(str::to_string)
        .or_else(|| {
            model_profile_for_model(cfg, model)
                .and_then(|profile| profile.reasoning.orientation)
                .filter(|reasoning| {
                    entry
                        .supported_reasoning
                        .iter()
                        .any(|option| option.effort == reasoning)
                })
        })
        .unwrap_or_else(|| entry.default_reasoning.to_string())
}

fn validate_reasoning_effort(model: &str, effort: &str) -> anyhow::Result<()> {
    if effort == REASONING_NONE && !model_supports_reasoning(model, effort) {
        return Ok(());
    }
    let Some(entry) = lookup_model(model) else {
        return Ok(());
    };
    if entry
        .supported_reasoning
        .iter()
        .any(|option| option.effort == effort)
    {
        Ok(())
    } else {
        anyhow::bail!("model {model} does not support reasoning effort {effort}")
    }
}

fn validate_runtime_config_reasoning(cfg: &RuntimeConfig) -> anyhow::Result<()> {
    let Some(reasoning) = cfg.reasoning.as_deref() else {
        return Ok(());
    };
    let Some(entry) = lookup_model(&cfg.default_model) else {
        return Ok(());
    };
    if entry.provider != PROVIDER_GEMINI {
        return Ok(());
    }
    validate_reasoning_effort(&cfg.default_model, reasoning)
}

fn validate_runtime_inference_router_config(
    registry: &ExtensionRegistry,
    cfg: &RuntimeConfig,
) -> anyhow::Result<()> {
    if !cfg.inference_router.enabled {
        return Ok(());
    }
    let Some(router_id) = cfg.inference_router.router_id.as_deref() else {
        anyhow::bail!("inference_router.enabled requires inference_router.router");
    };
    if registry.inference_router(router_id).is_some() {
        return Ok(());
    }
    let available = registry
        .inference_routers
        .iter()
        .map(|router| router.id())
        .collect::<Vec<_>>()
        .join(", ");
    if available.is_empty() {
        anyhow::bail!("inference router {router_id:?} is not registered");
    }
    anyhow::bail!(
        "inference router {router_id:?} is not registered; available routers: {available}"
    );
}

fn model_supports_reasoning(model: &str, effort: &str) -> bool {
    lookup_model(model)
        .map(|entry| {
            entry
                .supported_reasoning
                .iter()
                .any(|option| option.effort == effort)
        })
        .unwrap_or(false)
}

fn is_final_answer_phase(phase: Option<&str>) -> bool {
    phase.is_none_or(|phase| phase.is_empty() || phase == FINAL_ANSWER_PHASE)
}

fn edit_tool_for_model<'a>(cfg: &'a RuntimeConfig, model: &'a str) -> Option<&'a str> {
    cfg.model_edit_tools
        .get(model)
        .map(String::as_str)
        .or_else(|| {
            cfg.model_profiles
                .get(model)
                .and_then(|profile| profile.edit_tool.as_deref())
        })
        .or_else(|| lookup_model(model).and_then(|entry| entry.edit_tool))
        .or(Some(EDIT_TOOL_EDIT))
}

fn model_profile_for_model(cfg: &RuntimeConfig, model: &str) -> Option<ModelHarnessProfile> {
    cfg.model_profiles
        .get(model)
        .cloned()
        .or_else(|| built_in_model_profile(model))
}

pub(crate) fn allowlist_permits(allowlist: &[String], tool_name: &str) -> bool {
    allowlist.is_empty() || allowlist.iter().any(|allowed| allowed == tool_name)
}

/// Provider-aware profile resolution for the active turn.
///
/// Many model ids are shared across providers (for example Cursor proxies
/// `claude-opus-4-8`). Resolving the harness profile by id alone picks the
/// first catalog entry, which assigns cross-provider ids the wrong family and
/// instruction overlay (e.g. a `cursor/claude-opus-4-8` turn would otherwise
/// inherit the Anthropic overlay). Prefer the explicit `(provider, model)`
/// catalog entry, keeping user-configured profile overrides as the top
/// precedence.
fn model_profile_for_provider_model(
    cfg: &RuntimeConfig,
    provider: &str,
    model: &str,
) -> Option<ModelHarnessProfile> {
    cfg.model_profiles
        .get(model)
        .cloned()
        .or_else(|| built_in_model_profile_for_provider(provider, model))
}

fn schema_policy_for_model(profile: Option<&ModelHarnessProfile>) -> ModelSchemaPolicy {
    profile
        .map(|profile| profile.schema_policy)
        .unwrap_or_default()
}

fn model_profile_segment_metadata(
    profile: Option<&ModelHarnessProfile>,
    provider: &str,
    model: &str,
    segment: &str,
) -> serde_json::Value {
    serde_json::json!({
        "kind": MODEL_PROFILE_TRACE_KIND,
        "segment": segment,
        "provider": provider,
        "model": model,
        "profileModel": profile.map(|profile| profile.model.as_str()).unwrap_or(model),
        "providerFamily": profile.map(|profile| profile.provider_family),
        "editTool": profile.and_then(|profile| profile.edit_tool.as_deref()),
        "schemaPolicy": profile.map(|profile| profile.schema_policy),
        "instructionOverlay": profile.map(|profile| profile.instruction_overlay),
        "parallelToolCalls": profile.and_then(|profile| profile.parallel_tool_calls),
        "autoCompactTokenLimit": profile.and_then(|profile| profile.auto_compact_token_limit),
    })
}

fn model_switch_summary(
    transcript: &[TranscriptItem],
    profile: Option<&ModelHarnessProfile>,
    provider: &str,
    model: &str,
    tools: &[roder_api::tools::ToolSpec],
) -> Option<String> {
    let previous = latest_model_profile_segment(transcript)?;
    let previous_model = previous
        .get("model")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let previous_provider = previous
        .get("provider")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if previous_model == model && previous_provider == provider {
        return None;
    }

    let previous_profile = previous
        .get("profileModel")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(previous_model);
    let current_profile = profile
        .map(|profile| profile.model.as_str())
        .unwrap_or(model);
    let previous_edit_tool = previous
        .get("editTool")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("none");
    let current_edit_tool = profile
        .and_then(|profile| profile.edit_tool.as_deref())
        .unwrap_or("none");
    let tool_names = tools
        .iter()
        .map(|tool| tool.name.as_str())
        .take(12)
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!(
        "{MODEL_SWITCH_SUMMARY_PREFIX} previous profile {previous_provider}/{previous_profile} used edit tool {previous_edit_tool}. Current profile {provider}/{current_profile} uses edit tool {current_edit_tool}. Available tools now: {}.",
        if tool_names.is_empty() {
            "none"
        } else {
            &tool_names
        }
    ))
}

fn latest_model_profile_segment(transcript: &[TranscriptItem]) -> Option<&serde_json::Value> {
    transcript.iter().rev().find_map(|item| {
        let TranscriptItem::ProviderMetadata(value) = item else {
            return None;
        };
        (value.get("kind").and_then(serde_json::Value::as_str) == Some(MODEL_PROFILE_TRACE_KIND))
            .then_some(value)
    })
}

pub fn validate_edit_tool(value: &str) -> anyhow::Result<()> {
    match value.trim() {
        EDIT_TOOL_PATCH | EDIT_TOOL_EDIT => Ok(()),
        _ => anyhow::bail!(
            "unsupported edit_tool {value:?}; allowed values: {EDIT_TOOL_PATCH}, {EDIT_TOOL_EDIT}"
        ),
    }
}

fn should_persist_thread_event(thread_id: &str) -> bool {
    !is_synthetic_event_thread_id(thread_id)
}

#[cfg(test)]
#[path = "runtime/codex_v2_tests.rs"]
mod codex_v2_tests;

#[cfg(test)]
#[path = "runtime/codex_v2_lifecycle_tests.rs"]
mod codex_v2_lifecycle_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream;
    use roder_api::catalog::{
        PROVIDER_MOCK, REASONING_HIGH, REASONING_LOW, REASONING_MEDIUM, REASONING_MINIMAL,
        REASONING_NONE, REASONING_XHIGH,
    };
    use roder_api::extension::ExtensionRegistryBuilder;
    use roder_api::inference::{
        CompletionMetadata, InferenceCapabilities, InferenceEngine, InferenceEventStream,
        InferenceProviderContext, InferenceTurnContext, MessageDelta, ModelDescriptor,
        ModelInstructionOverlay, ModelProfileReasoning, ModelSchemaPolicy, ProviderFamily,
        ReasoningEffortDescriptor,
    };
    use roder_api::inference_routing::{
        InferenceRouter, InferenceRoutingContext, InferenceRoutingDecision, InferenceRoutingOutcome,
    };
    use roder_api::thread::ThreadStoreFactory;
    use roder_api::tools::{ToolContributor, ToolExecutor, ToolSpec};
    use roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory;
    use std::sync::Mutex as StdMutex;

    fn test_workspace() -> String {
        std::env::current_dir().unwrap().display().to_string()
    }

    struct MetadataMissingStore;

    #[async_trait::async_trait]
    impl ThreadStore for MetadataMissingStore {
        fn id(&self) -> roder_api::thread::ThreadStoreId {
            "metadata-missing-store".to_string()
        }

        async fn create_thread(&self, metadata: ThreadMetadata) -> anyhow::Result<ThreadMetadata> {
            Ok(metadata)
        }

        async fn list_threads(&self) -> anyhow::Result<Vec<ThreadMetadata>> {
            Ok(Vec::new())
        }

        async fn load_thread(
            &self,
            _thread_id: &ThreadId,
        ) -> anyhow::Result<Option<ThreadSnapshot>> {
            Ok(Some(ThreadSnapshot {
                metadata: None,
                ..ThreadSnapshot::default()
            }))
        }

        async fn append_event(
            &self,
            _thread_id: &ThreadId,
            _envelope: &EventEnvelope,
        ) -> anyhow::Result<()> {
            Ok(())
        }
    }

    struct MetadataMissingStoreFactory;

    impl ThreadStoreFactory for MetadataMissingStoreFactory {
        fn id(&self) -> roder_api::thread::ThreadStoreId {
            "metadata-missing-store".to_string()
        }

        fn create(&self) -> Arc<dyn ThreadStore> {
            Arc::new(MetadataMissingStore)
        }
    }

    #[test]
    fn synthetic_app_server_events_are_not_thread_events() {
        for thread_id in ["app-server", "runtime", "thread-workflow"] {
            assert!(!should_persist_thread_event(thread_id));
        }
        assert!(should_persist_thread_event("thread-discovery"));
        assert!(should_persist_thread_event("thread-plan"));
        assert!(should_persist_thread_event("thread-process"));
        assert!(should_persist_thread_event("thread-1"));
    }

    #[test]
    fn server_side_compaction_uses_catalog_ninety_percent_default() {
        assert_eq!(
            server_side_compaction_threshold(&RuntimeConfig::default(), "gpt-5.5"),
            Some(945_000)
        );
        assert_eq!(
            server_side_compaction_threshold(&RuntimeConfig::default(), "gpt-5.3-codex-spark"),
            Some(115_200)
        );
    }

    #[test]
    fn server_side_compaction_respects_explicit_config_override() {
        let cfg = RuntimeConfig {
            auto_compact_token_limit: Some(123_456),
            ..RuntimeConfig::default()
        };

        assert_eq!(
            server_side_compaction_threshold(&cfg, "gpt-5.5"),
            Some(123_456)
        );
    }

    #[tokio::test]
    async fn pre_request_compaction_runs_when_server_side_model_is_at_context_window() {
        let captured = Arc::new(StdMutex::new(None));
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(CapturingEngine {
            request: captured.clone(),
        }));
        let thread_root = std::env::temp_dir().join(format!(
            "roder-pre-request-compaction-{}",
            uuid::Uuid::new_v4()
        ));
        builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
            base_path: thread_root.clone(),
        }));
        let runtime = Arc::new(
            Runtime::new(
                builder.build().unwrap(),
                RuntimeConfig {
                    default_provider: PROVIDER_MOCK.to_string(),
                    default_model: "gpt-5.5".to_string(),
                    file_backed_dynamic_context: true,
                    ..RuntimeConfig::default()
                },
            )
            .unwrap(),
        );
        let thread_id = runtime
            .create_thread(Some("Pre-request compaction".to_string()))
            .await
            .unwrap()
            .thread_id;
        let old_turn = "old-turn".to_string();
        runtime
            .persist_turn_item(
                &thread_id,
                &old_turn,
                &TranscriptItem::UserMessage(UserMessage::text("old context ".repeat(4_300_000))),
            )
            .await
            .unwrap();

        let mut events = runtime.subscribe_events();
        runtime
            .start_turn(StartTurnRequest {
                thread_id: thread_id.clone(),
                message: "continue".to_string(),
                images: Vec::new(),
                provider_override: None,
                model_override: None,
                reasoning_override: None,
                workspace: test_workspace(),
                instructions: InstructionBundle::default(),
                developer_context: None,
                task_ledger_required: false,
            })
            .await
            .unwrap();
        loop {
            let envelope = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
                .await
                .unwrap()
                .unwrap();
            if envelope.thread_id.as_deref() == Some(&thread_id)
                && matches!(envelope.event, RoderEvent::TurnCompleted(_))
            {
                break;
            }
        }

        let request = captured.lock().unwrap().clone().unwrap();
        assert!(
            matches!(
                request.transcript.first(),
                Some(TranscriptItem::ContextCompaction(_))
            ),
            "provider request should start with a local emergency compaction item"
        );
        assert!(
            request.transcript.len() < 4,
            "provider request should not replay the full oversized prior transcript: {:?}",
            request.transcript
        );

        let _ = std::fs::remove_dir_all(thread_root);
    }

    #[tokio::test]
    async fn continue_after_context_window_failure_compacts_before_provider_request() {
        let captured = Arc::new(StdMutex::new(None));
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(CapturingEngine {
            request: captured.clone(),
        }));
        let thread_root = std::env::temp_dir().join(format!(
            "roder-context-failure-continue-{}",
            uuid::Uuid::new_v4()
        ));
        builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
            base_path: thread_root.clone(),
        }));
        let runtime = Arc::new(
            Runtime::new(
                builder.build().unwrap(),
                RuntimeConfig {
                    default_provider: PROVIDER_MOCK.to_string(),
                    default_model: "gpt-5.5".to_string(),
                    file_backed_dynamic_context: true,
                    ..RuntimeConfig::default()
                },
            )
            .unwrap(),
        );
        let thread_id = runtime
            .create_thread(Some("Context failure continue".to_string()))
            .await
            .unwrap()
            .thread_id;
        let failed_turn = "failed-turn".to_string();
        runtime
            .persist_turn_item(
                &thread_id,
                &failed_turn,
                &TranscriptItem::UserMessage(UserMessage::text("old work ".repeat(10_000))),
            )
            .await
            .unwrap();
        runtime
            .persist_turn_item(
                &thread_id,
                &failed_turn,
                &TranscriptItem::Error(ErrorRecord {
                    message: "Your input exceeds the context window of this model. Please adjust your input and try again."
                        .to_string(),
                }),
            )
            .await
            .unwrap();

        let mut events = runtime.subscribe_events();
        runtime
            .start_turn(StartTurnRequest {
                thread_id: thread_id.clone(),
                message: "continue".to_string(),
                images: Vec::new(),
                provider_override: None,
                model_override: None,
                reasoning_override: None,
                workspace: test_workspace(),
                instructions: InstructionBundle::default(),
                developer_context: None,
                task_ledger_required: false,
            })
            .await
            .unwrap();
        loop {
            let envelope = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
                .await
                .unwrap()
                .unwrap();
            if envelope.thread_id.as_deref() == Some(&thread_id)
                && matches!(envelope.event, RoderEvent::TurnCompleted(_))
            {
                break;
            }
        }

        let request = captured.lock().unwrap().clone().unwrap();
        assert!(
            matches!(
                request.transcript.first(),
                Some(TranscriptItem::ContextCompaction(_))
            ),
            "provider request after context-window failure should start with local compaction"
        );
        assert!(
            request
                .transcript
                .iter()
                .any(|item| matches!(item, TranscriptItem::UserMessage(message) if message.text == "continue")),
            "current continue prompt must be preserved: {:?}",
            request.transcript
        );
        assert!(
            !request.transcript.iter().any(
                |item| matches!(item, TranscriptItem::Error(error) if error.message.contains("context window"))
            ),
            "raw prior context-window error should be summarized, not replayed: {:?}",
            request.transcript
        );

        let _ = std::fs::remove_dir_all(thread_root);
    }

    #[tokio::test]
    async fn workspace_for_thread_falls_back_when_metadata_is_missing() {
        let workspace = test_workspace();
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(FakeInferenceEngine));
        builder.thread_store_factory(Arc::new(MetadataMissingStoreFactory));
        let runtime = Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                workspace: Some(workspace.clone()),
                ..RuntimeConfig::default()
            },
        )
        .unwrap();

        let resolved = runtime
            .workspace_for_thread(&ThreadId::from("thread-workflow"))
            .await
            .unwrap();

        assert_eq!(resolved, workspace);
    }

    #[tokio::test]
    async fn automations_can_create_project_thread_with_model_overrides() {
        let runtime = Runtime::fake().unwrap();
        let workspace = std::env::temp_dir().join("project");
        let metadata = runtime
            .create_thread_with(CreateThreadRequest {
                title: Some("Automation: nightly status".to_string()),
                workspace: workspace.display().to_string(),
                workspace_id: None,
                root_id: None,
                provider: Some("mock".to_string()),
                model: Some("mock".to_string()),
                selection_mode: None,
                tool_allowlist: Vec::new(),
                developer_instructions: None,
                external_tools: Vec::new(),
                runner: None,
            })
            .await
            .unwrap();

        assert_eq!(
            metadata.title.as_deref(),
            Some("Automation: nightly status")
        );
        assert_eq!(metadata.workspace, workspace.display().to_string());
        assert_eq!(metadata.provider.as_deref(), Some("mock"));
        assert_eq!(metadata.model.as_deref(), Some("mock"));
    }

    #[tokio::test]
    async fn prior_provider_compaction_boundary_is_used_on_next_turn() {
        let captured = Arc::new(StdMutex::new(None));
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(CapturingEngine {
            request: captured.clone(),
        }));
        let thread_root = std::env::temp_dir().join(format!(
            "roder-provider-compaction-boundary-{}",
            uuid::Uuid::new_v4()
        ));
        builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
            base_path: thread_root.clone(),
        }));
        let runtime = Arc::new(
            Runtime::new(
                builder.build().unwrap(),
                RuntimeConfig {
                    default_provider: PROVIDER_MOCK.to_string(),
                    default_model: "gpt-5.5".to_string(),
                    ..RuntimeConfig::default()
                },
            )
            .unwrap(),
        );
        let thread_id = runtime
            .create_thread(Some("Provider compaction boundary".to_string()))
            .await
            .unwrap()
            .thread_id;
        let old_turn = "old-turn".to_string();
        runtime
            .persist_turn_item(
                &thread_id,
                &old_turn,
                &TranscriptItem::UserMessage(UserMessage::text(
                    "old history that must not be replayed after provider compaction",
                )),
            )
            .await
            .unwrap();
        runtime
            .persist_turn_item(
                &thread_id,
                &old_turn,
                &TranscriptItem::AssistantMessage(AssistantMessage {
                    text: "old answer".to_string(),
                    phase: None,
                }),
            )
            .await
            .unwrap();
        runtime
            .persist_turn_item(
                &thread_id,
                &old_turn,
                &TranscriptItem::ProviderMetadata(serde_json::json!({
                    "output": [{
                        "id": "cmp_1",
                        "type": "compaction",
                        "encrypted_content": "opaque-state"
                    }]
                })),
            )
            .await
            .unwrap();
        runtime
            .persist_turn_item(
                &thread_id,
                &old_turn,
                &TranscriptItem::AssistantMessage(AssistantMessage {
                    text: "after compact".to_string(),
                    phase: None,
                }),
            )
            .await
            .unwrap();

        let mut events = runtime.subscribe_events();
        runtime
            .start_turn(StartTurnRequest {
                thread_id: thread_id.clone(),
                message: "continue".to_string(),
                images: Vec::new(),
                provider_override: None,
                model_override: None,
                reasoning_override: None,
                workspace: test_workspace(),
                instructions: InstructionBundle::default(),
                developer_context: None,
                task_ledger_required: false,
            })
            .await
            .unwrap();
        loop {
            let envelope = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
                .await
                .unwrap()
                .unwrap();
            if envelope.thread_id.as_deref() == Some(&thread_id)
                && matches!(envelope.event, RoderEvent::TurnCompleted(_))
            {
                break;
            }
        }

        let request = captured.lock().unwrap().clone().unwrap();
        let compaction_idx = request.transcript.iter().position(|item| {
            matches!(
                item,
                TranscriptItem::ProviderMetadata(metadata)
                    if crate::compaction::provider_metadata_has_compaction(metadata)
            )
        });
        assert!(
            compaction_idx.is_some(),
            "next provider request should include the provider compaction item: {:?}",
            request.transcript
        );
        // Skill/context injectors may prepend non-history items, but no prior-turn
        // conversation may appear before the latest provider compaction boundary.
        let history_before_boundary = request
            .transcript
            .iter()
            .take(compaction_idx.unwrap())
            .any(|item| match item {
                TranscriptItem::AssistantMessage(_)
                | TranscriptItem::ToolCall(_)
                | TranscriptItem::ToolResult(_) => true,
                TranscriptItem::UserMessage(message) => {
                    !message.text.contains("<skills>") && message.text != "continue"
                }
                _ => false,
            });
        assert!(
            !history_before_boundary,
            "pre-provider-compaction conversation must not be replayed: {:?}",
            request.transcript
        );
        assert!(
            !request.transcript.iter().any(|item| {
                matches!(
                    item,
                    TranscriptItem::UserMessage(message)
                        if message.text.contains("old history that must not be replayed")
                )
            }),
            "pre-provider-compaction history must not be replayed: {:?}",
            request.transcript
        );
        assert!(
            request.transcript.iter().any(|item| {
                matches!(
                    item,
                    TranscriptItem::UserMessage(message) if message.text == "continue"
                )
            }),
            "current continue prompt must be preserved: {:?}",
            request.transcript
        );
        assert_eq!(
            request.runtime.auto_compact_token_limit,
            Some(945_000),
            "gpt-5.5 should keep server-side auto compaction configured"
        );

        let _ = std::fs::remove_dir_all(thread_root);
    }

    #[test]
    fn server_side_compaction_is_only_enabled_for_supported_models() {
        let cfg = RuntimeConfig {
            auto_compact_token_limit: Some(123_456),
            ..RuntimeConfig::default()
        };

        assert_eq!(server_side_compaction_threshold(&cfg, "mock"), None);
        assert_eq!(
            server_side_compaction_threshold(&cfg, "codex-auto-review"),
            None
        );
    }

    #[test]
    fn reasoning_is_disabled_for_models_without_reasoning_support() {
        let cfg = RuntimeConfig {
            reasoning: Some(REASONING_HIGH.to_string()),
            ..RuntimeConfig::default()
        };

        assert_eq!(
            effective_reasoning_for_model(&cfg, "claude-haiku-4-5-20251001"),
            REASONING_NONE
        );
        assert_eq!(
            reasoning_for_model(&cfg, "claude-haiku-4-5-20251001"),
            ReasoningConfig::default()
        );
    }

    #[test]
    fn unsupported_configured_reasoning_falls_back_to_model_default() {
        let cfg = RuntimeConfig {
            reasoning: Some(REASONING_MINIMAL.to_string()),
            ..RuntimeConfig::default()
        };

        assert_eq!(
            effective_reasoning_for_model(&cfg, "gpt-5.5"),
            REASONING_MEDIUM
        );
    }

    #[test]
    fn unsupported_configured_gemini_reasoning_is_rejected() {
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(std::sync::Arc::new(FakeInferenceEngine));

        let err = match Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                default_model: "gemini-3.5-flash".to_string(),
                reasoning: Some(REASONING_XHIGH.to_string()),
                ..RuntimeConfig::default()
            },
        ) {
            Ok(_) => panic!("expected unsupported Gemini reasoning to be rejected"),
            Err(err) => err,
        };

        assert!(
            err.to_string()
                .contains("model gemini-3.5-flash does not support reasoning effort xhigh")
        );
    }

    #[tokio::test]
    async fn selecting_none_for_non_reasoning_model_preserves_stored_preference() {
        let runtime = Runtime::new(
            Runtime::fake().unwrap().registry,
            RuntimeConfig {
                reasoning: Some(REASONING_HIGH.to_string()),
                ..RuntimeConfig::default()
            },
        )
        .unwrap();

        let cfg = runtime
            .select_provider(
                roder_api::catalog::PROVIDER_MOCK.to_string(),
                Some("claude-haiku-4-5-20251001".to_string()),
                Some(REASONING_NONE.to_string()),
            )
            .await
            .unwrap();

        assert_eq!(cfg.reasoning.as_deref(), Some(REASONING_HIGH));
        assert_eq!(runtime.effective_reasoning().await, REASONING_NONE);
    }

    #[tokio::test]
    async fn selecting_none_for_model_that_supports_none_updates_preference() {
        let runtime = Runtime::new(
            Runtime::fake().unwrap().registry,
            RuntimeConfig {
                reasoning: Some(REASONING_HIGH.to_string()),
                ..RuntimeConfig::default()
            },
        )
        .unwrap();

        let cfg = runtime
            .select_provider(
                roder_api::catalog::PROVIDER_MOCK.to_string(),
                Some("mock".to_string()),
                Some(REASONING_NONE.to_string()),
            )
            .await
            .unwrap();

        assert_eq!(cfg.reasoning.as_deref(), Some(REASONING_NONE));
    }

    #[test]
    fn parallel_tool_calls_default_on_with_model_override() {
        assert!(parallel_tool_calls_for_model(
            &RuntimeConfig::default(),
            "custom-model"
        ));

        let cfg = RuntimeConfig {
            model_parallel_tool_calls: std::collections::HashMap::from([(
                "custom-model".to_string(),
                false,
            )]),
            ..RuntimeConfig::default()
        };

        assert!(!parallel_tool_calls_for_model(&cfg, "custom-model"));
        assert!(parallel_tool_calls_for_model(&cfg, "other-model"));
    }

    #[test]
    fn profile_parallel_tool_calls_applies_between_config_and_default() {
        let cfg = RuntimeConfig {
            model_profiles: std::collections::HashMap::from([(
                "gpt-5.5".to_string(),
                test_model_profile("gpt-5.5"),
            )]),
            ..RuntimeConfig::default()
        };

        assert!(!parallel_tool_calls_for_model(&cfg, "gpt-5.5"));

        let cfg = RuntimeConfig {
            model_parallel_tool_calls: std::collections::HashMap::from([(
                "gpt-5.5".to_string(),
                true,
            )]),
            ..cfg
        };

        assert!(parallel_tool_calls_for_model(&cfg, "gpt-5.5"));
    }

    struct CapturingEngine {
        request: Arc<StdMutex<Option<AgentInferenceRequest>>>,
    }

    #[async_trait::async_trait]
    impl InferenceEngine for CapturingEngine {
        fn id(&self) -> String {
            roder_api::catalog::PROVIDER_MOCK.to_string()
        }

        fn capabilities(&self) -> InferenceCapabilities {
            InferenceCapabilities::coding_agent_default()
        }

        async fn list_models(
            &self,
            _ctx: InferenceProviderContext<'_>,
        ) -> anyhow::Result<Vec<roder_api::inference::ModelDescriptor>> {
            Ok(roder_api::catalog::models_for_provider(
                roder_api::catalog::PROVIDER_MOCK,
                true,
            ))
        }

        async fn stream_turn(
            &self,
            _ctx: InferenceTurnContext<'_>,
            request: AgentInferenceRequest,
        ) -> anyhow::Result<InferenceEventStream> {
            *self.request.lock().unwrap() = Some(request);
            Ok(Box::pin(stream::iter(vec![
                Ok(InferenceEvent::MessageDelta(MessageDelta {
                    text: "done".to_string(),
                    phase: None,
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("stop".to_string()),
                    provider_response_id: None,
                })),
            ])))
        }
    }

    struct RoutingCaptureEngine {
        id: &'static str,
        models: Vec<ModelDescriptor>,
        requests: Arc<StdMutex<Vec<AgentInferenceRequest>>>,
    }

    #[async_trait::async_trait]
    impl InferenceEngine for RoutingCaptureEngine {
        fn id(&self) -> String {
            self.id.to_string()
        }

        fn capabilities(&self) -> InferenceCapabilities {
            InferenceCapabilities::coding_agent_default()
        }

        async fn list_models(
            &self,
            _ctx: InferenceProviderContext<'_>,
        ) -> anyhow::Result<Vec<ModelDescriptor>> {
            Ok(self.models.clone())
        }

        async fn stream_turn(
            &self,
            _ctx: InferenceTurnContext<'_>,
            request: AgentInferenceRequest,
        ) -> anyhow::Result<InferenceEventStream> {
            self.requests.lock().unwrap().push(request);
            Ok(Box::pin(stream::iter(vec![
                Ok(InferenceEvent::MessageDelta(MessageDelta {
                    text: "routed".to_string(),
                    phase: None,
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("stop".to_string()),
                    provider_response_id: None,
                })),
            ])))
        }
    }

    struct StaticRouter {
        id: &'static str,
        decision: InferenceRoutingDecision,
        contexts: Arc<StdMutex<Vec<InferenceRoutingContext>>>,
    }

    #[async_trait::async_trait]
    impl InferenceRouter for StaticRouter {
        fn id(&self) -> String {
            self.id.to_string()
        }

        async fn route(
            &self,
            context: InferenceRoutingContext,
        ) -> anyhow::Result<InferenceRoutingDecision> {
            self.contexts.lock().unwrap().push(context);
            Ok(self.decision.clone())
        }
    }

    fn routing_test_model(id: &str, supported_reasoning: &[&str]) -> ModelDescriptor {
        ModelDescriptor {
            id: id.to_string(),
            name: id.to_string(),
            context_window: Some(128_000),
            default_reasoning: supported_reasoning
                .first()
                .map(|effort| (*effort).to_string()),
            supported_reasoning: supported_reasoning
                .iter()
                .map(|effort| ReasoningEffortDescriptor {
                    effort: (*effort).to_string(),
                    description: format!("{effort} reasoning"),
                })
                .collect(),
        }
    }

    struct TaskLedgerCompletionGateEngine {
        calls: StdMutex<u32>,
        requests: Arc<StdMutex<Vec<AgentInferenceRequest>>>,
    }

    #[async_trait::async_trait]
    impl InferenceEngine for TaskLedgerCompletionGateEngine {
        fn id(&self) -> String {
            roder_api::catalog::PROVIDER_MOCK.to_string()
        }

        fn capabilities(&self) -> InferenceCapabilities {
            InferenceCapabilities::coding_agent_default()
        }

        async fn list_models(
            &self,
            _ctx: InferenceProviderContext<'_>,
        ) -> anyhow::Result<Vec<roder_api::inference::ModelDescriptor>> {
            Ok(roder_api::catalog::models_for_provider(
                roder_api::catalog::PROVIDER_MOCK,
                true,
            ))
        }

        async fn stream_turn(
            &self,
            _ctx: InferenceTurnContext<'_>,
            request: AgentInferenceRequest,
        ) -> anyhow::Result<InferenceEventStream> {
            self.requests.lock().unwrap().push(request);
            let mut calls = self.calls.lock().unwrap();
            *calls += 1;
            let events = match *calls {
                1 => vec![Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "ledger-open".to_string(),
                    name: TASK_LEDGER_TOOL_NAME.to_string(),
                    arguments: serde_json::json!({
                        "tasks": [
                            {
                                "id": "inspect",
                                "content": "Inspect local assets",
                                "status": "completed",
                                "evidence": "listed workspace"
                            },
                            {
                                "id": "write",
                                "content": "Write /app/result.txt",
                                "status": "pending"
                            }
                        ],
                        "requireCompletionEvidence": true
                    })
                    .to_string(),
                }))],
                3 => vec![Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "ledger-complete".to_string(),
                    name: TASK_LEDGER_TOOL_NAME.to_string(),
                    arguments: serde_json::json!({
                        "tasks": [
                            {
                                "id": "inspect",
                                "content": "Inspect local assets",
                                "status": "completed",
                                "evidence": "listed workspace"
                            },
                            {
                                "id": "write",
                                "content": "Write /app/result.txt",
                                "status": "completed",
                                "evidence": "wrote answer"
                            }
                        ],
                        "requireCompletionEvidence": true
                    })
                    .to_string(),
                }))],
                _ => vec![Ok(InferenceEvent::MessageDelta(MessageDelta {
                    text: "final".to_string(),
                    phase: None,
                }))],
            };
            Ok(Box::pin(stream::iter(events.into_iter().chain(
                std::iter::once(Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("stop".to_string()),
                    provider_response_id: None,
                }))),
            ))))
        }
    }

    struct VerificationGateEngine {
        calls: StdMutex<u32>,
    }

    #[async_trait::async_trait]
    impl InferenceEngine for VerificationGateEngine {
        fn id(&self) -> String {
            roder_api::catalog::PROVIDER_MOCK.to_string()
        }

        fn capabilities(&self) -> InferenceCapabilities {
            InferenceCapabilities::coding_agent_default()
        }

        async fn list_models(
            &self,
            _ctx: InferenceProviderContext<'_>,
        ) -> anyhow::Result<Vec<roder_api::inference::ModelDescriptor>> {
            Ok(roder_api::catalog::models_for_provider(
                roder_api::catalog::PROVIDER_MOCK,
                true,
            ))
        }

        async fn stream_turn(
            &self,
            _ctx: InferenceTurnContext<'_>,
            request: AgentInferenceRequest,
        ) -> anyhow::Result<InferenceEventStream> {
            let mut calls = self.calls.lock().unwrap();
            *calls += 1;
            let events = match *calls {
                1 => vec![Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "write-1".to_string(),
                    name: "write_file".to_string(),
                    arguments: serde_json::json!({
                        "path": "src/lib.rs",
                        "content": "pub fn answer() -> u8 { 42 }\n"
                    })
                    .to_string(),
                }))],
                2 => vec![Ok(InferenceEvent::MessageDelta(MessageDelta {
                    text: "done too early".to_string(),
                    phase: None,
                }))],
                3 if request.transcript.iter().any(|item| {
                    matches!(
                        item,
                        TranscriptItem::UserMessage(message)
                            if message.text.contains("Verification gate blocked final completion")
                    )
                }) =>
                {
                    vec![Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                        id: "verify-1".to_string(),
                        name: crate::verification_gate::VERIFICATION_TOOL_NAME.to_string(),
                        arguments: serde_json::json!({
                            "originalTask": "write code",
                            "changedFiles": ["src/lib.rs"],
                            "toolEvidence": ["write_file wrote src/lib.rs"],
                            "testsRun": ["cargo test -p roder-core verification_gate"],
                            "openGaps": [],
                            "status": "completed"
                        })
                        .to_string(),
                    }))]
                }
                _ => vec![Ok(InferenceEvent::MessageDelta(MessageDelta {
                    text: "verified final".to_string(),
                    phase: None,
                }))],
            };
            Ok(Box::pin(stream::iter(events.into_iter().chain(
                std::iter::once(Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("stop".to_string()),
                    provider_response_id: None,
                }))),
            ))))
        }
    }

    struct SpeedPolicyEngine {
        calls: StdMutex<u32>,
        requests: Arc<StdMutex<Vec<AgentInferenceRequest>>>,
    }

    #[async_trait::async_trait]
    impl InferenceEngine for SpeedPolicyEngine {
        fn id(&self) -> String {
            roder_api::catalog::PROVIDER_MOCK.to_string()
        }

        fn capabilities(&self) -> InferenceCapabilities {
            InferenceCapabilities::coding_agent_default()
        }

        async fn list_models(
            &self,
            _ctx: InferenceProviderContext<'_>,
        ) -> anyhow::Result<Vec<roder_api::inference::ModelDescriptor>> {
            Ok(roder_api::catalog::models_for_provider(
                roder_api::catalog::PROVIDER_MOCK,
                true,
            ))
        }

        async fn stream_turn(
            &self,
            _ctx: InferenceTurnContext<'_>,
            request: AgentInferenceRequest,
        ) -> anyhow::Result<InferenceEventStream> {
            self.requests.lock().unwrap().push(request.clone());
            let mut calls = self.calls.lock().unwrap();
            *calls += 1;
            let events = match *calls {
                1 => vec![Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "write-1".to_string(),
                    name: "write_file".to_string(),
                    arguments: serde_json::json!({
                        "path": "src/lib.rs",
                        "content": "pub fn answer() -> u8 { 42 }\n"
                    })
                    .to_string(),
                }))],
                3 if request.transcript.iter().any(|item| {
                    matches!(
                        item,
                        TranscriptItem::UserMessage(message)
                            if message.text.contains("Verification gate blocked final completion")
                    )
                }) =>
                {
                    vec![Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                        id: "verify-1".to_string(),
                        name: crate::verification_gate::VERIFICATION_TOOL_NAME.to_string(),
                        arguments: serde_json::json!({
                            "originalTask": "write code",
                            "changedFiles": ["src/lib.rs"],
                            "toolEvidence": ["write_file wrote src/lib.rs"],
                            "testsRun": ["cargo test -p roder-core speed_policy"],
                            "openGaps": [],
                            "status": "completed"
                        })
                        .to_string(),
                    }))]
                }
                _ => vec![Ok(InferenceEvent::MessageDelta(MessageDelta {
                    text: "done".to_string(),
                    phase: None,
                }))],
            };
            Ok(Box::pin(stream::iter(events.into_iter().chain(
                std::iter::once(Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("stop".to_string()),
                    provider_response_id: None,
                }))),
            ))))
        }
    }

    struct SwitchCaptureEngine {
        requests: Arc<StdMutex<Vec<AgentInferenceRequest>>>,
    }

    #[async_trait::async_trait]
    impl InferenceEngine for SwitchCaptureEngine {
        fn id(&self) -> String {
            roder_api::catalog::PROVIDER_MOCK.to_string()
        }

        fn capabilities(&self) -> InferenceCapabilities {
            InferenceCapabilities::coding_agent_default()
        }

        async fn list_models(
            &self,
            _ctx: InferenceProviderContext<'_>,
        ) -> anyhow::Result<Vec<roder_api::inference::ModelDescriptor>> {
            Ok(roder_api::catalog::models_for_provider(
                roder_api::catalog::PROVIDER_MOCK,
                true,
            ))
        }

        async fn stream_turn(
            &self,
            _ctx: InferenceTurnContext<'_>,
            request: AgentInferenceRequest,
        ) -> anyhow::Result<InferenceEventStream> {
            self.requests.lock().unwrap().push(request);
            Ok(Box::pin(stream::iter(vec![
                Ok(InferenceEvent::MessageDelta(MessageDelta {
                    text: "done".to_string(),
                    phase: None,
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("stop".to_string()),
                    provider_response_id: None,
                })),
            ])))
        }
    }

    struct DeadlineEngine;

    #[async_trait::async_trait]
    impl InferenceEngine for DeadlineEngine {
        fn id(&self) -> String {
            roder_api::catalog::PROVIDER_MOCK.to_string()
        }

        fn capabilities(&self) -> InferenceCapabilities {
            InferenceCapabilities::coding_agent_default()
        }

        async fn list_models(
            &self,
            _ctx: InferenceProviderContext<'_>,
        ) -> anyhow::Result<Vec<roder_api::inference::ModelDescriptor>> {
            Ok(Vec::new())
        }

        async fn stream_turn(
            &self,
            _ctx: InferenceTurnContext<'_>,
            _request: AgentInferenceRequest,
        ) -> anyhow::Result<InferenceEventStream> {
            Ok(Box::pin(stream::once(async {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                Ok(InferenceEvent::MessageDelta(MessageDelta {
                    text: "too late".to_string(),
                    phase: None,
                }))
            })))
        }
    }

    struct WriteFileContributor;

    impl ToolContributor for WriteFileContributor {
        fn id(&self) -> String {
            "test-write".to_string()
        }

        fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
            registry.register(Arc::new(WriteFileTool))
        }
    }

    struct WriteFileTool;

    #[async_trait::async_trait]
    impl ToolExecutor for WriteFileTool {
        fn spec(&self) -> ToolSpec {
            ToolSpec {
                name: "write_file".to_string(),
                description: "Write a test file.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "content": { "type": "string" }
                    },
                    "required": ["path", "content"],
                    "additionalProperties": false
                }),
            }
        }

        async fn execute(
            &self,
            _ctx: ToolExecutionContext,
            call: ToolCall,
        ) -> anyhow::Result<ToolResult> {
            let path = call
                .arguments
                .get("path")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("src/lib.rs");
            Ok(ToolResult {
                id: call.id,
                name: call.name,
                text: format!("wrote {path}"),
                data: serde_json::json!({ "path": path }),
                is_error: false,
            })
        }
    }

    struct ProfileToolContributor;

    impl ToolContributor for ProfileToolContributor {
        fn id(&self) -> String {
            "profile-tools".to_string()
        }

        fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
            for name in ["apply_patch", "edit", "multi_edit", "write_file"] {
                registry.register(Arc::new(ProfileTool {
                    name: name.to_string(),
                }))?;
            }
            Ok(())
        }
    }

    struct ProfileTool {
        name: String,
    }

    #[async_trait::async_trait]
    impl ToolExecutor for ProfileTool {
        fn spec(&self) -> ToolSpec {
            ToolSpec {
                name: self.name.clone(),
                description: format!("{} test tool", self.name),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "content": { "type": "string" }
                    },
                    "required": ["path", "content"],
                    "additionalProperties": false
                }),
            }
        }

        async fn execute(
            &self,
            _ctx: ToolExecutionContext,
            call: ToolCall,
        ) -> anyhow::Result<ToolResult> {
            Ok(ToolResult {
                id: call.id,
                name: call.name,
                text: "ok".to_string(),
                data: serde_json::json!({}),
                is_error: false,
            })
        }
    }

    fn test_model_profile(model: &str) -> ModelHarnessProfile {
        ModelHarnessProfile {
            model: model.to_string(),
            provider: roder_api::catalog::PROVIDER_OPENAI.to_string(),
            provider_family: ProviderFamily::OpenAi,
            edit_tool: Some(EDIT_TOOL_EDIT.to_string()),
            schema_policy: ModelSchemaPolicy::StandardRequiredFirst,
            instruction_overlay: ModelInstructionOverlay::IntuitiveContext,
            reasoning: ModelProfileReasoning {
                orientation: Some(REASONING_LOW.to_string()),
                execution: Some(REASONING_LOW.to_string()),
                verification: Some(REASONING_LOW.to_string()),
                recovery: Some(REASONING_LOW.to_string()),
            },
            parallel_tool_calls: Some(false),
            auto_compact_token_limit: Some(123_000),
        }
    }

    async fn captured_profile_request(cfg: RuntimeConfig) -> AgentInferenceRequest {
        let captured = Arc::new(StdMutex::new(None));
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(CapturingEngine {
            request: captured.clone(),
        }));
        builder.tool_contributor(Arc::new(ProfileToolContributor));
        let runtime = Arc::new(Runtime::new(builder.build().unwrap(), cfg).unwrap());
        let mut rx = runtime.subscribe_events();
        let turn_id = runtime
            .start_turn(StartTurnRequest {
                thread_id: "thread-model-profile".to_string(),
                message: "use profile knobs".to_string(),
                images: Vec::new(),
                provider_override: None,
                model_override: None,
                reasoning_override: None,
                workspace: test_workspace(),
                instructions: InstructionBundle {
                    system: None,
                    developer: Some("base developer".to_string()),
                    developer_context: None,
                },
                developer_context: None,
                task_ledger_required: false,
            })
            .await
            .unwrap();

        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let envelope = rx.recv().await.unwrap();
                if envelope.turn_id.as_deref() != Some(&turn_id) {
                    continue;
                }
                match envelope.event {
                    RoderEvent::TurnCompleted(_) => break,
                    RoderEvent::TurnFailed(event) => panic!("turn failed: {}", event.error),
                    _ => {}
                }
            }
        })
        .await
        .unwrap();

        captured.lock().unwrap().clone().unwrap()
    }

    struct ToolThenStopEngine {
        calls: StdMutex<u32>,
    }

    #[async_trait::async_trait]
    impl InferenceEngine for ToolThenStopEngine {
        fn id(&self) -> String {
            roder_api::catalog::PROVIDER_MOCK.to_string()
        }

        fn capabilities(&self) -> InferenceCapabilities {
            InferenceCapabilities::coding_agent_default()
        }

        async fn list_models(
            &self,
            _ctx: InferenceProviderContext<'_>,
        ) -> anyhow::Result<Vec<roder_api::inference::ModelDescriptor>> {
            Ok(roder_api::catalog::models_for_provider(
                roder_api::catalog::PROVIDER_MOCK,
                true,
            ))
        }

        async fn stream_turn(
            &self,
            _ctx: InferenceTurnContext<'_>,
            _request: AgentInferenceRequest,
        ) -> anyhow::Result<InferenceEventStream> {
            let mut calls = self.calls.lock().unwrap();
            *calls += 1;
            let events = match *calls {
                1 => vec![
                    Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                        id: "write-1".to_string(),
                        name: "write_file".to_string(),
                        arguments: serde_json::json!({
                            "path": "src/lib.rs",
                            "content": "pub fn answer() -> u8 { 42 }\n"
                        })
                        .to_string(),
                    })),
                    Ok(InferenceEvent::Completed(CompletionMetadata {
                        stop_reason: Some("tool_use".to_string()),
                        provider_response_id: None,
                    })),
                ],
                _ => vec![
                    Ok(InferenceEvent::MessageDelta(MessageDelta {
                        text: "final".to_string(),
                        phase: None,
                    })),
                    Ok(InferenceEvent::Completed(CompletionMetadata {
                        stop_reason: Some("end_turn".to_string()),
                        provider_response_id: None,
                    })),
                ],
            };
            Ok(Box::pin(stream::iter(events)))
        }
    }

    #[tokio::test]
    async fn turn_completed_reports_terminal_step_finish_reason() {
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(ToolThenStopEngine {
            calls: StdMutex::new(0),
        }));
        builder.tool_contributor(Arc::new(WriteFileContributor));
        let runtime = Arc::new(
            Runtime::new(
                builder.build().unwrap(),
                RuntimeConfig {
                    policy_mode: PolicyMode::Bypass,
                    agent_swarm_mode: false,
                    ..RuntimeConfig::default()
                },
            )
            .unwrap(),
        );
        let mut rx = runtime.subscribe_events();
        let turn_id = runtime
            .start_turn(StartTurnRequest {
                thread_id: "thread-finish-reason".to_string(),
                message: "write then finish".to_string(),
                images: Vec::new(),
                provider_override: None,
                model_override: None,
                reasoning_override: None,
                workspace: test_workspace(),
                instructions: InstructionBundle {
                    system: None,
                    developer: None,
                    developer_context: None,
                },
                developer_context: None,
                task_ledger_required: false,
            })
            .await
            .unwrap();

        let completed = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let envelope = rx.recv().await.unwrap();
                if envelope.turn_id.as_deref() != Some(&turn_id) {
                    continue;
                }
                match envelope.event {
                    RoderEvent::TurnCompleted(event) => break event,
                    RoderEvent::TurnFailed(event) => panic!("turn failed: {}", event.error),
                    _ => {}
                }
            }
        })
        .await
        .unwrap();

        // The mid-turn tool_use stop reason must not leak; the terminal
        // end_turn step decides the turn's finish reason.
        assert_eq!(completed.finish_reason.as_deref(), Some("stop"));
    }

    #[tokio::test]
    async fn inference_router_selection_changes_request_model_and_records_event() {
        let default_requests = Arc::new(StdMutex::new(Vec::<AgentInferenceRequest>::new()));
        let routed_requests = Arc::new(StdMutex::new(Vec::<AgentInferenceRequest>::new()));
        let contexts = Arc::new(StdMutex::new(Vec::<InferenceRoutingContext>::new()));
        let selected = ModelSelection {
            provider: "routed-provider".to_string(),
            model: "routed-model".to_string(),
        };
        let default = ModelSelection {
            provider: roder_api::catalog::PROVIDER_MOCK.to_string(),
            model: "mock".to_string(),
        };
        let decision = InferenceRoutingDecision {
            reasoning: Some(ReasoningConfig {
                enabled: true,
                level: Some(REASONING_LOW.to_string()),
            }),
            confidence: Some(0.91),
            baseline: Some(default.clone()),
            matched_signals: vec![roder_api::inference_routing::InferenceRoutingSignal::new(
                "intent", "routine",
            )],
            ..InferenceRoutingDecision::selected("test-router", selected.clone(), "routine request")
        };

        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(RoutingCaptureEngine {
            id: roder_api::catalog::PROVIDER_MOCK,
            models: vec![routing_test_model("mock", &[REASONING_LOW])],
            requests: default_requests.clone(),
        }));
        builder.inference_engine(Arc::new(RoutingCaptureEngine {
            id: "routed-provider",
            models: vec![routing_test_model(
                "routed-model",
                &[REASONING_LOW, REASONING_MEDIUM],
            )],
            requests: routed_requests.clone(),
        }));
        builder.inference_router(Arc::new(StaticRouter {
            id: "test-router",
            decision,
            contexts: contexts.clone(),
        }));
        let thread_root =
            std::env::temp_dir().join(format!("roder-routing-auto-{}", uuid::Uuid::new_v4()));
        builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
            base_path: thread_root.clone(),
        }));
        let runtime = Arc::new(
            Runtime::new(
                builder.build().unwrap(),
                RuntimeConfig {
                    default_provider: default.provider.clone(),
                    default_model: default.model.clone(),
                    ..RuntimeConfig::default()
                },
            )
            .unwrap(),
        );
        let thread_id = runtime
            .create_thread_with(CreateThreadRequest {
                title: Some("Routing auto".to_string()),
                workspace: test_workspace(),
                workspace_id: None,
                root_id: None,
                provider: Some(default.provider.clone()),
                model: Some(default.model.clone()),
                tool_allowlist: Vec::new(),
                developer_instructions: None,
                external_tools: Vec::new(),
                selection_mode: Some(ModelSelectionMode::auto(
                    "test-router:coding",
                    "test-router",
                    "Auto: Coding",
                    default.clone(),
                    Some("coding".to_string()),
                    None,
                )),
                runner: None,
            })
            .await
            .unwrap()
            .thread_id;
        let mut rx = runtime.subscribe_events();
        let turn_id = runtime
            .start_turn(StartTurnRequest {
                thread_id: thread_id.clone(),
                message: "small cleanup".to_string(),
                images: Vec::new(),
                provider_override: None,
                model_override: None,
                reasoning_override: None,
                workspace: test_workspace(),
                instructions: InstructionBundle::default(),
                developer_context: None,
                task_ledger_required: false,
            })
            .await
            .unwrap();

        let mut routing_event = None;
        let mut inference_started = None;
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let envelope = rx.recv().await.unwrap();
                if envelope.turn_id.as_deref() != Some(&turn_id) {
                    continue;
                }
                match envelope.event {
                    RoderEvent::InferenceRoutingDecision(event) => {
                        routing_event = Some(event);
                    }
                    RoderEvent::InferenceStarted(event) => {
                        inference_started = Some(event);
                    }
                    RoderEvent::TurnCompleted(_) => break,
                    RoderEvent::TurnFailed(event) => panic!("turn failed: {}", event.error),
                    _ => {}
                }
            }
        })
        .await
        .unwrap();

        assert!(default_requests.lock().unwrap().is_empty());
        let routed_requests = routed_requests.lock().unwrap();
        assert_eq!(routed_requests.len(), 1);
        assert_eq!(routed_requests[0].model, selected);
        assert_eq!(
            routed_requests[0].reasoning.level.as_deref(),
            Some(REASONING_LOW)
        );
        assert_eq!(
            routed_requests[0].metadata["inferenceRouting"]["outcome"],
            "selected"
        );

        let routing_event = routing_event.expect("routing decision event");
        assert_eq!(routing_event.default_selection, default);
        assert_eq!(routing_event.selected_selection, selected);
        assert_eq!(
            routing_event.decision.outcome,
            InferenceRoutingOutcome::Selected
        );
        assert_eq!(
            inference_started.expect("inference started event").model,
            selected
        );

        let contexts = contexts.lock().unwrap();
        assert_eq!(contexts.len(), 1);
        assert_eq!(contexts[0].default_selection, default);
        assert_eq!(contexts[0].candidates.len(), 2);
        assert!(
            contexts[0]
                .signals
                .iter()
                .any(|signal| signal.key == "profile" && signal.value == "coding")
        );
        let _ = std::fs::remove_dir_all(thread_root);
    }

    #[tokio::test]
    async fn inference_router_is_bypassed_for_explicit_selection() {
        let requests = Arc::new(StdMutex::new(Vec::<AgentInferenceRequest>::new()));
        let contexts = Arc::new(StdMutex::new(Vec::<InferenceRoutingContext>::new()));
        let selected = ModelSelection {
            provider: roder_api::catalog::PROVIDER_MOCK.to_string(),
            model: "mock".to_string(),
        };

        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(RoutingCaptureEngine {
            id: roder_api::catalog::PROVIDER_MOCK,
            models: vec![routing_test_model("mock", &[REASONING_LOW])],
            requests: requests.clone(),
        }));
        builder.inference_router(Arc::new(StaticRouter {
            id: "test-router",
            decision: InferenceRoutingDecision::selected(
                "test-router",
                ModelSelection {
                    provider: "missing".to_string(),
                    model: "missing".to_string(),
                },
                "would route if called",
            ),
            contexts: contexts.clone(),
        }));
        let thread_root =
            std::env::temp_dir().join(format!("roder-routing-explicit-{}", uuid::Uuid::new_v4()));
        builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
            base_path: thread_root.clone(),
        }));
        let runtime = Arc::new(
            Runtime::new(
                builder.build().unwrap(),
                RuntimeConfig {
                    default_provider: selected.provider.clone(),
                    default_model: selected.model.clone(),
                    inference_router: RuntimeInferenceRouterConfig {
                        enabled: true,
                        router_id: Some("test-router".to_string()),
                    },
                    ..RuntimeConfig::default()
                },
            )
            .unwrap(),
        );
        let thread_id = runtime
            .create_thread_with(CreateThreadRequest {
                title: Some("Routing explicit".to_string()),
                workspace: test_workspace(),
                workspace_id: None,
                root_id: None,
                provider: Some(selected.provider.clone()),
                model: Some(selected.model.clone()),
                tool_allowlist: Vec::new(),
                developer_instructions: None,
                external_tools: Vec::new(),
                selection_mode: Some(ModelSelectionMode::auto(
                    "test-router:default",
                    "test-router",
                    "Auto",
                    selected.clone(),
                    None,
                    None,
                )),
                runner: None,
            })
            .await
            .unwrap()
            .thread_id;
        let mut rx = runtime.subscribe_events();
        let turn_id = runtime
            .start_turn(StartTurnRequest {
                thread_id,
                message: "use explicit selection".to_string(),
                images: Vec::new(),
                provider_override: Some(selected.provider.clone()),
                model_override: Some(selected.model.clone()),
                reasoning_override: None,
                workspace: test_workspace(),
                instructions: InstructionBundle::default(),
                developer_context: None,
                task_ledger_required: false,
            })
            .await
            .unwrap();

        let mut saw_routing_event = false;
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let envelope = rx.recv().await.unwrap();
                if envelope.turn_id.as_deref() != Some(&turn_id) {
                    continue;
                }
                match envelope.event {
                    RoderEvent::InferenceRoutingDecision(_) => {
                        saw_routing_event = true;
                    }
                    RoderEvent::TurnCompleted(_) => break,
                    RoderEvent::TurnFailed(event) => panic!("turn failed: {}", event.error),
                    _ => {}
                }
            }
        })
        .await
        .unwrap();

        assert!(!saw_routing_event);
        assert!(contexts.lock().unwrap().is_empty());
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].model, selected);
        let _ = std::fs::remove_dir_all(thread_root);
    }

    #[tokio::test]
    async fn inference_router_is_bypassed_for_manual_selection_mode() {
        let requests = Arc::new(StdMutex::new(Vec::<AgentInferenceRequest>::new()));
        let contexts = Arc::new(StdMutex::new(Vec::<InferenceRoutingContext>::new()));
        let selected = ModelSelection {
            provider: roder_api::catalog::PROVIDER_MOCK.to_string(),
            model: "mock".to_string(),
        };

        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(RoutingCaptureEngine {
            id: roder_api::catalog::PROVIDER_MOCK,
            models: vec![routing_test_model("mock", &[REASONING_LOW])],
            requests: requests.clone(),
        }));
        builder.inference_router(Arc::new(StaticRouter {
            id: "test-router",
            decision: InferenceRoutingDecision::selected(
                "test-router",
                ModelSelection {
                    provider: "missing".to_string(),
                    model: "missing".to_string(),
                },
                "would route if called",
            ),
            contexts: contexts.clone(),
        }));
        let thread_root =
            std::env::temp_dir().join(format!("roder-routing-manual-{}", uuid::Uuid::new_v4()));
        builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
            base_path: thread_root.clone(),
        }));
        let runtime = Arc::new(
            Runtime::new(
                builder.build().unwrap(),
                RuntimeConfig {
                    default_provider: selected.provider.clone(),
                    default_model: selected.model.clone(),
                    inference_router: RuntimeInferenceRouterConfig {
                        enabled: true,
                        router_id: Some("test-router".to_string()),
                    },
                    ..RuntimeConfig::default()
                },
            )
            .unwrap(),
        );
        let thread_id = runtime
            .create_thread_with(CreateThreadRequest {
                title: Some("Routing manual".to_string()),
                workspace: test_workspace(),
                workspace_id: None,
                root_id: None,
                provider: Some(selected.provider.clone()),
                model: Some(selected.model.clone()),
                tool_allowlist: Vec::new(),
                developer_instructions: None,
                external_tools: Vec::new(),
                selection_mode: Some(ModelSelectionMode::manual(
                    selected.provider.clone(),
                    selected.model.clone(),
                    None,
                )),
                runner: None,
            })
            .await
            .unwrap()
            .thread_id;
        let mut rx = runtime.subscribe_events();
        let turn_id = runtime
            .start_turn(StartTurnRequest {
                thread_id,
                message: "use selected manual model".to_string(),
                images: Vec::new(),
                provider_override: None,
                model_override: None,
                reasoning_override: None,
                workspace: test_workspace(),
                instructions: InstructionBundle::default(),
                developer_context: None,
                task_ledger_required: false,
            })
            .await
            .unwrap();

        let mut saw_routing_event = false;
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let envelope = rx.recv().await.unwrap();
                if envelope.turn_id.as_deref() != Some(&turn_id) {
                    continue;
                }
                match envelope.event {
                    RoderEvent::InferenceRoutingDecision(_) => {
                        saw_routing_event = true;
                    }
                    RoderEvent::TurnCompleted(_) => break,
                    RoderEvent::TurnFailed(event) => panic!("turn failed: {}", event.error),
                    _ => {}
                }
            }
        })
        .await
        .unwrap();

        assert!(!saw_routing_event);
        assert!(contexts.lock().unwrap().is_empty());
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].model, selected);
        let _ = std::fs::remove_dir_all(thread_root);
    }

    #[test]
    fn enabled_inference_router_requires_registered_router() {
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(FakeInferenceEngine));

        let err = match Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                inference_router: RuntimeInferenceRouterConfig {
                    enabled: true,
                    router_id: Some("missing-router".to_string()),
                },
                ..RuntimeConfig::default()
            },
        ) {
            Ok(_) => panic!("runtime should reject unknown inference router"),
            Err(err) => err,
        };

        assert!(
            err.to_string()
                .contains("inference router \"missing-router\" is not registered")
        );
    }

    #[tokio::test]
    async fn model_profile_routes_request_knobs_to_next_inference() {
        let request = captured_profile_request(RuntimeConfig {
            default_provider: roder_api::catalog::PROVIDER_MOCK.to_string(),
            default_model: "gpt-5.5".to_string(),
            model_profiles: std::collections::HashMap::from([(
                "gpt-5.5".to_string(),
                test_model_profile("gpt-5.5"),
            )]),
            ..RuntimeConfig::default()
        })
        .await;

        let tool_names = request
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert!(tool_names.contains(&"apply_patch"));
        assert!(tool_names.contains(&"edit"));
        assert!(tool_names.contains(&"multi_edit"));
        assert!(tool_names.contains(&"write_file"));
        assert_eq!(request.reasoning.level.as_deref(), Some(REASONING_LOW));
        assert_eq!(request.runtime.parallel_tool_calls, Some(false));
        assert_eq!(request.runtime.auto_compact_token_limit, Some(123_000));
        assert!(
            request
                .instructions
                .developer
                .as_deref()
                .unwrap_or_default()
                .contains("Use the provided context as the current working set")
        );
        assert_eq!(
            request
                .metadata
                .pointer("/modelProfile/schemaPolicy")
                .and_then(serde_json::Value::as_str),
            Some("standard_required_first")
        );
    }

    #[tokio::test]
    async fn ultra_reasoning_reaches_transport_state_and_enables_proactive_delegation() {
        let request = captured_profile_request(RuntimeConfig {
            default_provider: roder_api::catalog::PROVIDER_MOCK.to_string(),
            default_model: "gpt-5.6-sol".to_string(),
            reasoning: Some(REASONING_ULTRA.to_string()),
            ..RuntimeConfig::default()
        })
        .await;

        assert_eq!(request.reasoning.level.as_deref(), Some(REASONING_ULTRA));
        let developer = request
            .instructions
            .developer
            .as_deref()
            .expect("ultra developer instructions");
        assert!(developer.contains("Proactive multi-agent delegation is active"));
    }

    #[tokio::test]
    async fn lower_sol_effort_requires_explicit_multi_agent_request() {
        let request = captured_profile_request(RuntimeConfig {
            default_provider: roder_api::catalog::PROVIDER_MOCK.to_string(),
            default_model: "gpt-5.6-sol".to_string(),
            reasoning: Some(roder_api::catalog::REASONING_MEDIUM.to_string()),
            ..RuntimeConfig::default()
        })
        .await;

        let developer = request
            .instructions
            .developer
            .as_deref()
            .expect("sol developer instructions");
        assert!(developer.contains("Do not spawn sub-agents unless"));
        assert!(!developer.contains("Proactive multi-agent delegation is active"));
    }

    #[tokio::test]
    async fn luna_does_not_receive_codex_v2_multi_agent_policy() {
        let request = captured_profile_request(RuntimeConfig {
            default_provider: roder_api::catalog::PROVIDER_MOCK.to_string(),
            default_model: "gpt-5.6-luna".to_string(),
            reasoning: Some(roder_api::catalog::REASONING_MAX.to_string()),
            ..RuntimeConfig::default()
        })
        .await;

        let developer = request
            .instructions
            .developer
            .as_deref()
            .unwrap_or_default();
        assert!(!developer.contains("Do not spawn sub-agents unless"));
        assert!(!developer.contains("Proactive multi-agent delegation is active"));
    }

    #[tokio::test]
    async fn turn_developer_context_reaches_inference_and_does_not_persist() {
        let captured = Arc::new(StdMutex::new(None));
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(CapturingEngine {
            request: captured.clone(),
        }));
        let runtime = Arc::new(
            Runtime::new(
                builder.build().unwrap(),
                RuntimeConfig {
                    default_provider: roder_api::catalog::PROVIDER_MOCK.to_string(),
                    default_model: "gpt-5.5".to_string(),
                    ..RuntimeConfig::default()
                },
            )
            .unwrap(),
        );

        async fn run_turn(runtime: &Arc<Runtime>, developer_context: Option<String>) {
            let mut rx = runtime.subscribe_events();
            let turn_id = runtime
                .start_turn(StartTurnRequest {
                    thread_id: "thread-turn-context".to_string(),
                    message: "hello".to_string(),
                    images: Vec::new(),
                    provider_override: None,
                    model_override: None,
                    reasoning_override: None,
                    workspace: test_workspace(),
                    instructions: InstructionBundle::default(),
                    developer_context,
                    task_ledger_required: false,
                })
                .await
                .unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    let envelope = rx.recv().await.unwrap();
                    if envelope.turn_id.as_deref() != Some(&turn_id) {
                        continue;
                    }
                    match envelope.event {
                        RoderEvent::TurnCompleted(_) => break,
                        RoderEvent::TurnFailed(event) => panic!("turn failed: {}", event.error),
                        _ => {}
                    }
                }
            })
            .await
            .unwrap();
        }

        run_turn(
            &runtime,
            Some("Connected accounts: example-service.".to_string()),
        )
        .await;
        let request = captured.lock().unwrap().clone().unwrap();
        assert_eq!(
            request.instructions.developer_context.as_deref(),
            Some("Connected accounts: example-service.")
        );

        // The context is per-turn only: the next turn on the same thread
        // without a developerContext must not see the previous one.
        run_turn(&runtime, None).await;
        let request = captured.lock().unwrap().clone().unwrap();
        assert_eq!(request.instructions.developer_context, None);
    }

    #[tokio::test]
    async fn tool_search_overrides_route_to_next_inference_request() {
        let request = captured_profile_request(RuntimeConfig {
            default_provider: roder_api::catalog::PROVIDER_MOCK.to_string(),
            default_model: "gpt-5.4".to_string(),
            tool_search: ToolSearchConfig {
                mode: roder_api::inference::ToolSearchMode::Auto,
                max_catalog_items: Some(100),
                ..ToolSearchConfig::default()
            },
            provider_tool_search: std::collections::HashMap::from([(
                roder_api::catalog::PROVIDER_MOCK.to_string(),
                roder_api::inference::ToolSearchConfigOverlay {
                    include_skills: Some(false),
                    provider_variant: Some(roder_api::inference::ToolSearchProviderVariant::Regex),
                    ..Default::default()
                },
            )]),
            model_tool_search: std::collections::HashMap::from([(
                "gpt-5.4".to_string(),
                roder_api::inference::ToolSearchConfigOverlay {
                    mode: Some(roder_api::inference::ToolSearchMode::ProviderNative),
                    max_catalog_items: Some(25),
                    provider_variant: Some(roder_api::inference::ToolSearchProviderVariant::Bm25),
                    ..Default::default()
                },
            )]),
            ..RuntimeConfig::default()
        })
        .await;

        assert_eq!(
            request.runtime.tool_search.mode,
            roder_api::inference::ToolSearchMode::ProviderNative
        );
        assert_eq!(request.runtime.tool_search.max_catalog_items, Some(25));
        assert_eq!(
            request.runtime.tool_search.provider_variant,
            roder_api::inference::ToolSearchProviderVariant::Bm25
        );
    }

    #[tokio::test]
    async fn context_entrypoint_hints_use_turn_workspace() {
        let process_workspace = runtime_test_workspace("entrypoint-process");
        let thread_workspace = runtime_test_workspace("entrypoint-thread");
        std::fs::create_dir_all(process_workspace.join("src")).unwrap();
        std::fs::create_dir_all(thread_workspace.join("src")).unwrap();
        std::fs::write(
            process_workspace.join("src/sidebar-thread-groups.ts"),
            "export const desktopLeak = true;\n",
        )
        .unwrap();
        std::fs::write(
            thread_workspace.join("src/voice-plan-feedback.ts"),
            "export const voicePlanFeedback = true;\n",
        )
        .unwrap();

        let captured = Arc::new(StdMutex::new(None));
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(CapturingEngine {
            request: captured.clone(),
        }));
        builder.context_planner(Arc::new(roder_context::EntrypointContextPlanner::new(
            process_workspace.clone(),
        )));
        let runtime = Arc::new(
            Runtime::new(
                builder.build().unwrap(),
                RuntimeConfig {
                    workspace: Some(process_workspace.display().to_string()),
                    ..RuntimeConfig::default()
                },
            )
            .unwrap(),
        );
        let mut rx = runtime.subscribe_events();

        let turn_id = runtime
            .start_turn(StartTurnRequest {
                thread_id: "thread-workspace-entrypoint".to_string(),
                message: "investigate voice plan feedback".to_string(),
                images: Vec::new(),
                provider_override: None,
                model_override: None,
                reasoning_override: None,
                workspace: thread_workspace.display().to_string(),
                instructions: crate::instructions::default_instructions(),
                developer_context: None,
                task_ledger_required: false,
            })
            .await
            .unwrap();

        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let envelope = rx.recv().await.unwrap();
                if envelope.turn_id.as_deref() != Some(&turn_id) {
                    continue;
                }
                match envelope.event {
                    RoderEvent::TurnCompleted(_) => break,
                    RoderEvent::TurnFailed(event) => panic!("turn failed: {}", event.error),
                    _ => {}
                }
            }
        })
        .await
        .unwrap();

        let request = captured.lock().unwrap().clone().expect("captured request");
        let transcript_text = request
            .transcript
            .iter()
            .map(|item| match item {
                TranscriptItem::UserMessage(message) => message.text.as_str(),
                _ => "",
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(transcript_text.contains("src/voice-plan-feedback.ts"));
        assert!(!transcript_text.contains("src/sidebar-thread-groups.ts"));

        let _ = std::fs::remove_dir_all(process_workspace);
        let _ = std::fs::remove_dir_all(thread_workspace);
    }

    fn runtime_test_workspace(name: &str) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("roder-runtime-{name}-{}", uuid::Uuid::new_v4()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[tokio::test]
    async fn model_profile_user_model_knobs_override_profile_defaults() {
        let request = captured_profile_request(RuntimeConfig {
            default_provider: roder_api::catalog::PROVIDER_MOCK.to_string(),
            default_model: "gpt-5.5".to_string(),
            reasoning: Some(REASONING_HIGH.to_string()),
            auto_compact_token_limit: Some(999),
            model_edit_tools: std::collections::HashMap::from([(
                "gpt-5.5".to_string(),
                EDIT_TOOL_PATCH.to_string(),
            )]),
            model_parallel_tool_calls: std::collections::HashMap::from([(
                "gpt-5.5".to_string(),
                true,
            )]),
            model_profiles: std::collections::HashMap::from([(
                "gpt-5.5".to_string(),
                test_model_profile("gpt-5.5"),
            )]),
            ..RuntimeConfig::default()
        })
        .await;

        let tool_names = request
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert!(tool_names.contains(&"apply_patch"));
        assert!(!tool_names.contains(&"edit"));
        assert_eq!(request.reasoning.level.as_deref(), Some(REASONING_HIGH));
        assert_eq!(request.runtime.parallel_tool_calls, Some(true));
        assert_eq!(request.runtime.auto_compact_token_limit, Some(999));
    }

    #[tokio::test]
    async fn runtime_tool_allowlist_filters_advertised_tools() {
        let request = captured_profile_request(RuntimeConfig {
            default_provider: roder_api::catalog::PROVIDER_MOCK.to_string(),
            default_model: "gpt-5.5".to_string(),
            tool_allowlist: vec!["edit".to_string()],
            model_profiles: std::collections::HashMap::from([(
                "gpt-5.5".to_string(),
                test_model_profile("gpt-5.5"),
            )]),
            ..RuntimeConfig::default()
        })
        .await;

        let tool_names = request
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(tool_names, vec!["edit"]);
    }

    /// Builds a store-backed runtime turn for a thread created with the given per-thread
    /// overrides and returns the captured inference request.
    async fn captured_thread_override_request(
        runtime: &Arc<Runtime>,
        requests: &Arc<StdMutex<Vec<AgentInferenceRequest>>>,
        tool_allowlist: Vec<String>,
        developer_instructions: Option<String>,
        external_tools: Vec<ToolSpec>,
    ) -> AgentInferenceRequest {
        let thread_id = runtime
            .create_thread_with(CreateThreadRequest {
                title: Some("Thread overrides".to_string()),
                workspace: test_workspace(),
                workspace_id: None,
                root_id: None,
                provider: None,
                model: None,
                selection_mode: None,
                tool_allowlist,
                developer_instructions,
                external_tools,
                runner: None,
            })
            .await
            .unwrap()
            .thread_id;
        let mut rx = runtime.subscribe_events();
        let turn_id = runtime
            .start_turn(StartTurnRequest {
                thread_id,
                message: "hello".to_string(),
                images: Vec::new(),
                provider_override: None,
                model_override: None,
                reasoning_override: None,
                workspace: test_workspace(),
                instructions: crate::default_instructions(),
                developer_context: None,
                task_ledger_required: false,
            })
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let envelope = rx.recv().await.unwrap();
                if envelope.turn_id.as_deref() != Some(&turn_id) {
                    continue;
                }
                match envelope.event {
                    RoderEvent::TurnCompleted(_) => break,
                    RoderEvent::TurnFailed(event) => panic!("turn failed: {}", event.error),
                    _ => {}
                }
            }
        })
        .await
        .unwrap();
        requests.lock().unwrap().pop().expect("captured request")
    }

    #[tokio::test]
    async fn thread_tool_allowlist_filters_only_that_thread() {
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let thread_root =
            std::env::temp_dir().join(format!("roder-thread-allowlist-{}", uuid::Uuid::new_v4()));
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(SwitchCaptureEngine {
            requests: requests.clone(),
        }));
        builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
            base_path: thread_root.clone(),
        }));
        builder.tool_contributor(Arc::new(ProfileToolContributor));
        let runtime = Arc::new(
            Runtime::new(
                builder.build().unwrap(),
                RuntimeConfig {
                    default_provider: roder_api::catalog::PROVIDER_MOCK.to_string(),
                    default_model: "gpt-5.5".to_string(),
                    model_profiles: std::collections::HashMap::from([(
                        "gpt-5.5".to_string(),
                        test_model_profile("gpt-5.5"),
                    )]),
                    ..RuntimeConfig::default()
                },
            )
            .unwrap(),
        );

        let allowlisted = captured_thread_override_request(
            &runtime,
            &requests,
            vec!["edit".to_string()],
            None,
            Vec::new(),
        )
        .await;
        let unrestricted =
            captured_thread_override_request(&runtime, &requests, Vec::new(), None, Vec::new())
                .await;

        let allowlisted_names = allowlisted
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(allowlisted_names, vec!["edit"]);
        let unrestricted_names = unrestricted
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert!(unrestricted_names.contains(&"edit"));
        assert!(unrestricted_names.len() > 1);

        let _ = std::fs::remove_dir_all(thread_root);
    }

    /// Builds a store-backed runtime whose `RuntimeConfig.tool_allowlist` is `["edit"]`.
    fn runtime_with_edit_allowlist(
        requests: &Arc<StdMutex<Vec<AgentInferenceRequest>>>,
        thread_root: &std::path::Path,
    ) -> Arc<Runtime> {
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(SwitchCaptureEngine {
            requests: requests.clone(),
        }));
        builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
            base_path: thread_root.to_path_buf(),
        }));
        builder.tool_contributor(Arc::new(ProfileToolContributor));
        Arc::new(
            Runtime::new(
                builder.build().unwrap(),
                RuntimeConfig {
                    default_provider: roder_api::catalog::PROVIDER_MOCK.to_string(),
                    default_model: "gpt-5.5".to_string(),
                    tool_allowlist: vec!["edit".to_string()],
                    model_profiles: std::collections::HashMap::from([(
                        "gpt-5.5".to_string(),
                        test_model_profile("gpt-5.5"),
                    )]),
                    ..RuntimeConfig::default()
                },
            )
            .unwrap(),
        )
    }

    #[tokio::test]
    async fn runtime_and_thread_allowlists_intersect() {
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let thread_root = std::env::temp_dir().join(format!(
            "roder-allowlist-intersect-{}",
            uuid::Uuid::new_v4()
        ));
        let runtime = runtime_with_edit_allowlist(&requests, &thread_root);

        let request = captured_thread_override_request(
            &runtime,
            &requests,
            vec!["edit".to_string(), "write_file".to_string()],
            None,
            Vec::new(),
        )
        .await;

        // The thread allowlist must not re-enable tools the runtime allowlist bans.
        let names = request
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["edit"]);

        let _ = std::fs::remove_dir_all(thread_root);
    }

    #[tokio::test]
    async fn route_tool_call_denies_tools_outside_allowlists() {
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let thread_root =
            std::env::temp_dir().join(format!("roder-allowlist-dispatch-{}", uuid::Uuid::new_v4()));
        let runtime = runtime_with_edit_allowlist(&requests, &thread_root);
        let thread_id = runtime
            .create_thread_with(CreateThreadRequest {
                title: Some("Dispatch allowlist".to_string()),
                workspace: test_workspace(),
                workspace_id: None,
                root_id: None,
                provider: None,
                model: None,
                selection_mode: None,
                tool_allowlist: vec!["edit".to_string(), "write_file".to_string()],
                developer_instructions: None,
                external_tools: Vec::new(),
                runner: None,
            })
            .await
            .unwrap()
            .thread_id;

        // write_file is registered and on the thread allowlist but banned by the runtime allowlist.
        let result = runtime
            .route_tool_call(
                &thread_id,
                &"turn-allowlist-dispatch".to_string(),
                roder_api::inference::ToolCallCompleted {
                    id: "call-1".to_string(),
                    name: "write_file".to_string(),
                    arguments: r#"{"path":"a.txt","content":"hi"}"#.to_string(),
                },
                None,
                None,
            )
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(
            result
                .result
                .contains("not permitted by the tool allowlist"),
            "unexpected result: {}",
            result.result
        );

        let _ = std::fs::remove_dir_all(thread_root);
    }

    #[tokio::test]
    async fn route_tool_calls_denies_agent_swarm_mixed_with_other_tools() {
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let thread_root =
            std::env::temp_dir().join(format!("roder-swarm-exclusive-{}", uuid::Uuid::new_v4()));
        let runtime = runtime_with_edit_allowlist(&requests, &thread_root);

        // A response that mixes agent_swarm with another tool is denied wholesale:
        // both calls get an error result with retry guidance, and no tool runs.
        let results = runtime
            .route_tool_calls(
                &"thread-swarm".to_string(),
                &"turn-swarm".to_string(),
                vec![
                    roder_api::inference::ToolCallCompleted {
                        id: "swarm-1".to_string(),
                        name: "agent_swarm".to_string(),
                        arguments: "{}".to_string(),
                    },
                    roder_api::inference::ToolCallCompleted {
                        id: "read-1".to_string(),
                        name: "read_file".to_string(),
                        arguments: "{}".to_string(),
                    },
                ],
                true,
                None,
                None,
            )
            .await
            .unwrap();

        assert_eq!(results.len(), 2, "every tool_call_id must get a response");
        assert_eq!(results[0].id, "swarm-1");
        assert_eq!(results[1].id, "read-1");
        for result in &results {
            assert!(result.is_error);
            assert!(
                result.result.contains("only tool call"),
                "unexpected result: {}",
                result.result
            );
        }

        let _ = std::fs::remove_dir_all(thread_root);
    }

    #[tokio::test]
    async fn route_tool_calls_emits_agent_swarm_started_event() {
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let thread_root =
            std::env::temp_dir().join(format!("roder-swarm-started-{}", uuid::Uuid::new_v4()));
        let runtime = runtime_with_edit_allowlist(&requests, &thread_root);
        let mut events = runtime.subscribe_events();

        // A single agent_swarm call (the valid shape) emits AgentSwarmStarted on
        // the event bus before dispatch, with the child count parsed from args.
        let _ = runtime
            .route_tool_calls(
                &"thread-swarm".to_string(),
                &"turn-swarm".to_string(),
                vec![roder_api::inference::ToolCallCompleted {
                    id: "swarm-1".to_string(),
                    name: "agent_swarm".to_string(),
                    arguments: r#"{"description":"x","prompt_template":"Read {{item}}","items":["a.rs","b.rs"]}"#
                        .to_string(),
                }],
                true,
                None,
                None,
            )
            .await
            .unwrap();

        let mut started_child_count = None;
        for _ in 0..16 {
            let envelope = tokio::time::timeout(std::time::Duration::from_secs(2), events.recv())
                .await
                .unwrap()
                .unwrap();
            if let RoderEvent::AgentSwarmStarted(event) = envelope.event {
                started_child_count = Some(event.child_count);
                assert_eq!(event.tool_id, "swarm-1");
                break;
            }
        }
        assert_eq!(started_child_count, Some(2));

        let _ = std::fs::remove_dir_all(thread_root);
    }

    #[test]
    fn agent_swarm_child_count_sums_items_and_resumes() {
        assert_eq!(
            agent_swarm_child_count(r#"{"description":"x","items":["a","b","c"]}"#),
            3
        );
        assert_eq!(
            agent_swarm_child_count(
                r#"{"description":"x","items":["a"],"resume_agent_ids":{"id1":"continue"}}"#
            ),
            2
        );
        // Malformed input is lenient.
        assert_eq!(agent_swarm_child_count("not json"), 0);
    }

    #[test]
    fn parse_swarm_counts_reads_summary_with_omitted_buckets() {
        let text = "<agent_swarm_result>\n<summary>completed: 2, failed: 1</summary>\n</agent_swarm_result>";
        assert_eq!(parse_swarm_counts(text), Some((2, 1, 0)));
        let text = "<agent_swarm_result>\n<summary>completed: 0</summary>\n</agent_swarm_result>";
        assert_eq!(parse_swarm_counts(text), Some((0, 0, 0)));
        // Not a swarm result.
        assert_eq!(parse_swarm_counts("just text"), None);
    }

    /// Signals when inference starts, then waits for `proceed` and fails the stream.
    struct SignalledFailureEngine {
        started: tokio::sync::mpsc::UnboundedSender<()>,
        proceed: Arc<tokio::sync::Notify>,
    }

    #[async_trait::async_trait]
    impl InferenceEngine for SignalledFailureEngine {
        fn id(&self) -> String {
            roder_api::catalog::PROVIDER_MOCK.to_string()
        }

        fn capabilities(&self) -> InferenceCapabilities {
            InferenceCapabilities::coding_agent_default()
        }

        async fn list_models(
            &self,
            _ctx: InferenceProviderContext<'_>,
        ) -> anyhow::Result<Vec<roder_api::inference::ModelDescriptor>> {
            Ok(roder_api::catalog::models_for_provider(
                roder_api::catalog::PROVIDER_MOCK,
                true,
            ))
        }

        async fn stream_turn(
            &self,
            _ctx: InferenceTurnContext<'_>,
            _request: AgentInferenceRequest,
        ) -> anyhow::Result<InferenceEventStream> {
            let _ = self.started.send(());
            self.proceed.notified().await;
            anyhow::bail!("engine failed mid-turn")
        }
    }

    #[tokio::test]
    async fn failed_turn_sweeps_pending_external_tool_calls() {
        let (started_tx, mut started_rx) = tokio::sync::mpsc::unbounded_channel();
        let proceed = Arc::new(tokio::sync::Notify::new());
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(SignalledFailureEngine {
            started: started_tx,
            proceed: proceed.clone(),
        }));
        let runtime =
            Arc::new(Runtime::new(builder.build().unwrap(), RuntimeConfig::default()).unwrap());
        let mut rx = runtime.subscribe_events();
        let turn_id = runtime
            .start_turn(StartTurnRequest {
                thread_id: "thread-sweep".to_string(),
                message: "go".to_string(),
                images: Vec::new(),
                provider_override: None,
                model_override: None,
                reasoning_override: None,
                workspace: test_workspace(),
                instructions: InstructionBundle {
                    system: None,
                    developer: None,
                    developer_context: None,
                },
                developer_context: None,
                task_ledger_required: false,
            })
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), started_rx.recv())
            .await
            .unwrap()
            .unwrap();

        let (tx, _pending_rx) = oneshot::channel();
        runtime.pending_external_tool_calls.lock().await.insert(
            "exttool-sweep-test".to_string(),
            PendingExternalToolCall {
                thread_id: "thread-sweep".to_string(),
                turn_id: turn_id.clone(),
                tool_id: "call-1".to_string(),
                tool_name: "acme_lookup".to_string(),
                tx,
            },
        );
        proceed.notify_one();

        let outcome = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let envelope = rx.recv().await.unwrap();
                if let RoderEvent::ExternalToolCallResolved(event) = envelope.event
                    && event.request_id == "exttool-sweep-test"
                {
                    break event.outcome;
                }
            }
        })
        .await
        .expect("turn failure must resolve pending external tool calls");
        assert_eq!(outcome, ExternalToolCallOutcome::Cancelled);
        assert!(runtime.pending_external_tool_calls.lock().await.is_empty());
    }

    #[tokio::test]
    async fn thread_developer_instructions_layer_under_harness_prompt() {
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let thread_root = std::env::temp_dir().join(format!(
            "roder-thread-instructions-{}",
            uuid::Uuid::new_v4()
        ));
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(SwitchCaptureEngine {
            requests: requests.clone(),
        }));
        builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
            base_path: thread_root.clone(),
        }));
        builder.tool_contributor(Arc::new(ProfileToolContributor));
        let runtime =
            Arc::new(Runtime::new(builder.build().unwrap(), RuntimeConfig::default()).unwrap());

        let request = captured_thread_override_request(
            &runtime,
            &requests,
            Vec::new(),
            Some("You are embedded in a host app.".to_string()),
            Vec::new(),
        )
        .await;

        let system = request.instructions.system.expect("system instructions");
        assert!(system.starts_with("You are Roder"));
        let developer = request
            .instructions
            .developer
            .expect("developer instructions");
        assert!(developer.starts_with("You are embedded in a host app."));

        let plain =
            captured_thread_override_request(&runtime, &requests, Vec::new(), None, Vec::new())
                .await;
        assert_eq!(plain.instructions.developer, None);

        let _ = std::fs::remove_dir_all(thread_root);
    }

    #[tokio::test]
    async fn thread_external_tools_are_advertised_and_shadow_builtins() {
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let thread_root = std::env::temp_dir().join(format!(
            "roder-thread-external-tools-{}",
            uuid::Uuid::new_v4()
        ));
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(SwitchCaptureEngine {
            requests: requests.clone(),
        }));
        builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
            base_path: thread_root.clone(),
        }));
        builder.tool_contributor(Arc::new(ProfileToolContributor));
        let runtime =
            Arc::new(Runtime::new(builder.build().unwrap(), RuntimeConfig::default()).unwrap());

        let external_tools = vec![
            ToolSpec {
                name: "acme_lookup".to_string(),
                description: "Look up Acme workspace state.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": { "query": { "type": "string" } },
                    "required": ["query"]
                }),
            },
            ToolSpec {
                name: "edit".to_string(),
                description: "Host-managed edit.".to_string(),
                parameters: serde_json::json!({ "type": "object" }),
            },
        ];
        let request =
            captured_thread_override_request(&runtime, &requests, Vec::new(), None, external_tools)
                .await;

        let acme = request
            .tools
            .iter()
            .find(|tool| tool.name == "acme_lookup")
            .expect("external tool advertised");
        assert_eq!(acme.description, "Look up Acme workspace state.");
        assert_eq!(acme.parameters["required"][0], "query");
        let edits = request
            .tools
            .iter()
            .filter(|tool| tool.name == "edit")
            .collect::<Vec<_>>();
        assert_eq!(edits.len(), 1, "external edit shadows the builtin");
        assert_eq!(edits[0].description, "Host-managed edit.");

        let plain =
            captured_thread_override_request(&runtime, &requests, Vec::new(), None, Vec::new())
                .await;
        assert!(plain.tools.iter().all(|tool| tool.name != "acme_lookup"));
        let plain_edit = plain
            .tools
            .iter()
            .find(|tool| tool.name == "edit")
            .expect("builtin edit advertised on plain thread");
        assert_eq!(plain_edit.description, "edit test tool");

        let _ = std::fs::remove_dir_all(thread_root);
    }

    #[tokio::test]
    async fn model_switch_injects_summary_and_records_profile_segments() {
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let thread_root = std::env::temp_dir().join(format!(
            "roder-model-switch-thread-{}",
            uuid::Uuid::new_v4()
        ));
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(SwitchCaptureEngine {
            requests: requests.clone(),
        }));
        builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
            base_path: thread_root.clone(),
        }));
        builder.tool_contributor(Arc::new(ProfileToolContributor));
        let mut claude_profile = test_model_profile("claude-haiku-4-5-20251001");
        claude_profile.provider_family = ProviderFamily::Anthropic;
        claude_profile.edit_tool = Some(EDIT_TOOL_EDIT.to_string());
        let runtime = Arc::new(
            Runtime::new(
                builder.build().unwrap(),
                RuntimeConfig {
                    default_provider: roder_api::catalog::PROVIDER_MOCK.to_string(),
                    default_model: "gpt-5.5".to_string(),
                    model_profiles: std::collections::HashMap::from([
                        ("gpt-5.5".to_string(), test_model_profile("gpt-5.5")),
                        ("claude-haiku-4-5-20251001".to_string(), claude_profile),
                    ]),
                    ..RuntimeConfig::default()
                },
            )
            .unwrap(),
        );
        let thread_id = runtime
            .create_thread_with(CreateThreadRequest {
                title: Some("Model switch".to_string()),
                workspace: test_workspace(),
                workspace_id: None,
                root_id: None,
                provider: None,
                model: None,
                selection_mode: None,
                tool_allowlist: Vec::new(),
                developer_instructions: None,
                external_tools: Vec::new(),
                runner: None,
            })
            .await
            .unwrap()
            .thread_id;
        let mut rx = runtime.subscribe_events();
        for (message, model_override) in [
            ("first turn", None),
            ("second turn", Some("claude-haiku-4-5-20251001".to_string())),
        ] {
            let turn_id = runtime
                .start_turn(StartTurnRequest {
                    thread_id: thread_id.clone(),
                    message: message.to_string(),
                    images: Vec::new(),
                    provider_override: None,
                    model_override,
                    reasoning_override: None,
                    workspace: test_workspace(),
                    instructions: InstructionBundle::default(),
                    developer_context: None,
                    task_ledger_required: false,
                })
                .await
                .unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    let envelope = rx.recv().await.unwrap();
                    if envelope.turn_id.as_deref() != Some(&turn_id) {
                        continue;
                    }
                    match envelope.event {
                        RoderEvent::TurnCompleted(_) => break,
                        RoderEvent::TurnFailed(event) => panic!("turn failed: {}", event.error),
                        _ => {}
                    }
                }
            })
            .await
            .unwrap();
        }

        let captured = requests.lock().unwrap().clone();
        assert_eq!(captured.len(), 2);
        assert!(captured[1].transcript.iter().any(|item| {
            matches!(
                item,
                TranscriptItem::UserMessage(message)
                    if message.text.starts_with(MODEL_SWITCH_SUMMARY_PREFIX)
                        && message.text.contains("previous profile mock/gpt-5.5")
                        && message.text.contains("Current profile mock/claude-haiku-4-5-20251001")
                        && message.text.contains("Available tools now:")
            )
        }));

        let snapshot = runtime
            .thread_store
            .as_ref()
            .unwrap()
            .load_thread(&thread_id)
            .await
            .unwrap()
            .unwrap();
        let trace_segments = snapshot
            .turns
            .iter()
            .flat_map(|turn| &turn.items)
            .filter(|item| {
                matches!(
                    item,
                    TranscriptItem::ProviderMetadata(value)
                        if value.get("kind").and_then(serde_json::Value::as_str)
                            == Some(MODEL_PROFILE_TRACE_KIND)
                            && value.get("segment").and_then(serde_json::Value::as_str)
                                == Some("assistant")
                )
            })
            .count();
        assert!(trace_segments >= 2);
        let _ = std::fs::remove_dir_all(thread_root);
    }

    struct CountingTaskTool {
        calls: Arc<StdMutex<u32>>,
    }

    #[async_trait::async_trait]
    impl ToolExecutor for CountingTaskTool {
        fn spec(&self) -> ToolSpec {
            ToolSpec {
                name: "task".to_string(),
                description: "Dispatch a test subagent.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "description": { "type": "string" },
                        "prompt": { "type": "string" },
                        "parent_deadline_seconds": { "type": "integer" }
                    },
                    "required": ["description", "prompt"],
                    "additionalProperties": false
                }),
            }
        }

        async fn execute(
            &self,
            _ctx: ToolExecutionContext,
            call: ToolCall,
        ) -> anyhow::Result<ToolResult> {
            *self.calls.lock().unwrap() += 1;
            Ok(ToolResult {
                id: call.id,
                name: call.name,
                text: "started child".to_string(),
                data: serde_json::json!({}),
                is_error: false,
            })
        }
    }

    #[tokio::test]
    async fn runtime_profile_reaches_inference_request_and_turn_metadata() {
        let captured = Arc::new(StdMutex::new(None));
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(CapturingEngine {
            request: captured.clone(),
        }));
        let runtime = Arc::new(
            Runtime::new(
                builder.build().unwrap(),
                RuntimeConfig {
                    runtime_profile: RuntimeProfile::NonInteractive,
                    ..RuntimeConfig::default()
                },
            )
            .unwrap(),
        );
        let mut rx = runtime.subscribe_events();
        let turn_id = runtime
            .start_turn(StartTurnRequest {
                thread_id: "thread-profile".to_string(),
                message: "work unattended".to_string(),
                images: Vec::new(),
                provider_override: None,
                model_override: None,
                reasoning_override: None,
                workspace: test_workspace(),
                instructions: InstructionBundle {
                    system: None,
                    developer: Some("base developer".to_string()),
                    developer_context: None,
                },
                developer_context: None,
                task_ledger_required: false,
            })
            .await
            .unwrap();

        let mut observed_profile = None;
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let envelope = rx.recv().await.unwrap();
                if envelope.turn_id.as_deref() != Some(&turn_id) {
                    continue;
                }
                match envelope.event {
                    RoderEvent::TurnStarted(event) => {
                        observed_profile = Some(event.runtime_profile);
                    }
                    RoderEvent::TurnCompleted(_) => break,
                    RoderEvent::TurnFailed(event) => panic!("turn failed: {}", event.error),
                    _ => {}
                }
            }
        })
        .await
        .unwrap();

        assert_eq!(observed_profile, Some(RuntimeProfile::NonInteractive));
        let request = captured.lock().unwrap().clone().unwrap();
        assert_eq!(request.runtime.profile, RuntimeProfile::NonInteractive);
        let developer = request.instructions.developer.unwrap();
        assert!(developer.contains("base developer"));
        assert!(developer.contains("non-interactive profile"));
    }

    #[tokio::test]
    async fn global_policy_mode_changes_do_not_create_runtime_thread_directory() {
        let workspace = runtime_test_workspace("global-policy-mode");
        let thread_root = workspace.join("threads");
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(FakeInferenceEngine));
        builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
            base_path: thread_root.clone(),
        }));
        let runtime = Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                workspace: Some(workspace.display().to_string()),
                ..Default::default()
            },
        )
        .unwrap();

        runtime
            .set_policy_mode(PolicyMode::AcceptAll, Some("test".to_string()))
            .await
            .unwrap();

        assert!(!thread_root.join("runtime").exists());
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[tokio::test]
    async fn task_ledger_enforcement_injects_eval_reminder_before_work() {
        let captured = Arc::new(StdMutex::new(None));
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(CapturingEngine {
            request: captured.clone(),
        }));
        builder.tool_contributor(Arc::new(
            roder_ext_task_ledger::TaskLedgerToolContributor::default(),
        ));
        let runtime = Arc::new(
            Runtime::new(
                builder.build().unwrap(),
                RuntimeConfig {
                    runtime_profile: RuntimeProfile::Eval,
                    policy_mode: PolicyMode::Bypass,
                    agent_swarm_mode: false,
                    ..RuntimeConfig::default()
                },
            )
            .unwrap(),
        );
        let mut rx = runtime.subscribe_events();
        let turn_id = runtime
            .start_turn(StartTurnRequest {
                thread_id: "thread-ledger".to_string(),
                message: "decomposed work".to_string(),
                images: Vec::new(),
                provider_override: None,
                model_override: None,
                reasoning_override: None,
                workspace: test_workspace(),
                instructions: InstructionBundle::default(),
                developer_context: None,
                task_ledger_required: true,
            })
            .await
            .unwrap();

        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let envelope = rx.recv().await.unwrap();
                if envelope.turn_id.as_deref() == Some(&turn_id)
                    && matches!(envelope.event, RoderEvent::TurnCompleted(_))
                {
                    break;
                }
            }
        })
        .await
        .unwrap();

        let request = captured.lock().unwrap().clone().unwrap();
        let developer = request.instructions.developer.unwrap();
        assert!(developer.contains("Task Ledger Required"));
        assert!(developer.contains("task_ledger.update"));
        let tool_names: Vec<_> = request
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect();
        assert!(
            tool_names.contains(&TASK_LEDGER_TOOL_NAME),
            "tool names: {tool_names:?}"
        );
        assert_eq!(
            request.tool_choice,
            ToolChoice::Specific(TASK_LEDGER_TOOL_NAME.to_string())
        );
        assert_eq!(request.tools.len(), 1);
        assert_eq!(request.tools[0].name, TASK_LEDGER_TOOL_NAME);
    }

    #[tokio::test]
    async fn eval_task_ledger_blocks_final_answer_until_open_items_are_completed() {
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(TaskLedgerCompletionGateEngine {
            calls: StdMutex::new(0),
            requests: requests.clone(),
        }));
        builder.tool_contributor(Arc::new(
            roder_ext_task_ledger::TaskLedgerToolContributor::default(),
        ));
        let runtime = Arc::new(
            Runtime::new(
                builder.build().unwrap(),
                RuntimeConfig {
                    runtime_profile: RuntimeProfile::Eval,
                    policy_mode: PolicyMode::Bypass,
                    agent_swarm_mode: false,
                    ..RuntimeConfig::default()
                },
            )
            .unwrap(),
        );
        let mut rx = runtime.subscribe_events();
        let turn_id = runtime
            .start_turn(StartTurnRequest {
                thread_id: "thread-ledger-completion".to_string(),
                message: "write the answer file".to_string(),
                images: Vec::new(),
                provider_override: None,
                model_override: None,
                reasoning_override: None,
                workspace: test_workspace(),
                instructions: InstructionBundle::default(),
                developer_context: None,
                task_ledger_required: true,
            })
            .await
            .unwrap();

        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let envelope = rx.recv().await.unwrap();
                if envelope.turn_id.as_deref() != Some(&turn_id) {
                    continue;
                }
                match envelope.event {
                    RoderEvent::TurnCompleted(_) => break,
                    RoderEvent::TurnFailed(event) => panic!("turn failed: {}", event.error),
                    _ => {}
                }
            }
        })
        .await
        .unwrap();

        let requests = requests.lock().unwrap().clone();
        assert_eq!(requests.len(), 4);
        assert!(requests[2].transcript.iter().any(|item| {
            matches!(
                item,
                TranscriptItem::UserMessage(message)
                    if message.text.contains("Task Ledger Completion Required")
                        && message.text.contains("Write /app/result.txt")
            )
        }));
        assert!(requests[3].transcript.iter().any(|item| {
            matches!(
                item,
                TranscriptItem::ToolResult(result)
                    if result.name.as_deref() == Some(TASK_LEDGER_TOOL_NAME)
                        && result.result.contains("Task ledger: 2/2 completed")
            )
        }));
    }

    #[tokio::test]
    async fn eval_task_ledger_checkpoint_requests_scoreable_file_before_final_reserve() {
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(TaskLedgerCompletionGateEngine {
            calls: StdMutex::new(0),
            requests: requests.clone(),
        }));
        builder.tool_contributor(Arc::new(
            roder_ext_task_ledger::TaskLedgerToolContributor::default(),
        ));
        let runtime = Arc::new(
            Runtime::new(
                builder.build().unwrap(),
                RuntimeConfig {
                    runtime_profile: RuntimeProfile::Eval,
                    policy_mode: PolicyMode::Bypass,
                    agent_swarm_mode: false,
                    turn_deadline_seconds: Some(120),
                    ..RuntimeConfig::default()
                },
            )
            .unwrap(),
        );
        let mut rx = runtime.subscribe_events();
        let turn_id = runtime
            .start_turn(StartTurnRequest {
                thread_id: "thread-ledger-checkpoint".to_string(),
                message: "write the answer file".to_string(),
                images: Vec::new(),
                provider_override: None,
                model_override: None,
                reasoning_override: None,
                workspace: test_workspace(),
                instructions: InstructionBundle::default(),
                developer_context: None,
                task_ledger_required: true,
            })
            .await
            .unwrap();

        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let envelope = rx.recv().await.unwrap();
                if envelope.turn_id.as_deref() != Some(&turn_id) {
                    continue;
                }
                match envelope.event {
                    RoderEvent::TurnCompleted(_) => break,
                    RoderEvent::TurnFailed(event) => panic!("turn failed: {}", event.error),
                    _ => {}
                }
            }
        })
        .await
        .unwrap();

        let requests = requests.lock().unwrap().clone();
        assert!(requests.len() >= 2);
        assert!(requests[1].transcript.iter().any(|item| {
            matches!(
                item,
                TranscriptItem::UserMessage(message)
                    if message.text.contains("Scoreable Output Checkpoint")
                        && message.text.contains("ensure the required output file(s) exist")
                        && message.text.contains("Write /app/result.txt")
            )
        }));
    }

    #[test]
    fn deadline_task_ledger_prompt_preserves_scoreable_work_instruction() {
        let prompt = task_ledger_deadline_completion_prompt(
            12,
            30,
            "Task Ledger Completion Required: write /app/result.txt, then call task_ledger.update",
        );

        assert!(prompt.contains("12 seconds remain"));
        assert!(prompt.contains("create or update the required scoreable output files"));
        assert!(prompt.contains("write /app/result.txt"));
        assert!(prompt.contains(TASK_LEDGER_TOOL_NAME));
    }

    #[test]
    fn scoreable_checkpoint_prompt_preserves_provisional_file_instruction() {
        let prompt = task_ledger_scoreable_checkpoint_prompt(
            120,
            "Task Ledger Completion Required: write /app/result.txt, then call task_ledger.update",
        );

        assert!(prompt.contains("120 seconds remain"));
        assert!(prompt.contains("best evidence-backed answer"));
        assert!(prompt.contains("even if provisional"));
        assert!(prompt.contains("preserve that candidate"));
        assert!(prompt.contains("partial-coverage"));
        assert!(prompt.contains("write /app/result.txt"));
        assert!(prompt.contains(TASK_LEDGER_TOOL_NAME));
    }

    #[test]
    fn open_task_ledger_moves_inference_timeout_to_scoreable_checkpoint() {
        let deadline = Some(OffsetDateTime::now_utc() + Duration::seconds(870));
        let transcript = vec![TranscriptItem::ToolResult(ToolResultRecord {
            id: "ledger-open".to_string(),
            name: Some(TASK_LEDGER_TOOL_NAME.to_string()),
            result: "Task ledger: 0/1 completed\n- pending: Write /app/result.txt [write]"
                .to_string(),
            display_payload: None,
            is_error: false,
        })];

        let (_, action) = inference_timeout_deadline(
            deadline,
            RuntimeProfile::Eval,
            true,
            30,
            false,
            0,
            &transcript,
        )
        .unwrap();

        assert_eq!(action, InferenceTimeoutAction::ScoreableCheckpoint);
    }

    #[tokio::test]
    async fn verification_gate_forces_eval_code_changes_through_review() {
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(VerificationGateEngine {
            calls: StdMutex::new(0),
        }));
        builder.tool_contributor(Arc::new(WriteFileContributor));
        builder.tool_contributor(Arc::new(
            roder_ext_verification::VerificationToolContributor,
        ));
        let runtime = Arc::new(
            Runtime::new(
                builder.build().unwrap(),
                RuntimeConfig {
                    default_provider: roder_api::catalog::PROVIDER_MOCK.to_string(),
                    default_model: "mock".to_string(),
                    runtime_profile: RuntimeProfile::Eval,
                    policy_mode: PolicyMode::Bypass,
                    agent_swarm_mode: false,
                    ..RuntimeConfig::default()
                },
            )
            .unwrap(),
        );
        let mut rx = runtime.subscribe_events();
        let turn_id = runtime
            .start_turn(StartTurnRequest {
                thread_id: "thread-verification".to_string(),
                message: "write code".to_string(),
                images: Vec::new(),
                provider_override: None,
                model_override: None,
                reasoning_override: None,
                workspace: test_workspace(),
                instructions: InstructionBundle::default(),
                developer_context: None,
                task_ledger_required: false,
            })
            .await
            .unwrap();

        let mut saw_required = false;
        let mut saw_completed = false;
        let mut final_text = String::new();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let envelope = rx.recv().await.unwrap();
                if envelope.turn_id.as_deref() != Some(&turn_id) {
                    continue;
                }
                match envelope.event {
                    RoderEvent::VerificationRequired(event) => {
                        saw_required = true;
                        assert_eq!(event.changed_files, vec!["src/lib.rs"]);
                    }
                    RoderEvent::VerificationCompleted(event) => {
                        saw_completed = true;
                        assert!(event.passed);
                    }
                    RoderEvent::InferenceEventReceived(event) => {
                        if let InferenceEvent::MessageDelta(delta) = event.event {
                            final_text.push_str(&delta.text);
                        }
                    }
                    RoderEvent::TurnCompleted(_) => break,
                    _ => {}
                }
            }
        })
        .await
        .unwrap();

        assert!(saw_required);
        assert!(saw_completed);
        assert!(final_text.contains("verified final"));
    }

    #[tokio::test]
    async fn speed_policy_changes_reasoning_across_eval_model_calls_without_model_switch() {
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(SpeedPolicyEngine {
            calls: StdMutex::new(0),
            requests: requests.clone(),
        }));
        builder.tool_contributor(Arc::new(WriteFileContributor));
        builder.tool_contributor(Arc::new(
            roder_ext_verification::VerificationToolContributor,
        ));
        let runtime = Arc::new(
            Runtime::new(
                builder.build().unwrap(),
                RuntimeConfig {
                    default_provider: roder_api::catalog::PROVIDER_MOCK.to_string(),
                    default_model: "gpt-5.5".to_string(),
                    runtime_profile: RuntimeProfile::Eval,
                    policy_mode: PolicyMode::Bypass,
                    agent_swarm_mode: false,
                    ..RuntimeConfig::default()
                },
            )
            .unwrap(),
        );
        let mut rx = runtime.subscribe_events();
        let turn_id = runtime
            .start_turn(StartTurnRequest {
                thread_id: "thread-speed-policy".to_string(),
                message: "write code".to_string(),
                images: Vec::new(),
                provider_override: None,
                model_override: None,
                reasoning_override: None,
                workspace: test_workspace(),
                instructions: InstructionBundle::default(),
                developer_context: None,
                task_ledger_required: false,
            })
            .await
            .unwrap();

        let mut saw_speed_policy_event = false;
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let envelope = rx.recv().await.unwrap();
                if envelope.turn_id.as_deref() != Some(&turn_id) {
                    continue;
                }
                match envelope.event {
                    RoderEvent::InferenceStarted(event) => {
                        if event.speed_policy.is_some() {
                            saw_speed_policy_event = true;
                        }
                    }
                    RoderEvent::TurnCompleted(_) => break,
                    RoderEvent::TurnFailed(event) => panic!("turn failed: {}", event.error),
                    _ => {}
                }
            }
        })
        .await
        .unwrap();

        let requests = requests.lock().unwrap().clone();
        assert!(saw_speed_policy_event);
        assert!(requests.len() >= 4);
        assert!(requests.iter().all(|request| {
            request.model.provider == roder_api::catalog::PROVIDER_MOCK
                && request.model.model == "gpt-5.5"
        }));
        assert_eq!(
            requests[0].runtime.speed_policy.as_ref().map(|d| d.phase),
            Some(roder_api::inference::SpeedPolicyPhase::Orientation)
        );
        assert_eq!(requests[0].reasoning.level.as_deref(), Some(REASONING_HIGH));
        assert_eq!(
            requests[1].runtime.speed_policy.as_ref().map(|d| d.phase),
            Some(roder_api::inference::SpeedPolicyPhase::Execution)
        );
        assert_eq!(requests[1].reasoning.level.as_deref(), Some(REASONING_LOW));
        assert_eq!(
            requests[2].runtime.speed_policy.as_ref().map(|d| d.phase),
            Some(roder_api::inference::SpeedPolicyPhase::Verification)
        );
        assert_eq!(requests[2].reasoning.level.as_deref(), Some(REASONING_HIGH));
        assert_eq!(
            requests[2]
                .metadata
                .pointer("/speedPolicy/phase")
                .and_then(serde_json::Value::as_str),
            Some("verification")
        );
    }

    #[tokio::test]
    async fn deadline_turn_timeout_emits_partial_result_and_clears_active_turn() {
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(DeadlineEngine));
        let runtime = Arc::new(
            Runtime::new(
                builder.build().unwrap(),
                RuntimeConfig {
                    default_provider: roder_api::catalog::PROVIDER_MOCK.to_string(),
                    default_model: "mock".to_string(),
                    runtime_profile: RuntimeProfile::Eval,
                    turn_deadline_seconds: Some(1),
                    ..RuntimeConfig::default()
                },
            )
            .unwrap(),
        );
        let mut rx = runtime.subscribe_events();
        let turn_id = runtime
            .start_turn(StartTurnRequest {
                thread_id: "thread-deadline".to_string(),
                message: "slow work".to_string(),
                images: Vec::new(),
                provider_override: None,
                model_override: None,
                reasoning_override: None,
                workspace: test_workspace(),
                instructions: InstructionBundle::default(),
                developer_context: None,
                task_ledger_required: false,
            })
            .await
            .unwrap();

        let mut saw_partial = false;
        let mut saw_deadline = false;
        let mut failed_kind = None;
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let envelope = rx.recv().await.unwrap();
                if envelope.turn_id.as_deref() != Some(&turn_id) {
                    continue;
                }
                match envelope.event {
                    RoderEvent::TurnPartialResult(event) => {
                        saw_partial = event.summary.contains("partial turn state");
                    }
                    RoderEvent::TurnDeadlineExceeded(event) => {
                        saw_deadline = event.partial_result.contains("transcript items");
                    }
                    RoderEvent::TurnFailed(event) => {
                        failed_kind = event.error_kind;
                        break;
                    }
                    _ => {}
                }
            }
        })
        .await
        .unwrap();

        for _ in 0..20 {
            if !runtime.active_turns.read().await.contains_key(&turn_id) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(saw_partial);
        assert!(saw_deadline);
        assert_eq!(failed_kind.as_deref(), Some("deadline_timeout"));
        assert!(!runtime.active_turns.read().await.contains_key(&turn_id));
    }

    #[tokio::test]
    async fn deadline_skips_subagent_task_when_remaining_budget_is_too_low() {
        let calls = Arc::new(StdMutex::new(0));
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(CapturingEngine {
            request: Arc::new(StdMutex::new(None)),
        }));
        let task_tool = Arc::new(CountingTaskTool {
            calls: calls.clone(),
        });
        builder.tool_contributor(Arc::new(TestToolContributor { tool: task_tool }));
        let runtime = Arc::new(
            Runtime::new(
                builder.build().unwrap(),
                RuntimeConfig {
                    policy_mode: PolicyMode::Bypass,
                    agent_swarm_mode: false,
                    ..RuntimeConfig::default()
                },
            )
            .unwrap(),
        );

        let result = runtime
            .route_tool_call(
                &"thread-deadline-task".to_string(),
                &"turn-deadline-task".to_string(),
                ToolCallCompleted {
                    id: "task-1".to_string(),
                    name: "task".to_string(),
                    arguments: serde_json::json!({
                        "description": "inspect",
                        "prompt": "read"
                    })
                    .to_string(),
                },
                None,
                Some(OffsetDateTime::now_utc() + Duration::seconds(1)),
            )
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.result.contains("deadline policy skipped"));
        assert_eq!(*calls.lock().unwrap(), 0);
    }

    struct TestToolContributor {
        tool: Arc<dyn ToolExecutor>,
    }

    impl ToolContributor for TestToolContributor {
        fn id(&self) -> String {
            "test-tool".to_string()
        }

        fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
            registry.register(self.tool.clone())
        }
    }

    #[tokio::test]
    async fn agent_swarm_mode_override_is_per_thread() {
        let runtime = Runtime::fake().unwrap();
        let trigger = roder_api::subagents::AgentSwarmModeTrigger::Manual;

        // Off everywhere by default.
        assert!(
            !runtime
                .effective_agent_swarm_mode_for_thread("thread-a")
                .await
        );
        assert!(
            !runtime
                .effective_agent_swarm_mode_for_thread("thread-b")
                .await
        );

        // Enabling on thread-a does not leak into thread-b.
        assert!(
            runtime
                .set_agent_swarm_mode_for_thread("thread-a", true, trigger)
                .await
        );
        assert!(
            runtime
                .effective_agent_swarm_mode_for_thread("thread-a")
                .await
        );
        assert!(
            !runtime
                .effective_agent_swarm_mode_for_thread("thread-b")
                .await
        );

        // A per-thread `off` override wins over a runtime-global `on` default.
        runtime.set_agent_swarm_mode(true, trigger).await.unwrap();
        runtime
            .set_agent_swarm_mode_for_thread("thread-b", false, trigger)
            .await;
        assert!(
            !runtime
                .effective_agent_swarm_mode_for_thread("thread-b")
                .await,
            "explicit per-thread off overrides the global on default"
        );
        // A thread with no override still follows the global default.
        assert!(
            runtime
                .effective_agent_swarm_mode_for_thread("thread-c")
                .await,
            "threads without an override follow the runtime-global default"
        );
    }

    #[tokio::test]
    async fn set_agent_swarm_mode_for_thread_emits_event_with_real_thread_id() {
        let runtime = Runtime::fake().unwrap();
        let mut events = runtime.subscribe_events();
        runtime
            .set_agent_swarm_mode_for_thread(
                "thread-xyz",
                true,
                roder_api::subagents::AgentSwarmModeTrigger::Task,
            )
            .await;
        let mut saw = false;
        for _ in 0..8 {
            let envelope = tokio::time::timeout(std::time::Duration::from_secs(2), events.recv())
                .await
                .unwrap()
                .unwrap();
            if let RoderEvent::AgentSwarmModeChanged(event) = envelope.event {
                assert_eq!(event.thread_id, "thread-xyz");
                assert!(event.enabled);
                assert_eq!(
                    event.trigger,
                    roder_api::subagents::AgentSwarmModeTrigger::Task
                );
                saw = true;
                break;
            }
        }
        assert!(
            saw,
            "expected an AgentSwarmModeChanged event for the thread"
        );
    }
}
