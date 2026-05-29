use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use futures::StreamExt;
use futures::future::{AbortHandle, Abortable, BoxFuture, try_join_all};
use roder_api::catalog::{
    EDIT_TOOL_EDIT, EDIT_TOOL_PATCH, PROVIDER_GEMINI, REASONING_NONE, built_in_model_profile,
    built_in_model_profile_for_provider, lookup_model,
};
use roder_api::context::PolicyGate;
use roder_api::events::*;
use roder_api::extension::ExtensionRegistry;
use roder_api::inference::{
    AgentInferenceRequest, HostedWebSearchConfig, HostedWebSearchMode, InferenceEngine,
    InferenceEvent, InferenceTurnContext, InstructionBundle, ModelHarnessProfile,
    ModelSchemaPolicy, ModelSelection, OutputConfig, ReasoningConfig, RuntimeHints, RuntimeProfile,
    TokenUsage, ToolCallCompleted,
};
use roder_api::policy_mode::{PolicyDecision, PolicyMode};
use roder_api::reliability::{
    ReliabilityContext, ReliabilityDetails, ReliabilityErrorClass, ReliabilityLimitRecorded,
    ReliabilityRequestPolicy, ReliabilityRetryDecision, ReliabilityRetryRecorded,
    provider_retry_delay_ms,
};
use roder_api::remote_runner::{RemoteRunnerSession, RunnerDestination};
use roder_api::subagents::SubagentDefinition;
use roder_api::teams::TeamMemberStatus;
use roder_api::thread::{
    ThreadItemEvent, ThreadItemEventKind, ThreadMetadata, ThreadSnapshot, ThreadStore,
    ThreadUsageMetadata, validate_thread_workspace,
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
use crate::instructions::{
    apply_model_instruction_overlay, apply_runtime_profile, apply_task_ledger_required,
};
use crate::policy_gate::DefaultPolicyGate;
use crate::reliability::{
    ReliabilityLimitHit, RuntimeReliabilityConfig, TurnReliabilityState,
    provider_stream_retry_cause,
};
pub use crate::speed_policy::RuntimeSpeedPolicyConfig;
use crate::speed_policy::{SpeedPolicyState, reasoning_from_decision};
use crate::subagent_traces::RuntimeSubagentTraceSink;
use crate::teams::{TeamManager, TeamMemberStartRequest, TeamStartRequest, TeamState};
use crate::thread_item_cache::{ThreadItemCache, ThreadItemCacheEntry};
use crate::verification_gate::VerificationGateState;

const MAX_TOOL_ROUNDS_PER_TURN: usize = 1024;
const FINAL_ANSWER_PHASE: &str = "final_answer";
const TASK_LEDGER_TOOL_NAME: &str = "task_ledger.update";
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
    pub model_edit_tools: HashMap<String, String>,
    pub model_parallel_tool_calls: HashMap<String, bool>,
    pub model_profiles: HashMap<String, ModelHarnessProfile>,
    pub command_shell: String,
    pub workspace: Option<String>,
    pub policy_mode: PolicyMode,
    pub runtime_profile: RuntimeProfile,
    pub speed_policy: RuntimeSpeedPolicyConfig,
    pub dynamic_workflows: RuntimeDynamicWorkflowConfig,
    pub reliability: RuntimeReliabilityConfig,
    pub turn_deadline_seconds: Option<u64>,
    pub remote_runner_destination: Option<RunnerDestination>,
    pub team_data_dir: Option<PathBuf>,
    pub roadmap_data_dir: Option<PathBuf>,
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
            model_edit_tools: HashMap::new(),
            model_parallel_tool_calls: HashMap::new(),
            model_profiles: HashMap::new(),
            command_shell: roder_api::command_shell::default_command_shell(),
            workspace: None,
            policy_mode: PolicyMode::Default,
            runtime_profile: RuntimeProfile::Interactive,
            speed_policy: RuntimeSpeedPolicyConfig::default(),
            dynamic_workflows: RuntimeDynamicWorkflowConfig::default(),
            reliability: RuntimeReliabilityConfig::default(),
            turn_deadline_seconds: None,
            remote_runner_destination: None,
            team_data_dir: None,
            roadmap_data_dir: None,
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
    pub task_ledger_required: bool,
}

