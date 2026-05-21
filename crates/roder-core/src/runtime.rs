use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use futures::StreamExt;
use futures::future::{AbortHandle, Abortable, try_join_all};
use roder_api::catalog::{EDIT_TOOL_EDIT, EDIT_TOOL_PATCH, REASONING_NONE, lookup_model};
use roder_api::context::PolicyGate;
use roder_api::conversation::{
    AssistantMessage, ConversationItem, ErrorRecord, InputImage, ReasoningSummary, ToolCallRecord,
    ToolResultRecord, UserMessage,
};
use roder_api::events::*;
use roder_api::extension::ExtensionRegistry;
use roder_api::inference::{
    AgentInferenceRequest, HostedWebSearchConfig, HostedWebSearchMode, InferenceEngine,
    InferenceEvent, InferenceTurnContext, InstructionBundle, ModelSelection, OutputConfig,
    ReasoningConfig, RuntimeHints, RuntimeProfile, ToolCallCompleted,
};
use roder_api::policy_mode::{PolicyDecision, PolicyMode};
use roder_api::remote_runner::{RemoteRunnerSession, RunnerDestination};
use roder_api::session::{SessionMetadata, SessionStore, ThreadSnapshot};
use roder_api::subagents::SubagentDefinition;
use roder_api::teams::TeamMemberStatus;
use roder_api::tools::{ToolCall, ToolChoice, ToolExecutionContext, ToolRegistry, ToolResult};
use roder_sandbox::ScopedFilesystem;
use roder_sandbox::process::LocalProcessRunner;
use time::{Duration, OffsetDateTime};
use tokio::sync::{Mutex, RwLock, oneshot};

use crate::artifacts::{ContextArtifactStore, default_context_artifact_dir};
use crate::bus::EventBus;
use crate::fake_provider::FakeInferenceEngine;
use crate::instructions::{apply_runtime_profile, apply_task_ledger_required};
use crate::policy_gate::DefaultPolicyGate;
pub use crate::speed_policy::RuntimeSpeedPolicyConfig;
use crate::speed_policy::{SpeedPolicyState, reasoning_from_decision};
use crate::subagent_traces::RuntimeSubagentTraceSink;
use crate::teams::{TeamManager, TeamMemberStartRequest, TeamStartRequest, TeamState};
use crate::verification_gate::VerificationGateState;

const MAX_TOOL_ROUNDS_PER_TURN: usize = 1024;
const FINAL_ANSWER_PHASE: &str = "final_answer";
pub(crate) const MIN_CHILD_DEADLINE_SECONDS: u64 = 2;

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
    pub workspace: Option<String>,
    pub policy_mode: PolicyMode,
    pub runtime_profile: RuntimeProfile,
    pub speed_policy: RuntimeSpeedPolicyConfig,
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
            workspace: None,
            policy_mode: PolicyMode::Default,
            runtime_profile: RuntimeProfile::Interactive,
            speed_policy: RuntimeSpeedPolicyConfig::default(),
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
    pub workspace: Option<String>,
    pub instructions: InstructionBundle,
    pub task_ledger_required: bool,
}

#[derive(Debug, Clone, Default)]
pub struct CreateSessionRequest {
    pub title: Option<String>,
    pub workspace: Option<String>,
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
    abort: AbortHandle,
    steers: Arc<Mutex<Vec<UserMessage>>>,
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
    context_artifacts: Arc<ContextArtifactStore>,
    pub(crate) session_store: Option<Arc<dyn SessionStore>>,
    pub(crate) tool_registry: ToolRegistry,
}

