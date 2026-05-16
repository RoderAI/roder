use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Context;
use futures::StreamExt;
use futures::future::{AbortHandle, Abortable};
use roder_api::catalog::{EDIT_TOOL_EDIT, EDIT_TOOL_PATCH, REASONING_NONE, lookup_model};
use roder_api::conversation::{
    AssistantMessage, ConversationItem, ErrorRecord, ReasoningSummary, ToolCallRecord, UserMessage,
};
use roder_api::events::*;
use roder_api::extension::ExtensionRegistry;
use roder_api::inference::{
    AgentInferenceRequest, InferenceEngine, InferenceEvent, InferenceTurnContext,
    InstructionBundle, ModelSelection, OutputConfig, ReasoningConfig, RuntimeHints,
};
use roder_api::policy_mode::PolicyMode;
use roder_api::session::{SessionMetadata, SessionStore, ThreadSnapshot};
use roder_api::subagents::SubagentDefinition;
use roder_api::tools::{ToolChoice, ToolRegistry};
use time::{Duration, OffsetDateTime};
use tokio::sync::{Mutex, RwLock, oneshot};

use crate::bus::EventBus;
use crate::fake_provider::FakeInferenceEngine;

const MAX_TOOL_ROUNDS_PER_TURN: usize = 1024;

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub default_provider: String,
    pub default_model: String,
    pub reasoning: Option<String>,
    pub auto_compact_token_limit: Option<u32>,
    pub model_edit_tools: HashMap<String, String>,
    pub workspace: Option<String>,
    pub policy_mode: PolicyMode,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            default_provider: roder_api::catalog::PROVIDER_MOCK.to_string(),
            default_model: "mock".to_string(),
            reasoning: None,
            auto_compact_token_limit: None,
            model_edit_tools: HashMap::new(),
            workspace: None,
            policy_mode: PolicyMode::Default,
        }
    }
}

#[derive(Debug, Clone)]
pub struct StartTurnRequest {
    pub thread_id: ThreadId,
    pub message: String,
    pub provider_override: Option<String>,
    pub model_override: Option<String>,
    pub instructions: InstructionBundle,
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
    pub(crate) tx: oneshot::Sender<bool>,
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
    active_turns: RwLock<HashMap<TurnId, AbortHandle>>,
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

