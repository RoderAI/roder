use std::sync::Arc;

use futures::StreamExt;
use roder_api::context::{ContextBlockKind, ContextPlan, ContextQuery};
use roder_api::conversation::{
    AssistantMessage, ConversationItem, ErrorRecord, ReasoningSummary, ToolCallRecord,
    ToolResultRecord, UserMessage,
};
use roder_api::events::*;
use roder_api::extension::ExtensionRegistry;
use roder_api::inference::{
    AgentInferenceRequest, InferenceEngine, InferenceEvent, InferenceTurnContext,
    InstructionBundle, ModelSelection, OutputConfig, ReasoningConfig, RuntimeHints,
};
use roder_api::session::{SessionMetadata, SessionStore, ThreadSnapshot};
use roder_api::tools::{ToolCall, ToolChoice, ToolExecutionContext, ToolRegistry};
use time::OffsetDateTime;
use tokio::sync::RwLock;

use crate::bus::EventBus;
use crate::fake_provider::FakeInferenceEngine;

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub default_provider: String,
    pub default_model: String,
    pub workspace: Option<String>,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            default_provider: roder_api::catalog::PROVIDER_MOCK.to_string(),
            default_model: "mock".to_string(),
            workspace: None,
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

pub struct Runtime {
    pub bus: EventBus,
    pub registry: ExtensionRegistry,
    config: RwLock<RuntimeConfig>,
    session_store: Option<Arc<dyn SessionStore>>,
    tool_registry: ToolRegistry,
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
            contributor.contribute(&mut tool_registry)?;
        }

        let runtime = Self {
            bus,
            registry,
            config: RwLock::new(config),
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
        let cfg = self.config.read().await.clone();
        let now = OffsetDateTime::now_utc();
        let metadata = SessionMetadata {
            thread_id: uuid::Uuid::new_v4().to_string(),
            title,
            workspace: cfg.workspace,
            provider: Some(cfg.default_provider),
            model: Some(cfg.default_model),
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
            .unwrap_or(cfg.default_provider);
        self.engine_for(&provider)?;
        let turn_id = uuid::Uuid::new_v4().to_string();
        let runtime = Arc::clone(self);
        let turn_req = req;
        let turn_id_for_task = turn_id.clone();
        tokio::spawn(async move {
            if let Err(err) = runtime.run_turn(turn_req, turn_id_for_task.clone()).await {
                let thread_id = runtime_thread_id_from_error_turn(&turn_id_for_task);
                let _ = thread_id;
                // run_turn emits failures after the turn has started; this is only a last-resort guard.
                runtime.bus.emit(RoderEvent::TurnFailed(TurnFailed {
                    thread_id: "unknown".to_string(),
                    turn_id: turn_id_for_task,
                    error: err.to_string(),
                    timestamp: OffsetDateTime::now_utc(),
                }));
            }
        });
        Ok(turn_id)
    }

    pub async fn interrupt_turn(&self, thread_id: ThreadId, turn_id: TurnId) -> anyhow::Result<()> {
        self.emit(RoderEvent::TurnInterrupted(TurnInterrupted {
            thread_id,
            turn_id,
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        Ok(())
    }

    pub fn tool_specs(&self) -> Vec<roder_api::tools::ToolSpec> {
        self.tool_registry.specs()
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

        let context_plan = self.assemble_context(&req, &turn_id).await?;
        let cfg = self.config.read().await.clone();
        let provider = req
            .provider_override
            .clone()
            .unwrap_or(cfg.default_provider);
        let model = req.model_override.clone().unwrap_or(cfg.default_model);
        let engine = self.engine_for(&provider)?;

        self.emit(RoderEvent::InferenceStarted(InferenceStarted {
            thread_id: req.thread_id.clone(),
            turn_id: turn_id.clone(),
            engine_id: engine.id(),
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;

        let mut conversation = Vec::new();
        for block in context_plan.blocks {
            if matches!(
                block.kind,
                ContextBlockKind::Instruction
                    | ContextBlockKind::Memory
                    | ContextBlockKind::RepositoryFact
                    | ContextBlockKind::PriorSummary
            ) {
                conversation.push(ConversationItem::UserMessage(UserMessage {
                    text: block.text,
                }));
            }
        }
        conversation.push(ConversationItem::UserMessage(UserMessage {
            text: req.message,
        }));

        let request = AgentInferenceRequest {
            model: ModelSelection {
                provider: provider.clone(),
                model,
            },
            instructions: req.instructions,
            conversation,
            tools: self.tool_registry.specs(),
            tool_choice: if self.tool_registry.is_empty() {
                ToolChoice::None
            } else {
                ToolChoice::Auto
            },
            reasoning: ReasoningConfig::default(),
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
                InferenceEvent::ToolCallCompleted(call) => {
                    self.persist_turn_item(
                        &req.thread_id,
                        &turn_id,
                        &ConversationItem::ToolCall(ToolCallRecord {
                            id: call.id.clone(),
                            name: call.name.clone(),
                            arguments: call.arguments.clone(),
                        }),
                    )
                    .await?;
                    self.route_tool_call(&req.thread_id, &turn_id, call).await?;
                }
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

        if !reasoning_text.is_empty() {
            self.persist_turn_item(
                &req.thread_id,
                &turn_id,
                &ConversationItem::ReasoningSummary(ReasoningSummary {
                    text: reasoning_text,
                }),
            )
            .await?;
        }
        if !assistant_text.is_empty() {
            self.persist_turn_item(
                &req.thread_id,
                &turn_id,
                &ConversationItem::AssistantMessage(AssistantMessage {
                    text: assistant_text,
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

    async fn assemble_context(
        &self,
        req: &StartTurnRequest,
        turn_id: &TurnId,
    ) -> anyhow::Result<ContextPlan> {
        self.emit(RoderEvent::ContextAssemblyStarted(ContextAssemblyStarted {
            thread_id: req.thread_id.clone(),
            turn_id: turn_id.clone(),
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;

        let query = ContextQuery {
            thread_id: req.thread_id.clone(),
            turn_id: turn_id.clone(),
            prompt: req.message.clone(),
            token_budget: None,
        };
        let mut blocks = Vec::new();
        for provider in &self.registry.context_providers {
            for block in provider.blocks(&query).await? {
                self.emit(RoderEvent::ContextBlockAdded(ContextBlockAdded {
                    thread_id: req.thread_id.clone(),
                    turn_id: turn_id.clone(),
                    block_type: format!("{:?}", block.kind),
                    timestamp: OffsetDateTime::now_utc(),
                }))
                .await;
                blocks.push(block);
            }
        }
        let plan = if let Some(planner) = self.registry.context_planners.first() {
            planner.plan(&query, blocks).await?
        } else {
            ContextPlan { blocks }
        };
        self.emit(RoderEvent::ContextAssemblyCompleted(
            ContextAssemblyCompleted {
                thread_id: req.thread_id.clone(),
                turn_id: turn_id.clone(),
                timestamp: OffsetDateTime::now_utc(),
            },
        ))
        .await;
        Ok(plan)
    }

    async fn route_tool_call(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        call: roder_api::inference::ToolCallCompleted,
    ) -> anyhow::Result<()> {
        self.emit(RoderEvent::ToolCallRequested(ToolCallRequested {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            tool_id: call.id.clone(),
            tool_name: call.name.clone(),
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        let Some(executor) = self.tool_registry.get(&call.name) else {
            let item = ConversationItem::ToolResult(ToolResultRecord {
                id: call.id,
                name: Some(call.name),
                result: "tool not found".to_string(),
                is_error: true,
            });
            self.persist_turn_item(thread_id, turn_id, &item).await?;
            return Ok(());
        };
        self.emit(RoderEvent::ToolCallStarted(ToolCallStarted {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            tool_id: call.id.clone(),
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        let parsed_args = serde_json::from_str(&call.arguments)
            .unwrap_or_else(|_| serde_json::json!({ "raw": call.arguments }));
        let result = executor
            .execute(
                ToolExecutionContext {
                    thread_id: thread_id.clone(),
                    turn_id: turn_id.clone(),
                },
                ToolCall {
                    id: call.id.clone(),
                    name: call.name.clone(),
                    arguments: parsed_args,
                    raw_arguments: call.arguments,
                    thread_id: thread_id.clone(),
                    turn_id: turn_id.clone(),
                },
            )
            .await?;
        self.persist_turn_item(
            thread_id,
            turn_id,
            &ConversationItem::ToolResult(ToolResultRecord {
                id: result.id.clone(),
                name: Some(result.name.clone()),
                result: result.text,
                is_error: result.is_error,
            }),
        )
        .await?;
        self.emit(RoderEvent::ToolCallCompleted(ToolCallCompleted {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            tool_id: result.id,
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        Ok(())
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

    async fn emit(&self, event: RoderEvent) -> EventEnvelope {
        let envelope = self.bus.emit(event);
        if let (Some(store), Some(thread_id)) = (&self.session_store, envelope.thread_id.as_ref()) {
            let _ = store.append_event(thread_id, &envelope).await;
        }
        envelope
    }

    async fn persist_turn_item(
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

fn runtime_thread_id_from_error_turn(_turn_id: &str) -> ThreadId {
    "unknown".to_string()
}