impl Runtime {
    pub fn new(registry: ExtensionRegistry, config: RuntimeConfig) -> anyhow::Result<Self> {
        if registry.inference_engines.is_empty() {
            anyhow::bail!("at least one inference engine must be registered");
        }

        let bus = EventBus::new(1024);
        let session_store = registry
            .session_stores
            .first()
            .map(|factory| factory.create());
        let mut tool_registry = ToolRegistry::default();
        for contributor in &registry.tools {
            contributor
                .contribute(&mut tool_registry)
                .with_context(|| format!("tool contributor {} failed", contributor.id()))?;
        }

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
        let context_artifacts = Arc::new(
            session_store
                .as_ref()
                .and_then(|store| store.local_session_root())
                .map(ContextArtifactStore::new_session_scoped)
                .unwrap_or_else(|| ContextArtifactStore::new(default_context_artifact_dir())),
        );
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
            context_artifacts,
            session_store,
            tool_registry,
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

    pub fn context_artifacts(&self) -> Arc<ContextArtifactStore> {
        Arc::clone(&self.context_artifacts)
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
        );
        executor.execute(ctx, tool_call).await
    }

    pub(crate) fn tool_execution_context(
        &self,
        thread_id: ThreadId,
        turn_id: TurnId,
        mode: PolicyMode,
        workspace: Option<&str>,
    ) -> ToolExecutionContext {
        let mut ctx = ToolExecutionContext::new(thread_id, turn_id, mode)
            .with_process_runner(Arc::new(LocalProcessRunner))
            .with_context_artifacts(self.context_artifacts.clone())
            .with_subagent_trace_sink(Arc::new(RuntimeSubagentTraceSink::new(
                self.bus.clone(),
                self.session_store.clone(),
            )));
        if let Some(workspace) = workspace {
            ctx = ctx.with_workspace_handle(Arc::new(ScopedFilesystem::new(workspace)));
        }
        ctx
    }

    pub async fn status(&self) -> RuntimeConfig {
        self.config.read().await.clone()
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
        self.engine_for(&provider)?;
        let mut cfg = self.config.write().await;
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
        Ok(cfg.clone())
    }

    pub async fn effective_reasoning(&self) -> String {
        let cfg = self.config.read().await;
        effective_reasoning_for_model(&cfg, &cfg.default_model)
    }

    pub async fn create_session(&self, title: Option<String>) -> anyhow::Result<SessionMetadata> {
        self.create_session_with(CreateSessionRequest {
            title,
            ..CreateSessionRequest::default()
        })
        .await
    }