#[derive(Debug, Clone)]
pub struct CreateThreadRequest {
    pub title: Option<String>,
    pub workspace: String,
    pub provider: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingPlanExit {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub request_id: String,
    pub target_mode: PolicyMode,
    pub plan_summary: Option<String>,
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

#[derive(Clone)]
struct ActiveTurnHandle {
    thread_id: ThreadId,
    abort: AbortHandle,
    steers: Arc<Mutex<Vec<UserMessage>>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThreadActivity {
    pub active_turn_id: Option<TurnId>,
    pub active_flags: Vec<String>,
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
    ) -> Self {
        let requested_at = OffsetDateTime::now_utc();
        Self {
            thread_id,
            turn_id,
            request_id,
            target_mode,
            plan_summary,
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

pub struct Runtime {
    pub bus: EventBus,
    pub registry: ExtensionRegistry,
    config: RwLock<RuntimeConfig>,
    pending_plan_exit: RwLock<Option<PendingPlanExit>>,
    pub(crate) pending_tool_approvals: Mutex<HashMap<String, PendingToolApproval>>,
    pub(crate) pending_user_inputs: Mutex<HashMap<String, PendingUserInput>>,
    active_turns: RwLock<HashMap<TurnId, ActiveTurnHandle>>,
    workspace: PathBuf,
    teams: TeamManager,
    pub(crate) roadmaps: Mutex<roder_roadmap::RoadmapRuntime>,
    pub(crate) goals: Arc<RuntimeGoalController>,
    context_artifacts: roder_api::artifacts::ContextArtifactStore,
    pub(crate) thread_store: Option<Arc<dyn ThreadStore>>,
    thread_item_cache: Mutex<ThreadItemCache>,
    pub(crate) tool_registry: ToolRegistry,
    pub(crate) skills: RwLock<SkillRegistry>,
}

impl Runtime {
    pub fn new(registry: ExtensionRegistry, config: RuntimeConfig) -> anyhow::Result<Self> {
        if registry.inference_engines.is_empty() {
            anyhow::bail!("at least one inference engine must be registered");
        }
        validate_runtime_config_reasoning(&config)?;

        let bus = EventBus::new(1024);
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
            active_turns: RwLock::new(HashMap::new()),
            workspace: workspace.clone(),
            teams: TeamManager::new(
                team_data_dir.unwrap_or_else(crate::teams::default_team_data_dir),
            ),
            roadmaps: Mutex::new(roder_roadmap::RoadmapRuntime::new(
                workspace,
                roadmap_data_dir,
            )),
            goals,
            context_artifacts,
            thread_store,
            thread_item_cache: Mutex::new(ThreadItemCache::default()),
            tool_registry,
            skills: RwLock::new(SkillRegistry::load(SkillRegistryOptions::new(
                PathBuf::new(),
            ))),
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
            provider: None,
            model: None,
        })
        .await
    }

    pub async fn create_thread_with(
        &self,
        req: CreateThreadRequest,
    ) -> anyhow::Result<ThreadMetadata> {
        let cfg = self.config.read().await.clone();
        let now = OffsetDateTime::now_utc();
        let workspace = validate_thread_workspace(&req.workspace)?;
        let metadata = ThreadMetadata {
            thread_id: uuid::Uuid::new_v4().to_string(),
            title: req.title,
            workspace,
            provider: Some(req.provider.unwrap_or(cfg.default_provider)),
            model: Some(req.model.unwrap_or(cfg.default_model)),
            runner_destination: cfg.remote_runner_destination.clone(),
            runner_state: None,
            created_at: now,
            updated_at: now,
            message_count: 0,
            usage: None,
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
                    provider: None,
                    model: None,
                })
                .await?
                .thread_id
            }
        };
        let team_id = uuid::Uuid::new_v4().to_string();
        let mut members = vec![crate::teams::lead_member(
            lead_thread_id.clone(),
            Some(cfg.default_provider.clone()),
            Some(cfg.default_model.clone()),
            cfg.policy_mode,
        )];

        for (index, member) in req.members.into_iter().enumerate() {
            let thread = self
                .create_thread_with(CreateThreadRequest {
                    title: Some(member.name.clone()),
                    workspace: workspace.clone(),
                    provider: member.model_provider.clone(),
                    model: member.model.clone(),
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
            self.emit(RoderEvent::TeamMemberStarted(TeamMemberStarted {
                team_id: team_id.clone(),
                member_id,
                member_thread_id: thread.thread_id,
                role: descriptor.role,
                name: descriptor.name.clone(),
                timestamp: OffsetDateTime::now_utc(),
            }))
            .await;
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
            team_id,
            lead_thread_id,
            display_mode: team.display_mode,
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        Ok(team)
    }

    pub async fn list_teams(&self) -> Vec<TeamState> {
        self.teams.list().await
    }

    pub async fn read_team(&self, team_id: &str) -> Option<TeamState> {
        self.teams.get(team_id).await
    }

    pub async fn start_team_member(
        &self,
        team_id: &str,
        req: TeamMemberStartRequest,
    ) -> anyhow::Result<TeamState> {
        let cfg = self.config.read().await.clone();
        let thread = self
            .create_thread_with(CreateThreadRequest {
                title: Some(req.name.clone()),
                workspace: self.workspace.display().to_string(),
                provider: req.model_provider.clone(),
                model: req.model.clone(),
            })
            .await?;
        let team = self
            .read_team(team_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("unknown team {team_id:?}"))?;
        let member_id = format!("member-{}", team.members.len());
        let descriptor = crate::teams::teammate_member(
            member_id.clone(),
            req.name,
            thread.thread_id.clone(),
            req.model_provider.or(thread.provider),
            req.model.or(thread.model),
            cfg.policy_mode,
        );
        let mut next = team;
        next.members.push(descriptor.clone());
        next.updated_at = OffsetDateTime::now_utc();
        let next = self.teams.insert(next).await?;
        self.emit(RoderEvent::TeamMemberStarted(TeamMemberStarted {
            team_id: next.id.clone(),
            member_id,
            member_thread_id: descriptor.thread_id,
            role: descriptor.role,
            name: descriptor.name,
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        Ok(next)
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
        let member = team
            .members
            .iter()
            .find(|member| member.id == member_id)
            .ok_or_else(|| anyhow::anyhow!("unknown team member {member_id:?}"))?
            .clone();
        if member.status == TeamMemberStatus::Closed {
            anyhow::bail!("subagent {} is closed", member.name);
        }
        self.teams
            .append_mailbox_message(team_id, None, member_id.to_string(), message.clone())
            .await?;
        let workspace = self.workspace.display().to_string();
        let turn_id = if member.status == TeamMemberStatus::Running {
            if let Some(turn_id) = member.current_turn_id.clone() {
                self.steer_turn(
                    member.thread_id.clone(),
                    turn_id.clone(),
                    message,
                    Vec::new(),
                )
                .await?;
                turn_id
            } else {
                self.start_turn(StartTurnRequest {
                    thread_id: member.thread_id.clone(),
                    message,
                    images: Vec::new(),
                    provider_override: member.model_provider.clone(),
                    model_override: member.model.clone(),
                    reasoning_override: None,
                    workspace: workspace.clone(),
                    instructions: crate::default_instructions(),
                    task_ledger_required: false,
                })
                .await?
            }
        } else {
            self.start_turn(StartTurnRequest {
                thread_id: member.thread_id.clone(),
                message,
                images: Vec::new(),
                provider_override: member.model_provider.clone(),
                model_override: member.model.clone(),
                reasoning_override: None,
                workspace,
                instructions: crate::default_instructions(),
                task_ledger_required: false,
            })
            .await?
        };
        let is_active = self.active_turns.read().await.contains_key(&turn_id);
        self.teams
            .update_member(team_id, member_id, |member| {
                if is_active {
                    member.current_turn_id = Some(turn_id.clone());
                    member.status = TeamMemberStatus::Running;
                } else {
                    member.current_turn_id = None;
                    member.status = TeamMemberStatus::Completed;
                }
            })
            .await?;
        if is_active {
            self.emit(RoderEvent::TeamMemberStatusChanged(
                TeamMemberStatusChanged {
                    team_id: team_id.to_string(),
                    member_id: member_id.to_string(),
                    member_thread_id: member.thread_id,
                    status: TeamMemberStatus::Running,
                    timestamp: OffsetDateTime::now_utc(),
                },
            ))
            .await;
        } else {
            self.emit(RoderEvent::TeamMemberCompleted(TeamMemberCompleted {
                team_id: team_id.to_string(),
                member_id: member_id.to_string(),
                member_thread_id: member.thread_id,
                turn_id: Some(turn_id.clone()),
                status: TeamMemberStatus::Completed,
                timestamp: OffsetDateTime::now_utc(),
            }))
            .await;
        }
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
        let Some(turn_id) = member.current_turn_id.clone() else {
            return Ok(None);
        };
        self.interrupt_turn(member.thread_id.clone(), turn_id.clone())
            .await?;
        self.teams
            .update_member(team_id, member_id, |member| {
                member.status = TeamMemberStatus::Interrupted;
                member.current_turn_id = None;
            })
            .await?;
        self.emit(RoderEvent::TeamMemberCompleted(TeamMemberCompleted {
            team_id: team_id.to_string(),
            member_id: member_id.to_string(),
            member_thread_id: member.thread_id,
            turn_id: Some(turn_id.clone()),
            status: TeamMemberStatus::Interrupted,
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
        self.emit(crate::agent_control_tools::closed_member_event(
            team_id.to_string(),
            &closed,
            interrupted_turn_id,
        ))
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
        let removed = self.teams.remove(team_id).await?.is_some();
        if removed {
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

    async fn complete_team_member_turn(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        status: TeamMemberStatus,
    ) -> anyhow::Result<()> {
        let Some((team_id, member)) = self
            .teams
            .complete_member_turn(thread_id, turn_id, status)
            .await?
        else {
            return Ok(());
        };
        self.emit(RoderEvent::TeamMemberCompleted(TeamMemberCompleted {
            team_id,
            member_id: member.id,
            member_thread_id: member.thread_id,
            turn_id: Some(turn_id.clone()),
            status,
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
                .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;
            let metadata = snapshot
                .metadata
                .ok_or_else(|| anyhow::anyhow!("thread metadata missing for {thread_id}"))?;
            return Ok(metadata.workspace);
        }
        Ok(self.workspace.display().to_string())
    }

    async fn runner_session_for_thread(
        &self,
        thread_id: &ThreadId,
    ) -> anyhow::Result<Option<(RunnerDestination, Arc<dyn RemoteRunnerSession>)>> {
        let Some(destination) = self.config.read().await.remote_runner_destination.clone() else {
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
        let persisted_state = if let Some(store) = &self.thread_store {
            store
                .load_thread(thread_id)
                .await?
                .and_then(|snapshot| snapshot.metadata)
                .and_then(|metadata| metadata.runner_state)
        } else {
            None
        };
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
            let cfg = self.config.read().await.clone();
            let provider = req
                .provider_override
                .clone()
                .unwrap_or_else(|| cfg.default_provider.clone());
            self.engine_for(&provider)?;
            let turn_id = uuid::Uuid::new_v4().to_string();
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
            let runtime = Arc::clone(self);
            let turn_req = req;
            let thread_id_for_task = turn_req.thread_id.clone();
            let turn_id_for_task = turn_id.clone();
            tokio::spawn(async move {
                let result = Abortable::new(
                    runtime.run_turn(turn_req, turn_id_for_task.clone()),
                    abort_registration,
                )
                .await;
                let completed = matches!(&result, Ok(Ok(TurnRunOutcome::Completed)));
                if let Ok(Err(err)) = &result {
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
                }
                runtime.active_turns.write().await.remove(&turn_id_for_task);
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
        let message = message.trim().to_string();
        if message.is_empty() && images.is_empty() {
            return Ok(());
        }

        let Some(active) = self.active_turns.read().await.get(&turn_id).cloned() else {
            anyhow::bail!("no active turn to steer");
        };
        active
            .steers
            .lock()
            .await
            .push(UserMessage::with_images(message.clone(), images));
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
        self.filtered_tool_specs(&cfg, &cfg.default_model, model_profile.as_ref())
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

        let mut cfg = self.config.read().await.clone();
        if let Some(reasoning) = &req.reasoning_override {
            validate_reasoning_effort(
                req.model_override
                    .as_deref()
                    .unwrap_or(cfg.default_model.as_str()),
                reasoning,
            )?;
            cfg.reasoning = Some(reasoning.clone());
        }
        let runtime_profile = cfg.runtime_profile;
        let turn_deadline = turn_deadline_for_config(&cfg);
        let deadline_finalization_reserve =
            crate::deadline_policy::finalization_reserve_seconds(cfg.turn_deadline_seconds);
        let provider = req
            .provider_override
            .clone()
            .unwrap_or(cfg.default_provider.clone());
        let model = req
            .model_override
            .clone()
            .unwrap_or(cfg.default_model.clone());
        let engine = self.engine_for(&provider)?;
        let capabilities = engine.capabilities();
        let model_profile = model_profile_for_provider_model(&cfg, &provider, &model);
        let tools = if capabilities.tool_calls {
            self.filtered_tool_specs(&cfg, &model, model_profile.as_ref())
        } else {
            Vec::new()
        };
        let workspace = req.workspace.clone();
        let parallel_tool_calls = parallel_tool_calls_for_model(&cfg, &model);
        let tool_choice = if tools.is_empty() {
            ToolChoice::None
        } else {
            ToolChoice::Auto
        };
        let mut transcript = self.transcript_for_turn(&req, &turn_id, &model).await?;
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
        let runner_session = self.runner_session_for_thread(&req.thread_id).await?;
        if !capabilities.image_input && transcript_has_images(&transcript) {
            self.fail_turn_with_error(
                &req.thread_id,
                &turn_id,
                format!("provider {provider} does not support image input"),
            )
            .await?;
            return Ok(TurnRunOutcome::Stopped);
        }
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
        let mut deadline_finalization_requested = false;
        let mut deadline_scoreable_completion_requested = false;
        let mut task_ledger_completion_reminders = 0_u8;
        let mut task_ledger_scoreable_checkpoints = 0_u8;
        let mut provider_stream_retry_attempts = 0_u32;

        'tool_rounds: for _ in 0..MAX_TOOL_ROUNDS_PER_TURN {
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
            if !capabilities.image_input && transcript_has_images(&transcript) {
                self.fail_turn_with_error(
                    &req.thread_id,
                    &turn_id,
                    format!("provider {provider} does not support image input"),
                )
                .await?;
                return Ok(TurnRunOutcome::Stopped);
            }

            let speed_policy_decision =
                speed_policy.decision(runtime_profile, &model, &cfg.speed_policy);
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
                speed_policy: speed_policy_decision.clone(),
                deadline_remaining_seconds: deadline_remaining_seconds(turn_deadline),
                timestamp: OffsetDateTime::now_utc(),
            }))
            .await;

            let mut instructions = apply_runtime_profile(req.instructions.clone(), runtime_profile);
            if let Some(profile) = &model_profile {
                instructions = apply_model_instruction_overlay(instructions, profile);
            }
            if req.task_ledger_required
                && runtime_profile == RuntimeProfile::Eval
                && !transcript_has_task_ledger(&transcript)
            {
                instructions = apply_task_ledger_required(instructions);
            }
            instructions = self
                .goals
                .apply_goal_instructions(&req.thread_id, instructions)
                .await?;
            let mut request_metadata = serde_json::json!({});
            if let Some(decision) = &speed_policy_decision {
                request_metadata["speedPolicy"] = serde_json::json!(decision);
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
                reasoning: reasoning_from_decision(
                    speed_policy_decision.as_ref(),
                    reasoning_for_model(&cfg, &model),
                ),
                output: OutputConfig::default(),
                runtime: RuntimeHints {
                    auto_compact_token_limit: server_side_compaction_threshold(&cfg, &model),
                    profile: runtime_profile,
                    parallel_tool_calls: Some(parallel_tool_calls),
                    hosted_web_search: cfg.hosted_web_search.clone(),
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
                            error,
                            error_kind: None,
                            usage: None,
                            timestamp: OffsetDateTime::now_utc(),
                        }))
                        .await;
                        self.complete_team_member_turn(
                            &req.thread_id,
                            &turn_id,
                            TeamMemberStatus::Failed,
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
                        self.persist_turn_item(
                            &req.thread_id,
                            &turn_id,
                            &TranscriptItem::Error(ErrorRecord {
                                message: failure.message.clone(),
                            }),
                        )
                        .await?;
                        self.emit(RoderEvent::TurnFailed(TurnFailed {
                            thread_id: req.thread_id.clone(),
                            turn_id: turn_id.clone(),
                            error: failure.message,
                            error_kind: None,
                            usage: None,
                            timestamp: OffsetDateTime::now_utc(),
                        }))
                        .await;
                        self.complete_team_member_turn(
                            &req.thread_id,
                            &turn_id,
                            TeamMemberStatus::Failed,
                        )
                        .await?;
                        return Ok(TurnRunOutcome::Stopped);
                    }
                    InferenceEvent::Usage(usage) => {
                        turn_usage.add_assign(&usage);
                    }
                    InferenceEvent::Completed(_)
                    | InferenceEvent::Compaction(_)
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
                        let item = TranscriptItem::ProviderMetadata(metadata);
                        self.persist_turn_item(&req.thread_id, &turn_id, &item)
                            .await?;
                        transcript.push(item);
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
                let item = TranscriptItem::ProviderMetadata(metadata);
                self.persist_turn_item(&req.thread_id, &turn_id, &item)
                    .await?;
                transcript.push(item);
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
            if let Some(limit) = reliability.record_tool_results(
                &cfg.reliability,
                &results,
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
            transcript = self
                .compact_transcript_if_needed(&req.thread_id, &turn_id, &model, transcript)
                .await?;
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
                error: message,
                error_kind: None,
                usage: None,
                timestamp: OffsetDateTime::now_utc(),
            }))
            .await;
            self.complete_team_member_turn(&req.thread_id, &turn_id, TeamMemberStatus::Failed)
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
                    text: final_assistant_text,
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
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        self.complete_team_member_turn(&req.thread_id, &turn_id, TeamMemberStatus::Completed)
            .await?;
        self.persist_runner_state(&req.thread_id, runner_session.as_ref())
            .await?;
        Ok(TurnRunOutcome::Completed)
    }

    async fn drain_turn_steers(&self, turn_id: &TurnId) -> Vec<UserMessage> {
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
        let force_sequential = calls
            .iter()
            .any(|call| crate::agent_control_tools::is_agent_control_tool(&call.name));
        if parallel && !force_sequential {
            try_join_all(
                calls.into_iter().map(|call| {
                    self.route_tool_call(thread_id, turn_id, call, workspace, deadline)
                }),
            )
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
        }
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
            error: message,
            error_kind: None,
            usage: None,
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        self.complete_team_member_turn(thread_id, turn_id, TeamMemberStatus::Failed)
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
            error: message,
            error_kind: Some("deadline_timeout".to_string()),
            usage: None,
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        self.complete_team_member_turn(thread_id, turn_id, TeamMemberStatus::Failed)
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
            error: message,
            error_kind: Some("reliability_limit".to_string()),
            usage: None,
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        self.complete_team_member_turn(thread_id, turn_id, TeamMemberStatus::Failed)
            .await?;
        Ok(())
    }

    async fn append_steers(
        &self,
        req: &StartTurnRequest,
        turn_id: &TurnId,
        transcript: &mut Vec<TranscriptItem>,
        steers: Vec<UserMessage>,
    ) -> anyhow::Result<()> {
        for mut steer in steers {
            steer.text = steer.text.trim().to_string();
            if steer.text.is_empty() && steer.images.is_empty() {
                continue;
            }
            let item = TranscriptItem::UserMessage(steer);
            self.persist_turn_item(&req.thread_id, turn_id, &item)
                .await?;
            transcript.push(item);
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

    fn filtered_tool_specs(
        &self,
        cfg: &RuntimeConfig,
        model: &str,
        profile: Option<&ModelHarnessProfile>,
    ) -> Vec<roder_api::tools::ToolSpec> {
        self.tool_registry.specs_for_edit_tool_with_schema_policy(
            edit_tool_for_model(cfg, model),
            schema_policy_for_model(profile),
        )
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

    fn engine_for(&self, provider: &str) -> anyhow::Result<Arc<dyn InferenceEngine>> {
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

fn parallel_tool_calls_for_model(cfg: &RuntimeConfig, model: &str) -> bool {
    cfg.model_parallel_tool_calls
        .get(model)
        .copied()
        .or_else(|| {
            model_profile_for_model(cfg, model).and_then(|profile| profile.parallel_tool_calls)
        })
        .unwrap_or(true)
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
    thread_id != "runtime"
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream;
    use roder_api::catalog::{
        REASONING_HIGH, REASONING_LOW, REASONING_MEDIUM, REASONING_MINIMAL, REASONING_NONE,
        REASONING_XHIGH,
    };
    use roder_api::extension::ExtensionRegistryBuilder;
    use roder_api::inference::{
        CompletionMetadata, InferenceCapabilities, InferenceEngine, InferenceEventStream,
        InferenceProviderContext, InferenceTurnContext, MessageDelta, ModelInstructionOverlay,
        ModelProfileReasoning, ModelSchemaPolicy, ProviderFamily,
    };
    use roder_api::tools::{ToolContributor, ToolExecutor, ToolSpec};
    use roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory;
    use std::sync::Mutex as StdMutex;

    fn test_workspace() -> String {
        std::env::current_dir().unwrap().display().to_string()
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
    async fn automations_can_create_project_thread_with_model_overrides() {
        let runtime = Runtime::fake().unwrap();
        let metadata = runtime
            .create_thread_with(CreateThreadRequest {
                title: Some("Automation: nightly status".to_string()),
                workspace: "/tmp/project".to_string(),
                provider: Some("mock".to_string()),
                model: Some("mock".to_string()),
            })
            .await
            .unwrap();

        assert_eq!(
            metadata.title.as_deref(),
            Some("Automation: nightly status")
        );
        assert_eq!(metadata.workspace, "/tmp/project");
        assert_eq!(metadata.provider.as_deref(), Some("mock"));
        assert_eq!(metadata.model.as_deref(), Some("mock"));
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
                },
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
        assert!(!tool_names.contains(&"apply_patch"));
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
                provider: None,
                model: None,
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
                },
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
}