        let runtime = Self {
            bus,
            registry,
            config: RwLock::new(config),
            pending_plan_exit: RwLock::new(None),
            pending_tool_approvals: Mutex::new(HashMap::new()),
            active_turns: RwLock::new(HashMap::new()),
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

    pub async fn status(&self) -> RuntimeConfig {
        self.config.read().await.clone()
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
        Ok(next)
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
    ) -> anyhow::Result<RuntimeConfig> {
        self.engine_for(&provider)?;
        let mut cfg = self.config.write().await;
        cfg.default_provider = provider;
        if let Some(model) = model {
            cfg.default_model = model;
        }
        Ok(cfg.clone())
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

    pub async fn start_turn(self: &Arc<Self>, req: StartTurnRequest) -> anyhow::Result<TurnId> {
        let cfg = self.config.read().await.clone();
        let provider = req
            .provider_override
            .clone()
            .unwrap_or_else(|| cfg.default_provider.clone());
        self.engine_for(&provider)?;
        let turn_id = uuid::Uuid::new_v4().to_string();
        let (abort_handle, abort_registration) = AbortHandle::new_pair();
        self.active_turns
            .write()
            .await
            .insert(turn_id.clone(), abort_handle);
        let runtime = Arc::clone(self);
        let turn_req = req;
        let turn_id_for_task = turn_id.clone();
        tokio::spawn(async move {
            let result = Abortable::new(
                runtime.run_turn(turn_req, turn_id_for_task.clone()),
                abort_registration,
            )
            .await;
            if let Ok(Err(err)) = result {
                let thread_id = runtime_thread_id_from_error_turn(&turn_id_for_task);
                let _ = thread_id;
                // run_turn emits failures after the turn has started; this is only a last-resort guard.
                runtime.bus.emit(RoderEvent::TurnFailed(TurnFailed {
                    thread_id: "unknown".to_string(),
                    turn_id: turn_id_for_task.clone(),
                    error: err.to_string(),
                    timestamp: OffsetDateTime::now_utc(),
                }));
            }
            runtime.active_turns.write().await.remove(&turn_id_for_task);
        });
        Ok(turn_id)
    }

    pub async fn interrupt_turn(&self, thread_id: ThreadId, turn_id: TurnId) -> anyhow::Result<()> {
        if let Some(handle) = self.active_turns.write().await.remove(&turn_id) {
            handle.abort();
        }
        self.emit(RoderEvent::TurnInterrupted(TurnInterrupted {
            thread_id,
            turn_id,
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
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        self.persist_turn_item(
            &req.thread_id,
            &turn_id,
            &ConversationItem::UserMessage(UserMessage {
                text: req.message.clone(),
            }),
        )
        .await?;

        let cfg = self.config.read().await.clone();
        let provider = req
            .provider_override
            .clone()
            .unwrap_or(cfg.default_provider.clone());
        let model = req
            .model_override
            .clone()
            .unwrap_or(cfg.default_model.clone());
        let engine = self.engine_for(&provider)?;
        let tools = self.filtered_tool_specs(&cfg, &model);
        let tool_choice = if tools.is_empty() {
            ToolChoice::None
        } else {
            ToolChoice::Auto
        };
        let mut conversation = self.conversation_for_turn(&req, &turn_id, &model).await?;
        let mut final_assistant_text = String::new();
        let mut final_reasoning_text = String::new();
        let mut exhausted_tool_rounds = true;

        for _ in 0..MAX_TOOL_ROUNDS_PER_TURN {
            self.emit(RoderEvent::InferenceStarted(InferenceStarted {
                thread_id: req.thread_id.clone(),
                turn_id: turn_id.clone(),
                engine_id: engine.id(),
                timestamp: OffsetDateTime::now_utc(),
            }))
            .await;

            let request = AgentInferenceRequest {
                model: ModelSelection {
                    provider: provider.clone(),
                    model: model.clone(),
                },
                instructions: req.instructions.clone(),
                conversation: conversation.clone(),
                tools: tools.clone(),
                tool_choice: tool_choice.clone(),
                reasoning: reasoning_for_model(&cfg, &model),
                output: OutputConfig::default(),
                runtime: RuntimeHints::default(),
                metadata: serde_json::json!({}),
            };

            let ctx = InferenceTurnContext {
                thread_id: &req.thread_id,
                turn_id: &turn_id,
            };
            let mut stream = engine.stream_turn(ctx, request).await?;
            let mut assistant_text = String::new();
            let mut reasoning_text = String::new();
            let mut tool_calls = Vec::new();

            while let Some(res) = stream.next().await {
                let event = match res {
                    Ok(event) => event,
                    Err(err) => {
                        self.emit(RoderEvent::TurnFailed(TurnFailed {
                            thread_id: req.thread_id.clone(),
                            turn_id: turn_id.clone(),
                            error: err.to_string(),
                            timestamp: OffsetDateTime::now_utc(),
                        }))
                        .await;
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
                    InferenceEvent::MessageDelta(delta) => assistant_text.push_str(&delta.text),
                    InferenceEvent::ReasoningDelta(delta) => reasoning_text.push_str(&delta.text),
                    InferenceEvent::ToolCallCompleted(call) => tool_calls.push(call),
                    InferenceEvent::Failed(failure) => {
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
                            timestamp: OffsetDateTime::now_utc(),
                        }))
                        .await;
                        return Ok(());
                    }
                    InferenceEvent::Completed(_)
                    | InferenceEvent::Usage(_)
                    | InferenceEvent::ToolCallStarted(_)
                    | InferenceEvent::ToolCallDelta(_)
                    | InferenceEvent::ProviderMetadata(_) => {}
                }
            }

            if tool_calls.is_empty() {
                final_assistant_text = assistant_text;
                final_reasoning_text = reasoning_text;
                exhausted_tool_rounds = false;
                break;
            }

            if !assistant_text.is_empty() {
                conversation.push(ConversationItem::AssistantMessage(AssistantMessage {
                    text: assistant_text,
                }));
            }
            for call in tool_calls {
                let tool_item = ConversationItem::ToolCall(ToolCallRecord {
                    id: call.id.clone(),
                    name: call.name.clone(),
                    arguments: call.arguments.clone(),
                });
                self.persist_turn_item(&req.thread_id, &turn_id, &tool_item)
                    .await?;
                conversation.push(tool_item);
                let result = self.route_tool_call(&req.thread_id, &turn_id, call).await?;
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
                thread_id: req.thread_id,
                turn_id,
                error: message,
                timestamp: OffsetDateTime::now_utc(),
            }))
            .await;
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
        if !final_assistant_text.is_empty() {
            self.persist_turn_item(
                &req.thread_id,
                &turn_id,
                &ConversationItem::AssistantMessage(AssistantMessage {
                    text: final_assistant_text,
                }),
            )
            .await?;
        }

        self.emit(RoderEvent::TurnCompleted(TurnCompleted {
            thread_id: req.thread_id,
            turn_id,
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
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

    pub(crate) async fn emit(&self, event: RoderEvent) -> EventEnvelope {
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

fn reasoning_for_model(cfg: &RuntimeConfig, model: &str) -> ReasoningConfig {
    let level = cfg
        .reasoning
        .clone()
        .or_else(|| lookup_model(model).map(|entry| entry.default_reasoning.to_string()));
    match level.as_deref() {
        Some("") | None | Some(REASONING_NONE) => ReasoningConfig::default(),
        Some(level) => ReasoningConfig {
            enabled: true,
            level: Some(level.to_string()),
        },
    }
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

fn runtime_thread_id_from_error_turn(_turn_id: &str) -> ThreadId {
    "unknown".to_string()
}
