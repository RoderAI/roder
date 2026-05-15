use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, bail};
use futures::StreamExt;
use roder_api::conversation::{
    AssistantMessage, ConversationItem, ToolCallRecord, ToolResultRecord, UserMessage,
};
use roder_api::events::{ThreadId, TurnId};
use roder_api::extension::{InferenceEngineId, SubagentDispatcherId};
use roder_api::inference::{
    AgentInferenceRequest, CompletionMetadata, InferenceEngine, InferenceEvent,
    InferenceTurnContext, InstructionBundle, ModelSelection, OutputConfig, ReasoningConfig,
    RuntimeHints,
};
use roder_api::subagents::{
    SubagentDefinition, SubagentDispatcher, SubagentExitReason, SubagentRequest, SubagentResult,
};
use roder_api::tools::{ToolCall, ToolChoice, ToolExecutionContext, ToolRegistry};
use tokio::sync::Semaphore;

use crate::transcript::{BoundedTranscript, truncate_text};

tokio::task_local! {
    static DISPATCH_DEPTH: usize;
}

#[derive(Clone, Default)]
pub struct InferenceEngineRegistry {
    engines: BTreeMap<InferenceEngineId, Arc<dyn InferenceEngine>>,
    default_engine: Option<InferenceEngineId>,
}

impl InferenceEngineRegistry {
    pub fn new() -> Self {
        std::default::Default::default()
    }

    pub fn insert(&mut self, engine: Arc<dyn InferenceEngine>) {
        let id = engine.id();
        self.default_engine.get_or_insert_with(|| id.clone());
        self.engines.insert(id, engine);
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn InferenceEngine>> {
        self.engines.get(id).cloned()
    }

    pub fn default_engine(&self) -> Option<Arc<dyn InferenceEngine>> {
        self.default_engine
            .as_deref()
            .and_then(|id| self.engines.get(id))
            .cloned()
    }
}

#[derive(Debug, Clone)]
pub struct InProcessDispatcherConfig {
    pub id: SubagentDispatcherId,
    pub default_agent: String,
    pub default_provider: Option<String>,
    pub default_model: String,
    pub max_concurrent: usize,
    pub max_depth: usize,
    pub default_timeout_seconds: u64,
    pub include_child_transcript: bool,
    pub default_max_turns: u32,
    pub default_max_result_chars: usize,
}

impl Default for InProcessDispatcherConfig {
    fn default() -> Self {
        Self {
            id: "in-process-subagents".to_string(),
            default_agent: "explore".to_string(),
            default_provider: None,
            default_model: "mock".to_string(),
            max_concurrent: 2,
            max_depth: 1,
            default_timeout_seconds: 180,
            include_child_transcript: false,
            default_max_turns: 8,
            default_max_result_chars: 4000,
        }
    }
}

pub struct InProcessDispatcher {
    config: InProcessDispatcherConfig,
    definitions: BTreeMap<String, SubagentDefinition>,
    // Task 2 intentionally stays outside roder-core: engines and tools are injected here.
    // Host/runtime/app-server wiring belongs to later roadmap tasks.
    engines: InferenceEngineRegistry,
    parent_tools: ToolRegistry,
    semaphore: Arc<Semaphore>,
}

impl InProcessDispatcher {
    pub fn new(
        config: InProcessDispatcherConfig,
        definitions: Vec<SubagentDefinition>,
        engines: InferenceEngineRegistry,
        parent_tools: ToolRegistry,
    ) -> anyhow::Result<Self> {
        if config.max_concurrent == 0 {
            bail!("max_concurrent must be at least 1");
        }
        let mut mapped = BTreeMap::new();
        for definition in definitions {
            if mapped
                .insert(definition.agent_type.clone(), definition)
                .is_some()
            {
                bail!("duplicate subagent definition");
            }
        }
        Ok(Self {
            semaphore: Arc::new(Semaphore::new(config.max_concurrent)),
            config,
            definitions: mapped,
            engines,
            parent_tools,
        })
    }

    pub fn filtered_tool_registry(
        &self,
        definition: &SubagentDefinition,
        request_tools: Option<&[String]>,
    ) -> anyhow::Result<ToolRegistry> {
        let allow = if let Some(request_tools) = request_tools {
            for tool in request_tools {
                if !definition.tools.iter().any(|allowed| allowed == tool) {
                    bail!(
                        "requested tool {tool:?} is not allowed by subagent {:?}",
                        definition.agent_type
                    );
                }
            }
            request_tools.to_vec()
        } else {
            definition.tools.clone()
        };

        let mut child = ToolRegistry::default();
        for name in allow {
            let tool = self
                .parent_tools
                .get(&name)
                .with_context(|| format!("subagent tool {name:?} is not registered"))?;
            child.register(tool)?;
        }
        Ok(child)
    }