    pub async fn create_session_with(
        &self,
        req: CreateSessionRequest,
    ) -> anyhow::Result<SessionMetadata> {
        let cfg = self.config.read().await.clone();
        let now = OffsetDateTime::now_utc();
        let metadata = SessionMetadata {
            thread_id: uuid::Uuid::new_v4().to_string(),
            title: req.title,
            workspace: req.workspace.or(cfg.workspace),
            provider: Some(req.provider.unwrap_or(cfg.default_provider)),
            model: Some(req.model.unwrap_or(cfg.default_model)),
            runner_destination: cfg.remote_runner_destination.clone(),
            runner_state: None,
            created_at: now,
            updated_at: now,
            message_count: 0,
        };

        let metadata = if let Some(store) = &self.session_store {
            store.create_session(metadata).await?
        } else {
            metadata
        };
        self.emit(RoderEvent::SessionCreated(SessionCreated {
            thread_id: metadata.thread_id.clone(),
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        Ok(metadata)
    }

    pub async fn list_sessions(&self) -> anyhow::Result<Vec<SessionMetadata>> {
        if let Some(store) = &self.session_store {
            return store.list_sessions().await;
        }
        Ok(Vec::new())
    }

    pub async fn archive_session(&self, thread_id: &str) -> anyhow::Result<bool> {
        if let Some(store) = &self.session_store {
            return store.archive_session(&thread_id.to_string()).await;
        }
        Ok(false)
    }

    pub async fn start_team(&self, req: TeamStartRequest) -> anyhow::Result<TeamState> {
        let cfg = self.config.read().await.clone();
        let lead_thread_id = match req.lead_thread_id {
            Some(thread_id) => thread_id,
            None => {
                self.create_session_with(CreateSessionRequest {
                    title: Some("Team lead".to_string()),
                    ..CreateSessionRequest::default()
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
                .create_session_with(CreateSessionRequest {
                    title: Some(member.name.clone()),
                    provider: member.model_provider.clone(),
                    model: member.model.clone(),
                    ..CreateSessionRequest::default()
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
            .create_session_with(CreateSessionRequest {
                title: Some(req.name.clone()),
                provider: req.model_provider.clone(),
                model: req.model.clone(),
                ..CreateSessionRequest::default()
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
        self.teams
            .append_mailbox_message(team_id, None, member_id.to_string(), message.clone())
            .await?;
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
                    workspace: None,
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
                workspace: None,
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

    pub async fn load_session(
        &self,
        thread_id: &ThreadId,
    ) -> anyhow::Result<Option<ThreadSnapshot>> {
        let loaded = if let Some(store) = &self.session_store {
            store.load_session(thread_id).await?
        } else {
            None
        };
        if loaded.is_some() {
            self.emit(RoderEvent::SessionLoaded(SessionLoaded {
                thread_id: thread_id.clone(),
                timestamp: OffsetDateTime::now_utc(),
            }))
            .await;
        }
        Ok(loaded)
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
        let persisted_state = if let Some(store) = &self.session_store {
            store
                .load_session(thread_id)
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
        let Some(store) = &self.session_store else {
            return Ok(());
        };
        let Some(snapshot) = store.load_session(thread_id).await? else {
            return Ok(());
        };
        let Some(mut metadata) = snapshot.metadata else {
            return Ok(());
        };
        metadata.runner_destination = Some(destination.clone());
        metadata.runner_state = Some(session.state());
        metadata.updated_at = OffsetDateTime::now_utc();
        store.update_session_metadata(metadata).await?;
        Ok(())
    }

    pub async fn start_turn(self: &Arc<Self>, req: StartTurnRequest) -> anyhow::Result<TurnId> {
        let cfg = self.config.read().await.clone();
        let provider = req
            .provider_override
            .clone()
            .unwrap_or_else(|| cfg.default_provider.clone());
        self.engine_for(&provider)?;
        let turn_id = uuid::Uuid::new_v4().to_string();
        let (abort_handle, abort_registration) = AbortHandle::new_pair();
        let active = ActiveTurnHandle {
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
            if let Ok(Err(err)) = result {
                // run_turn emits failures after the stream starts; this covers setup/startup errors.
                runtime
                    .emit(RoderEvent::TurnFailed(TurnFailed {
                        thread_id: thread_id_for_task,
                        turn_id: turn_id_for_task.clone(),
                        error: err.to_string(),
                        error_kind: None,
                        timestamp: OffsetDateTime::now_utc(),
                    }))
                    .await;
            }
            runtime.active_turns.write().await.remove(&turn_id_for_task);
        });
        Ok(turn_id)
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
        self.filtered_tool_specs(&cfg, &cfg.default_model)
    }

    pub fn subagent_definitions(&self) -> Vec<SubagentDefinition> {
        self.registry
            .subagent_dispatchers
            .iter()
            .flat_map(|dispatcher| dispatcher.definitions())
            .collect()
    }

    async fn run_turn(&self, req: StartTurnRequest, turn_id: TurnId) -> anyhow::Result<()> {
        self.emit(RoderEvent::TurnStarted(TurnStarted {
            thread_id: req.thread_id.clone(),
            turn_id: turn_id.clone(),
            runtime_profile: self.config.read().await.runtime_profile,
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        self.persist_turn_item(
            &req.thread_id,
            &turn_id,
            &ConversationItem::UserMessage(UserMessage::with_images(
                req.message.clone(),
                req.images.clone(),
            )),
        )
        .await?;

        let cfg = self.config.read().await.clone();
        let runtime_profile = cfg.runtime_profile;
        let turn_deadline = turn_deadline_for_config(&cfg);
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
        let tools = self.filtered_tool_specs(&cfg, &model);
        let workspace = req.workspace.clone().or_else(|| cfg.workspace.clone());
        let parallel_tool_calls = parallel_tool_calls_for_model(&cfg, &model);
        let tool_choice = if tools.is_empty() {
            ToolChoice::None
        } else {
            ToolChoice::Auto
        };
        let mut conversation = self.conversation_for_turn(&req, &turn_id, &model).await?;
        let runner_session = self.runner_session_for_thread(&req.thread_id).await?;
        if !capabilities.image_input && conversation_has_images(&conversation) {
            self.fail_turn_with_error(
                &req.thread_id,
                &turn_id,
                format!("provider {provider} does not support image input"),
            )
            .await?;
            return Ok(());
        }
        let mut final_assistant_text = String::new();
        let mut final_phase_messages = Vec::<AssistantMessage>::new();
        let mut final_reasoning_text = String::new();
        let mut final_provider_metadata = None;
        let mut exhausted_tool_rounds = true;
        let mut verification_gate =
            VerificationGateState::new(req.message.clone(), runtime_profile);
        let mut speed_policy = SpeedPolicyState::default();

        for _ in 0..MAX_TOOL_ROUNDS_PER_TURN {
            if let Some(deadline) = turn_deadline
                && deadline_expired(deadline)
            {
                self.fail_turn_due_to_deadline(&req.thread_id, &turn_id, deadline, &conversation)
                    .await?;
                return Ok(());
            }
            let steers = self.drain_turn_steers(&turn_id).await;
            self.append_steers(&req, &turn_id, &mut conversation, steers)
                .await?;
            if !capabilities.image_input && conversation_has_images(&conversation) {
                self.fail_turn_with_error(
                    &req.thread_id,
                    &turn_id,
                    format!("provider {provider} does not support image input"),
                )
                .await?;
                return Ok(());
            }

            let speed_policy_decision =
                speed_policy.decision(runtime_profile, &model, &cfg.speed_policy);
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
            if req.task_ledger_required
                && runtime_profile == RuntimeProfile::Eval
                && !conversation_has_task_ledger(&conversation)
            {
                instructions = apply_task_ledger_required(instructions);
            }
            let mut request_metadata = serde_json::json!({});
            if let Some(decision) = &speed_policy_decision {
                request_metadata["speedPolicy"] = serde_json::json!(decision);
            }
            if let Some(remaining) = deadline_remaining_seconds(turn_deadline) {
                request_metadata["deadlineRemainingSeconds"] = serde_json::json!(remaining);
            }
            let request = AgentInferenceRequest {
                model: ModelSelection {
                    provider: provider.clone(),
                    model: model.clone(),
                },
                instructions,
                conversation: conversation.clone(),
                tools: tools.clone(),
                tool_choice: tool_choice.clone(),
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
            let mut stream = if let Some(deadline) = turn_deadline {
                match tokio::time::timeout_at(deadline_instant(deadline), stream_future).await {
                    Ok(stream) => stream?,
                    Err(_) => {
                        self.fail_turn_due_to_deadline(
                            &req.thread_id,
                            &turn_id,
                            deadline,
                            &conversation,
                        )
                        .await?;
                        return Ok(());
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
                let next = if let Some(deadline) = turn_deadline {
                    match tokio::time::timeout_at(deadline_instant(deadline), stream.next()).await {
                        Ok(next) => next,
                        Err(_) => {
                            self.fail_turn_due_to_deadline(
                                &req.thread_id,
                                &turn_id,
                                deadline,
                                &conversation,
                            )
                            .await?;
                            return Ok(());
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
                        self.emit(RoderEvent::TurnFailed(TurnFailed {
                            thread_id: req.thread_id.clone(),
                            turn_id: turn_id.clone(),
                            error: err.to_string(),
                            error_kind: None,
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

                self.emit(RoderEvent::InferenceEventReceived(InferenceEventReceived {
                    thread_id: req.thread_id.clone(),
                    turn_id: turn_id.clone(),
                    event: event.clone(),
                    timestamp: OffsetDateTime::now_utc(),
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
                            &ConversationItem::Error(ErrorRecord {
                                message: failure.message.clone(),
                            }),
                        )
                        .await?;
                        self.emit(RoderEvent::TurnFailed(TurnFailed {
                            thread_id: req.thread_id.clone(),
                            turn_id: turn_id.clone(),
                            error: failure.message,
                            error_kind: None,
                            timestamp: OffsetDateTime::now_utc(),
                        }))
                        .await;
                        self.complete_team_member_turn(
                            &req.thread_id,
                            &turn_id,
                            TeamMemberStatus::Failed,
                        )
                        .await?;
                        return Ok(());
                    }
                    InferenceEvent::Completed(_)
                    | InferenceEvent::Usage(_)
                    | InferenceEvent::Compaction(_)
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
                        let item = ConversationItem::AssistantMessage(message);
                        self.persist_turn_item(&req.thread_id, &turn_id, &item)
                            .await?;
                        conversation.push(item);
                    }
                    if !assistant_text.is_empty() {
                        let assistant = ConversationItem::AssistantMessage(AssistantMessage {
                            text: assistant_text,
                            phase: Some(FINAL_ANSWER_PHASE.to_string()),
                        });
                        self.persist_turn_item(&req.thread_id, &turn_id, &assistant)
                            .await?;
                        conversation.push(assistant);
                    }
                    if let Some(metadata) = provider_metadata {
                        let item = ConversationItem::ProviderMetadata(metadata);
                        self.persist_turn_item(&req.thread_id, &turn_id, &item)
                            .await?;
                        conversation.push(item);
                    }
                    self.append_steers(&req, &turn_id, &mut conversation, steers)
                        .await?;
                    continue;
                }
                if let Some(prompt) = verification_gate.blocking_prompt() {
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
                    let item = ConversationItem::UserMessage(UserMessage::text(prompt));
                    self.persist_turn_item(&req.thread_id, &turn_id, &item)
                        .await?;
                    conversation.push(item);
                    continue;
                }
                final_phase_messages = phase_messages;
                final_assistant_text = assistant_text;
                final_reasoning_text = reasoning_text;
                final_provider_metadata = provider_metadata;
                exhausted_tool_rounds = false;
                break;
            }

            for message in phase_messages {
                let item = ConversationItem::AssistantMessage(message);
                self.persist_turn_item(&req.thread_id, &turn_id, &item)
                    .await?;
                conversation.push(item);
            }
            if !assistant_text.is_empty() {
                conversation.push(ConversationItem::AssistantMessage(AssistantMessage {
                    text: assistant_text,
                    phase: Some(FINAL_ANSWER_PHASE.to_string()),
                }));
            }
            if let Some(metadata) = provider_metadata {
                let item = ConversationItem::ProviderMetadata(metadata);
                self.persist_turn_item(&req.thread_id, &turn_id, &item)
                    .await?;
                conversation.push(item);
            }
            for call in &tool_calls {
                let tool_item = ConversationItem::ToolCall(ToolCallRecord {
                    id: call.id.clone(),
                    name: call.name.clone(),
                    arguments: call.arguments.clone(),
                });
                self.persist_turn_item(&req.thread_id, &turn_id, &tool_item)
                    .await?;
                conversation.push(tool_item);
            }
            if let Some(deadline) = turn_deadline
                && deadline_expired(deadline)
            {
                self.fail_turn_due_to_deadline(&req.thread_id, &turn_id, deadline, &conversation)
                    .await?;
                return Ok(());
            }
            let results = self
                .route_tool_calls(
                    &req.thread_id,
                    &turn_id,
                    tool_calls,
                    parallel_tool_calls,
                    workspace.as_deref(),
                    turn_deadline,
                )
                .await?;
            for result in results {
                verification_gate.record_tool_result(&result);
                conversation.push(ConversationItem::ToolResult(result));
            }
            conversation = self
                .compact_conversation_if_needed(&req.thread_id, &turn_id, &model, conversation)
                .await?;
        }

        if exhausted_tool_rounds {
            let message =
                format!("tool call limit reached after {MAX_TOOL_ROUNDS_PER_TURN} rounds");
            self.persist_turn_item(
                &req.thread_id,
                &turn_id,
                &ConversationItem::Error(ErrorRecord {
                    message: message.clone(),
                }),
            )
            .await?;
            self.emit(RoderEvent::TurnFailed(TurnFailed {
                thread_id: req.thread_id.clone(),
                turn_id: turn_id.clone(),
                error: message,
                error_kind: None,
                timestamp: OffsetDateTime::now_utc(),
            }))
            .await;
            self.complete_team_member_turn(&req.thread_id, &turn_id, TeamMemberStatus::Failed)
                .await?;
            return Ok(());
        }

        if !final_reasoning_text.is_empty() {
            self.persist_turn_item(
                &req.thread_id,
                &turn_id,
                &ConversationItem::ReasoningSummary(ReasoningSummary {
                    text: final_reasoning_text,
                }),
            )
            .await?;
        }
        for message in final_phase_messages {
            self.persist_turn_item(
                &req.thread_id,
                &turn_id,
                &ConversationItem::AssistantMessage(message),
            )
            .await?;
        }
        if !final_assistant_text.is_empty() {
            self.persist_turn_item(
                &req.thread_id,
                &turn_id,
                &ConversationItem::AssistantMessage(AssistantMessage {
                    text: final_assistant_text,
                    phase: Some(FINAL_ANSWER_PHASE.to_string()),
                }),
            )
            .await?;
        }
        if let Some(metadata) = final_provider_metadata {
            self.persist_turn_item(
                &req.thread_id,
                &turn_id,
                &ConversationItem::ProviderMetadata(metadata),
            )
            .await?;
        }

        self.emit(RoderEvent::TurnCompleted(TurnCompleted {
            thread_id: req.thread_id.clone(),
            turn_id: turn_id.clone(),
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        self.complete_team_member_turn(&req.thread_id, &turn_id, TeamMemberStatus::Completed)
            .await?;
        self.persist_runner_state(&req.thread_id, runner_session.as_ref())
            .await?;
        Ok(())
    }

    async fn drain_turn_steers(&self, turn_id: &TurnId) -> Vec<UserMessage> {
        let Some(active) = self.active_turns.read().await.get(turn_id).cloned() else {
            return Vec::new();
        };
        let mut steers = active.steers.lock().await;
        std::mem::take(&mut *steers)
    }

    async fn route_tool_calls(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        calls: Vec<ToolCallCompleted>,
        parallel: bool,
        workspace: Option<&str>,
        deadline: Option<OffsetDateTime>,
    ) -> anyhow::Result<Vec<ToolResultRecord>> {
        if parallel {
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
            &ConversationItem::Error(ErrorRecord {
                message: message.clone(),
            }),
        )
        .await?;
        self.emit(RoderEvent::TurnFailed(TurnFailed {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            error: message,
            error_kind: None,
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
        conversation: &[ConversationItem],
    ) -> anyhow::Result<()> {
        let partial_result = turn_partial_result(conversation);
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
            &ConversationItem::Error(ErrorRecord {
                message: format!("{message}: {partial_result}"),
            }),
        )
        .await?;
        self.emit(RoderEvent::TurnFailed(TurnFailed {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            error: message,
            error_kind: Some("deadline_timeout".to_string()),
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
        conversation: &mut Vec<ConversationItem>,
        steers: Vec<UserMessage>,
    ) -> anyhow::Result<()> {
        for mut steer in steers {
            steer.text = steer.text.trim().to_string();
            if steer.text.is_empty() && steer.images.is_empty() {
                continue;
            }
            let item = ConversationItem::UserMessage(steer);
            self.persist_turn_item(&req.thread_id, turn_id, &item)
                .await?;
            conversation.push(item);
        }
        Ok(())
    }

    fn filtered_tool_specs(
        &self,
        cfg: &RuntimeConfig,
        model: &str,
    ) -> Vec<roder_api::tools::ToolSpec> {
        self.tool_registry
            .specs_for_edit_tool(edit_tool_for_model(cfg, model))
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
        if let (Some(store), Some(thread_id)) = (&self.session_store, envelope.thread_id.as_ref()) {
            let _ = store.append_event(thread_id, &envelope).await;
        }
        envelope
    }

    pub(crate) async fn persist_turn_item(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        item: &ConversationItem,
    ) -> anyhow::Result<()> {
        if let Some(store) = &self.session_store {
            store.append_turn_item(thread_id, turn_id, item).await?;
        }
        self.emit(RoderEvent::TurnItemAppended(TurnItemAppended {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            item_type: match item {
                ConversationItem::UserMessage(_) => "user_message",
                ConversationItem::AssistantMessage(_) => "assistant_message",
                ConversationItem::ReasoningSummary(_) => "reasoning_summary",
                ConversationItem::ToolCall(_) => "tool_call",
                ConversationItem::ToolResult(_) => "tool_result",
                ConversationItem::FileChange(_) => "file_change",
                ConversationItem::ContextCompaction(_) => "context_compaction",
                ConversationItem::Error(_) => "error",
                ConversationItem::ProviderMetadata(_) => "provider_metadata",
            }
            .to_string(),
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        Ok(())
    }
}

fn conversation_has_images(conversation: &[ConversationItem]) -> bool {
    conversation.iter().any(|item| {
        matches!(
            item,
            ConversationItem::UserMessage(message) if !message.images.is_empty()
        )
    })
}

fn conversation_has_task_ledger(conversation: &[ConversationItem]) -> bool {
    conversation.iter().any(|item| {
        matches!(
            item,
            ConversationItem::ToolResult(result)
                if result.name.as_deref() == Some("task_ledger.update") && !result.is_error
        )
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

fn turn_partial_result(conversation: &[ConversationItem]) -> String {
    let tool_results = conversation
        .iter()
        .filter(|item| matches!(item, ConversationItem::ToolResult(_)))
        .count();
    let assistant_messages = conversation
        .iter()
        .filter(|item| matches!(item, ConversationItem::AssistantMessage(_)))
        .count();
    format!(
        "partial turn state: {} conversation items, {assistant_messages} assistant messages, {tool_results} tool results",
        conversation.len()
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
        .or(Some(entry.auto_compact_token_limit))
        .filter(|threshold| *threshold > 0)
}

fn parallel_tool_calls_for_model(cfg: &RuntimeConfig, model: &str) -> bool {
    cfg.model_parallel_tool_calls
        .get(model)
        .copied()
        .unwrap_or(true)
}

fn effective_reasoning_for_model(cfg: &RuntimeConfig, model: &str) -> String {
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
        .unwrap_or(entry.default_reasoning)
        .to_string()
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
        .or_else(|| lookup_model(model).and_then(|entry| entry.edit_tool))
        .or(Some(EDIT_TOOL_EDIT))
}

pub fn validate_edit_tool(value: &str) -> anyhow::Result<()> {
    match value.trim() {
        EDIT_TOOL_PATCH | EDIT_TOOL_EDIT => Ok(()),
        _ => anyhow::bail!(
            "unsupported edit_tool {value:?}; allowed values: {EDIT_TOOL_PATCH}, {EDIT_TOOL_EDIT}"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream;
    use roder_api::catalog::{
        REASONING_HIGH, REASONING_LOW, REASONING_MEDIUM, REASONING_MINIMAL, REASONING_NONE,
    };
    use roder_api::extension::ExtensionRegistryBuilder;
    use roder_api::inference::{
        CompletionMetadata, InferenceCapabilities, InferenceEngine, InferenceEventStream,
        InferenceProviderContext, InferenceTurnContext, MessageDelta,
    };
    use roder_api::tools::{ToolContributor, ToolExecutor, ToolSpec};
    use std::sync::Mutex as StdMutex;

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
                3 if request.conversation.iter().any(|item| {
                    matches!(
                        item,
                        ConversationItem::UserMessage(message)
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
                3 if request.conversation.iter().any(|item| {
                    matches!(
                        item,
                        ConversationItem::UserMessage(message)
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
                workspace: None,
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
    async fn task_ledger_enforcement_injects_eval_reminder_before_work() {
        let captured = Arc::new(StdMutex::new(None));
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(CapturingEngine {
            request: captured.clone(),
        }));
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
                workspace: None,
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
                workspace: None,
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
                workspace: None,
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
                workspace: None,
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
                        saw_deadline = event.partial_result.contains("conversation items");
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
        let runtime = Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                policy_mode: PolicyMode::Bypass,
                ..RuntimeConfig::default()
            },
        )
        .unwrap();

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