    pub async fn dispatch_at_depth(
        &self,
        depth: usize,
        parent_thread_id: ThreadId,
        parent_turn_id: TurnId,
        request: SubagentRequest,
    ) -> anyhow::Result<SubagentResult> {
        if depth >= self.config.max_depth {
            bail!(
                "subagent max_depth {} exceeded at depth {}",
                self.config.max_depth,
                depth
            );
        }

        let permit = self
            .semaphore
            .clone()
            .try_acquire_owned()
            .context("subagent max_concurrent limit reached")?;
        let timeout = request
            .timeout_seconds
            .unwrap_or(self.config.default_timeout_seconds);
        let run = DISPATCH_DEPTH.scope(depth + 1, async {
            self.run_child_loop(parent_thread_id, parent_turn_id, request)
                .await
        });
        let result = tokio::time::timeout(Duration::from_secs(timeout), run).await;
        drop(permit);
        match result {
            Ok(result) => result,
            Err(_) => {
                let thread_id = uuid::Uuid::new_v4().to_string();
                let turn_id = uuid::Uuid::new_v4().to_string();
                Ok(SubagentResult {
                    thread_id,
                    turn_id,
                    agent_type: "unknown".to_string(),
                    model: None,
                    final_message: "subagent timed out".to_string(),
                    usage: None,
                    exit_reason: SubagentExitReason::Timeout,
                    transcript: None,
                    metadata: serde_json::json!({ "error": { "kind": "timeout" } }),
                })
            }
        }
    }

    async fn run_child_loop(
        &self,
        parent_thread_id: ThreadId,
        parent_turn_id: TurnId,
        request: SubagentRequest,
    ) -> anyhow::Result<SubagentResult> {
        let agent_type = request
            .subagent_type
            .clone()
            .unwrap_or_else(|| self.config.default_agent.clone());
        let definition = self
            .definitions
            .get(&agent_type)
            .with_context(|| format!("unknown subagent type {agent_type:?}"))?;
        let tools = self.filtered_tool_registry(definition, request.tools.as_deref())?;
        let model = request
            .model
            .clone()
            .or_else(|| definition.model.clone())
            .unwrap_or_else(|| self.config.default_model.clone());
        let engine = self
            .config
            .default_provider
            .as_deref()
            .and_then(|provider| self.engines.get(provider))
            .or_else(|| self.engines.default_engine())
            .with_context(|| {
                format!("no inference engine registered for subagent {agent_type:?}")
            })?;
        let provider = engine.id();
        let thread_id = uuid::Uuid::new_v4().to_string();
        let turn_id = uuid::Uuid::new_v4().to_string();
        let max_result_chars = definition
            .max_result_chars
            .unwrap_or(self.config.default_max_result_chars);
        let mut transcript = BoundedTranscript::new(max_result_chars);
        transcript.push_text("user", request.prompt.clone());

        let mut conversation = vec![ConversationItem::UserMessage(UserMessage {
            text: request.prompt,
        })];
        let mut usage = None;
        let mut final_message = String::new();
        let mut exit_reason = SubagentExitReason::MaxTurns;
        let max_turns = definition
            .max_turns
            .unwrap_or(self.config.default_max_turns);

        for _ in 0..max_turns {
            let inference_request = AgentInferenceRequest {
                model: ModelSelection {
                    provider: provider.clone(),
                    model: model.clone(),
                },
                instructions: InstructionBundle {
                    system: definition.system_prompt.clone(),
                    developer: None,
                },
                conversation: conversation.clone(),
                tools: tools.specs(),
                tool_choice: if tools.is_empty() {
                    ToolChoice::None
                } else {
                    ToolChoice::Auto
                },
                reasoning: ReasoningConfig::default(),
                output: OutputConfig::default(),
                runtime: RuntimeHints {
                    trace_id: Some(format!("{parent_thread_id}:{parent_turn_id}")),
                    prompt_cache_key: None,
                },
                metadata: serde_json::json!({
                    "subagent": {
                        "agent_type": agent_type,
                        "parent_thread_id": parent_thread_id,
                        "parent_turn_id": parent_turn_id,
                    }
                }),
            };
            let ctx = InferenceTurnContext {
                thread_id: &thread_id,
                turn_id: &turn_id,
            };
            let mut stream = engine.stream_turn(ctx, inference_request).await?;
            let mut assistant_text = String::new();
            let mut tool_calls = Vec::new();

            while let Some(event) = stream.next().await.transpose()? {
                match event {
                    InferenceEvent::MessageDelta(delta) => assistant_text.push_str(&delta.text),
                    InferenceEvent::ToolCallCompleted(call) => {
                        transcript.push_tool_call(call.id.clone(), call.name.clone());
                        tool_calls.push(call);
                    }
                    InferenceEvent::Usage(next_usage) => usage = Some(next_usage),
                    InferenceEvent::Completed(CompletionMetadata { stop_reason, .. }) => {
                        if stop_reason.as_deref() != Some("tool_calls") && tool_calls.is_empty() {
                            exit_reason = SubagentExitReason::Completed;
                        }
                    }
                    InferenceEvent::Failed(failure) => {
                        bail!("subagent inference failed: {}", failure.message);
                    }
                    InferenceEvent::ReasoningDelta(_)
                    | InferenceEvent::ToolCallStarted(_)
                    | InferenceEvent::ToolCallDelta(_)
                    | InferenceEvent::ProviderMetadata(_) => {}
                }
            }

            if tool_calls.is_empty() {
                final_message = truncate_text(&assistant_text, max_result_chars);
                transcript.push_text("assistant", final_message.clone());
                if exit_reason != SubagentExitReason::Completed {
                    exit_reason = SubagentExitReason::Completed;
                }
                break;
            }

            if !assistant_text.is_empty() {
                transcript.push_text("assistant", assistant_text.clone());
                conversation.push(ConversationItem::AssistantMessage(AssistantMessage {
                    text: assistant_text,
                }));
            }
            for call in tool_calls {
                conversation.push(ConversationItem::ToolCall(ToolCallRecord {
                    id: call.id.clone(),
                    name: call.name.clone(),
                    arguments: call.arguments.clone(),
                }));
                let result = execute_tool(&tools, &thread_id, &turn_id, call).await?;
                transcript.push_text("tool", result.result.clone());
                conversation.push(ConversationItem::ToolResult(result));
            }
        }

        if final_message.is_empty() && exit_reason == SubagentExitReason::MaxTurns {
            final_message = "subagent reached max_turns without a final message".to_string();
        }

        Ok(SubagentResult {
            thread_id,
            turn_id,
            agent_type,
            model: Some(model),
            final_message,
            usage,
            exit_reason,
            transcript: self
                .config
                .include_child_transcript
                .then(|| transcript.to_json()),
            metadata: serde_json::json!({}),
        })
    }
}

#[async_trait::async_trait]
impl SubagentDispatcher for InProcessDispatcher {
    fn id(&self) -> SubagentDispatcherId {
        self.config.id.clone()
    }

    fn definitions(&self) -> Vec<SubagentDefinition> {
        self.definitions.values().cloned().collect()
    }

    async fn dispatch(
        &self,
        parent_thread_id: ThreadId,
        parent_turn_id: TurnId,
        request: SubagentRequest,
    ) -> anyhow::Result<SubagentResult> {
        let depth = DISPATCH_DEPTH.try_with(|depth| *depth).unwrap_or(0);
        self.dispatch_at_depth(depth, parent_thread_id, parent_turn_id, request)
            .await
    }
}

async fn execute_tool(
    tools: &ToolRegistry,
    thread_id: &ThreadId,
    turn_id: &TurnId,
    call: roder_api::inference::ToolCallCompleted,
) -> anyhow::Result<ToolResultRecord> {
    let executor = tools
        .get(&call.name)
        .with_context(|| format!("subagent attempted unavailable tool {:?}", call.name))?;
    let arguments = serde_json::from_str(&call.arguments).with_context(|| {
        format!(
            "subagent tool {:?} emitted invalid JSON arguments",
            call.name
        )
    })?;
    let result = executor
        .execute(
            ToolExecutionContext {
                thread_id: thread_id.clone(),
                turn_id: turn_id.clone(),
            },
            ToolCall {
                id: call.id,
                name: call.name,
                raw_arguments: call.arguments,
                arguments,
                thread_id: thread_id.clone(),
                turn_id: turn_id.clone(),
            },
        )
        .await?;
    Ok(ToolResultRecord {
        id: result.id,
        name: Some(result.name),
        result: result.text,
        is_error: result.is_error,
    })
}
