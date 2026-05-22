use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use futures::StreamExt;
use roder_api::conversation::{
    AssistantMessage, ConversationItem, ToolCallRecord, ToolResultRecord, UserMessage,
    tool_display_payload,
};
use roder_api::events::{ThreadId, TurnId};
use roder_api::extension::{InferenceEngineId, SubagentDispatcherId};
use roder_api::inference::{
    AgentInferenceRequest, CompletionMetadata, HostedWebSearchConfig, InferenceEngine,
    InferenceEvent, InferenceTurnContext, InstructionBundle, ModelSelection, OutputConfig,
    ReasoningConfig, RuntimeHints,
};
use roder_api::policy_mode::PolicyMode;
use roder_api::subagents::{
    SUBAGENT_SUMMARY_CONTRACT, SubagentDefinition, SubagentDispatcher, SubagentExitReason,
    SubagentLane, SubagentPermissionMode, SubagentRequest, SubagentResult,
};
use roder_api::tools::{ToolCall, ToolChoice, ToolExecutionContext, ToolRegistry};
use roder_api::trace::{
    PagedTraceText, ParentTurnRef, SubagentTraceDelta, SubagentTraceItem, SubagentTraceSink,
    SubagentTraceStatus,
};
use tokio::sync::Semaphore;

use crate::trace::{
    TRACE_TEXT_MAX_CHARS, TraceIds, TraceSummaryArgs, emit_trace_completed, emit_trace_created,
    emit_trace_delta, emit_trace_failed, emit_trace_status_changed, trace_summary,
};
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
    // Task 2 intentionally stays outside the core crate: engines and tools are injected here.
    // Host/runtime/app-server wiring belongs to later roadmap tasks.
    engines: InferenceEngineRegistry,
    parent_tools: ToolRegistry,
    semaphore: Arc<Semaphore>,
    active_lanes: Arc<StdMutex<BTreeMap<String, usize>>>,
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
            active_lanes: Arc::new(StdMutex::new(BTreeMap::new())),
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

    fn filtered_tool_registry_for_request(
        &self,
        definition: &SubagentDefinition,
        request: &SubagentRequest,
    ) -> anyhow::Result<ToolRegistry> {
        let mut allow = definition.tools.clone();
        apply_explicit_tool_restriction(definition, &mut allow, request.tools.as_deref())?;
        apply_explicit_tool_restriction(definition, &mut allow, request.allowed_tools.as_deref())?;
        if let Some(lane) = request.lane {
            apply_lane_tool_restriction(&mut allow, lane);
        }

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
        self.dispatch_at_depth_with_trace(depth, parent_thread_id, parent_turn_id, request, None)
            .await
    }

    async fn dispatch_at_depth_with_trace(
        &self,
        depth: usize,
        parent_thread_id: ThreadId,
        parent_turn_id: TurnId,
        request: SubagentRequest,
        trace_sink: Option<Arc<dyn SubagentTraceSink>>,
    ) -> anyhow::Result<SubagentResult> {
        if depth >= self.config.max_depth {
            bail!(
                "subagent max_depth {} exceeded at depth {}",
                self.config.max_depth,
                depth
            );
        }

        let trace_ids = TraceIds {
            trace_id: uuid::Uuid::new_v4().to_string(),
            child_thread_id: uuid::Uuid::new_v4().to_string(),
            child_turn_id: uuid::Uuid::new_v4().to_string(),
        };
        let parent = ParentTurnRef {
            thread_id: parent_thread_id.clone(),
            turn_id: parent_turn_id.clone(),
        };
        let lane = effective_lane(&request);
        let started_at = Instant::now();
        emit_trace_created(
            trace_sink.as_deref(),
            trace_summary(TraceSummaryArgs {
                trace_ids: &trace_ids,
                parent: &parent,
                request: &request,
                default_role: self.config.default_agent.as_str(),
                model: request.model.clone(),
                status: SubagentTraceStatus::Queued,
                started_at,
                usage: None,
                latest_activity: Some("queued".to_string()),
                error_summary: None,
                exit_reason: None,
            }),
        )
        .await;

        let permit = self
            .semaphore
            .clone()
            .try_acquire_owned()
            .context("subagent max_concurrent limit reached");
        let permit = match permit {
            Ok(permit) => permit,
            Err(err) => {
                let summary = trace_summary(TraceSummaryArgs {
                    trace_ids: &trace_ids,
                    parent: &parent,
                    request: &request,
                    default_role: self.config.default_agent.as_str(),
                    model: request.model.clone(),
                    status: SubagentTraceStatus::Failed,
                    started_at,
                    usage: None,
                    latest_activity: Some("concurrency limit reached".to_string()),
                    error_summary: Some(err.to_string()),
                    exit_reason: Some(SubagentExitReason::Failed),
                });
                emit_trace_failed(trace_sink.as_deref(), summary, err.to_string()).await;
                return Err(err);
            }
        };
        let lane_guard = match ActiveLaneGuard::try_acquire(
            self.active_lanes.clone(),
            lane,
            effective_max_concurrent(&request, lane),
        ) {
            Ok(guard) => guard,
            Err(err) => {
                let summary = trace_summary(TraceSummaryArgs {
                    trace_ids: &trace_ids,
                    parent: &parent,
                    request: &request,
                    default_role: self.config.default_agent.as_str(),
                    model: request.model.clone(),
                    status: SubagentTraceStatus::Failed,
                    started_at,
                    usage: None,
                    latest_activity: Some(format!(
                        "{} lane concurrency limit reached",
                        lane.as_str()
                    )),
                    error_summary: Some(err.to_string()),
                    exit_reason: Some(SubagentExitReason::Failed),
                });
                emit_trace_failed(trace_sink.as_deref(), summary, err.to_string()).await;
                return Err(err);
            }
        };
        let timeout = request
            .timeout_seconds
            .or_else(|| request.lane.map(|lane| lane.preset().timeout_seconds))
            .unwrap_or(self.config.default_timeout_seconds);
        let timeout_summary_request = request.clone();
        let timeout_trace_ids = trace_ids.clone();
        let timeout_parent = parent.clone();
        let timeout_model = request.model.clone();
        let run = DISPATCH_DEPTH.scope(depth + 1, async {
            self.run_child_loop(
                parent_thread_id,
                parent_turn_id,
                request,
                trace_sink.clone(),
                trace_ids,
                started_at,
            )
            .await
        });
        let result = tokio::time::timeout(Duration::from_secs(timeout), run).await;
        drop(lane_guard);
        drop(permit);
        match result {
            Ok(result) => result,
            Err(_) => {
                let summary = trace_summary(TraceSummaryArgs {
                    trace_ids: &timeout_trace_ids,
                    parent: &timeout_parent,
                    request: &timeout_summary_request,
                    default_role: self.config.default_agent.as_str(),
                    model: timeout_model.clone(),
                    status: SubagentTraceStatus::Failed,
                    started_at,
                    usage: None,
                    latest_activity: Some("timed out".to_string()),
                    error_summary: Some("subagent timed out".to_string()),
                    exit_reason: Some(SubagentExitReason::Timeout),
                });
                emit_trace_failed(
                    trace_sink.as_deref(),
                    summary,
                    "subagent timed out".to_string(),
                )
                .await;
                Ok(SubagentResult {
                    thread_id: timeout_trace_ids.child_thread_id,
                    turn_id: timeout_trace_ids.child_turn_id,
                    agent_type: "unknown".to_string(),
                    model: timeout_model,
                    final_message: "subagent timed out".to_string(),
                    usage: None,
                    exit_reason: SubagentExitReason::Timeout,
                    transcript: None,
                    metadata: serde_json::json!({
                        "lane": timeout_summary_request.lane.unwrap_or(SubagentLane::Scout).as_str(),
                        "exit_reason": "timeout",
                        "error": { "kind": "timeout" }
                    }),
                })
            }
        }
    }

    async fn run_child_loop(
        &self,
        parent_thread_id: ThreadId,
        parent_turn_id: TurnId,
        request: SubagentRequest,
        trace_sink: Option<Arc<dyn SubagentTraceSink>>,
        trace_ids: TraceIds,
        started_at: Instant,
    ) -> anyhow::Result<SubagentResult> {
        let agent_type = request
            .subagent_type
            .clone()
            .unwrap_or_else(|| self.config.default_agent.clone());
        let definition = self
            .definitions
            .get(&agent_type)
            .with_context(|| format!("unknown subagent type {agent_type:?}"))?;
        let tools = self.filtered_tool_registry_for_request(definition, &request)?;
        let lane = effective_lane_for_definition(&request, definition);
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
        let thread_id = trace_ids.child_thread_id.clone();
        let turn_id = trace_ids.child_turn_id.clone();
        let parent = ParentTurnRef {
            thread_id: parent_thread_id.clone(),
            turn_id: parent_turn_id.clone(),
        };
        let trace_model = Some(model.clone());
        emit_trace_status_changed(
            trace_sink.as_deref(),
            trace_ids.trace_id.clone(),
            parent.clone(),
            SubagentTraceStatus::Running,
            Some("running".to_string()),
        )
        .await;
        let max_result_chars = definition
            .max_result_chars
            .unwrap_or(self.config.default_max_result_chars);
        let mut transcript = BoundedTranscript::new(max_result_chars);
        transcript.push_text("user", request.prompt.clone());

        let mut conversation = vec![ConversationItem::UserMessage(UserMessage::text(
            request.prompt.clone(),
        ))];
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
                    developer: Some(subagent_developer_instructions(lane, &request)),
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
                    auto_compact_token_limit: None,
                    // Give subagents live (codex-native) web browsing so the swarm
                    // can actually research in parallel, not just reason.
                    hosted_web_search: HostedWebSearchConfig::live(),
                    ..RuntimeHints::default()
                },
                metadata: serde_json::json!({
                    "subagent": {
                        "agent_type": agent_type,
                        "lane": lane.as_str(),
                        "summary_contract": SUBAGENT_SUMMARY_CONTRACT,
                        "parent_deadline_seconds": request.parent_deadline_seconds,
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
                    InferenceEvent::MessageDelta(delta) => {
                        assistant_text.push_str(&delta.text);
                        emit_trace_delta(
                            trace_sink.as_deref(),
                            SubagentTraceDelta {
                                trace_id: trace_ids.trace_id.clone(),
                                parent: parent.clone(),
                                item: SubagentTraceItem::Message {
                                    role: "assistant".to_string(),
                                    content: PagedTraceText::capped(
                                        delta.text,
                                        TRACE_TEXT_MAX_CHARS,
                                    ),
                                },
                            },
                        )
                        .await;
                    }
                    InferenceEvent::ToolCallCompleted(call) => {
                        transcript.push_tool_call(call.id.clone(), call.name.clone());
                        let input = serde_json::from_str(&call.arguments).unwrap_or_else(
                            |_| serde_json::json!({ "raw": call.arguments.clone() }),
                        );
                        emit_trace_delta(
                            trace_sink.as_deref(),
                            SubagentTraceDelta {
                                trace_id: trace_ids.trace_id.clone(),
                                parent: parent.clone(),
                                item: SubagentTraceItem::ToolCall {
                                    tool_id: call.id.clone(),
                                    tool_name: call.name.clone(),
                                    input,
                                },
                            },
                        )
                        .await;
                        tool_calls.push(call);
                    }
                    InferenceEvent::Usage(next_usage) => usage = Some(next_usage),
                    InferenceEvent::Completed(CompletionMetadata { stop_reason, .. }) => {
                        if stop_reason.as_deref() != Some("tool_calls") && tool_calls.is_empty() {
                            exit_reason = SubagentExitReason::Completed;
                        }
                    }
                    InferenceEvent::Failed(failure) => {
                        let summary = trace_summary(TraceSummaryArgs {
                            trace_ids: &trace_ids,
                            parent: &parent,
                            request: &request,
                            default_role: &agent_type,
                            model: trace_model.clone(),
                            status: SubagentTraceStatus::Failed,
                            started_at,
                            usage: usage.clone(),
                            latest_activity: Some("inference failed".to_string()),
                            error_summary: Some(failure.message.clone()),
                            exit_reason: Some(SubagentExitReason::Failed),
                        });
                        emit_trace_failed(trace_sink.as_deref(), summary, failure.message.clone())
                            .await;
                        bail!("subagent inference failed: {}", failure.message);
                    }
                    InferenceEvent::ReasoningDelta(delta) => {
                        emit_trace_delta(
                            trace_sink.as_deref(),
                            SubagentTraceDelta {
                                trace_id: trace_ids.trace_id.clone(),
                                parent: parent.clone(),
                                item: SubagentTraceItem::Reasoning {
                                    content: PagedTraceText::capped(
                                        delta.text,
                                        TRACE_TEXT_MAX_CHARS,
                                    ),
                                },
                            },
                        )
                        .await;
                    }
                    InferenceEvent::ToolCallStarted(started) => {
                        emit_trace_delta(
                            trace_sink.as_deref(),
                            SubagentTraceDelta {
                                trace_id: trace_ids.trace_id.clone(),
                                parent: parent.clone(),
                                item: SubagentTraceItem::ToolCall {
                                    tool_id: started.id,
                                    tool_name: started.name,
                                    input: serde_json::json!({}),
                                },
                            },
                        )
                        .await;
                    }
                    InferenceEvent::ToolCallDelta(_)
                    | InferenceEvent::HostedToolCallStarted(_)
                    | InferenceEvent::HostedToolCallCompleted(_)
                    | InferenceEvent::Compaction(_)
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
                    phase: None,
                }));
            }
            for call in tool_calls {
                conversation.push(ConversationItem::ToolCall(ToolCallRecord {
                    id: call.id.clone(),
                    name: call.name.clone(),
                    arguments: call.arguments.clone(),
                }));
                let tool_id = call.id.clone();
                let result = execute_tool(&tools, &thread_id, &turn_id, call).await?;
                transcript.push_text("tool", result.result.clone());
                emit_trace_delta(
                    trace_sink.as_deref(),
                    SubagentTraceDelta {
                        trace_id: trace_ids.trace_id.clone(),
                        parent: parent.clone(),
                        item: SubagentTraceItem::ToolResult {
                            tool_id,
                            is_error: result.is_error,
                            output: PagedTraceText::capped(
                                result.result.clone(),
                                TRACE_TEXT_MAX_CHARS,
                            ),
                        },
                    },
                )
                .await;
                conversation.push(ConversationItem::ToolResult(result));
            }
        }

        if final_message.is_empty() && exit_reason == SubagentExitReason::MaxTurns {
            final_message = "subagent reached max_turns without a final message".to_string();
        }

        let final_status = match exit_reason {
            SubagentExitReason::Completed => SubagentTraceStatus::Completed,
            SubagentExitReason::Cancelled => SubagentTraceStatus::Cancelled,
            SubagentExitReason::Failed
            | SubagentExitReason::MaxTurns
            | SubagentExitReason::Timeout => SubagentTraceStatus::Failed,
        };
        let final_summary = trace_summary(TraceSummaryArgs {
            trace_ids: &trace_ids,
            parent: &parent,
            request: &request,
            default_role: &agent_type,
            model: trace_model.clone(),
            status: final_status.clone(),
            started_at,
            usage: usage.clone(),
            latest_activity: Some(match exit_reason {
                SubagentExitReason::Completed => "completed".to_string(),
                SubagentExitReason::MaxTurns => "reached max_turns".to_string(),
                SubagentExitReason::Timeout => "timed out".to_string(),
                SubagentExitReason::Cancelled => "cancelled".to_string(),
                SubagentExitReason::Failed => "failed".to_string(),
            }),
            error_summary: (final_status == SubagentTraceStatus::Failed)
                .then(|| final_message.clone()),
            exit_reason: Some(exit_reason.clone()),
        });
        if final_status == SubagentTraceStatus::Completed {
            emit_trace_completed(trace_sink.as_deref(), final_summary).await;
        } else {
            emit_trace_failed(trace_sink.as_deref(), final_summary, final_message.clone()).await;
        }

        Ok(SubagentResult {
            thread_id,
            turn_id,
            agent_type,
            model: Some(model),
            final_message,
            usage,
            exit_reason: exit_reason.clone(),
            transcript: self
                .config
                .include_child_transcript
                .then(|| transcript.to_json()),
            metadata: serde_json::json!({
                "lane": lane.as_str(),
                "exit_reason": exit_reason,
                "summary_contract": SUBAGENT_SUMMARY_CONTRACT,
            }),
        })
    }
}

struct ActiveLaneGuard {
    active_lanes: Arc<StdMutex<BTreeMap<String, usize>>>,
    key: String,
}

impl ActiveLaneGuard {
    fn try_acquire(
        active_lanes: Arc<StdMutex<BTreeMap<String, usize>>>,
        lane: SubagentLane,
        max_concurrent: usize,
    ) -> anyhow::Result<Self> {
        if max_concurrent == 0 {
            bail!(
                "subagent {} lane max_concurrent must be at least 1",
                lane.as_str()
            );
        }
        let key = lane.as_str().to_string();
        {
            let mut active = active_lanes.lock().unwrap();
            let count = active.entry(key.clone()).or_insert(0);
            if *count >= max_concurrent {
                bail!(
                    "subagent {} lane max_concurrent limit {max_concurrent} reached",
                    lane.as_str()
                );
            }
            *count += 1;
        }
        Ok(Self { active_lanes, key })
    }
}

impl Drop for ActiveLaneGuard {
    fn drop(&mut self) {
        let mut active = self.active_lanes.lock().unwrap();
        if let Some(count) = active.get_mut(&self.key) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                active.remove(&self.key);
            }
        }
    }
}

fn effective_lane(request: &SubagentRequest) -> SubagentLane {
    request.lane.unwrap_or(SubagentLane::Scout)
}

fn effective_lane_for_definition(
    request: &SubagentRequest,
    definition: &SubagentDefinition,
) -> SubagentLane {
    request.lane.unwrap_or(match definition.permission_mode {
        SubagentPermissionMode::ReadOnly => SubagentLane::Scout,
        SubagentPermissionMode::AutoEdit => SubagentLane::Editor,
        SubagentPermissionMode::Default => SubagentLane::Reviewer,
    })
}

fn effective_max_concurrent(request: &SubagentRequest, lane: SubagentLane) -> usize {
    request
        .max_concurrent
        .unwrap_or_else(|| lane.preset().max_concurrent)
}

fn apply_explicit_tool_restriction(
    definition: &SubagentDefinition,
    allow: &mut Vec<String>,
    restriction: Option<&[String]>,
) -> anyhow::Result<()> {
    let Some(restriction) = restriction else {
        return Ok(());
    };
    for tool in restriction {
        if !definition.tools.iter().any(|allowed| allowed == tool) {
            bail!(
                "requested tool {tool:?} is not allowed by subagent {:?}",
                definition.agent_type
            );
        }
    }
    allow.retain(|tool| restriction.iter().any(|requested| requested == tool));
    Ok(())
}

fn apply_lane_tool_restriction(allow: &mut Vec<String>, lane: SubagentLane) {
    let preset = lane.preset();
    allow.retain(|tool| preset.allowed_tools.iter().any(|allowed| allowed == tool));
}

fn subagent_developer_instructions(lane: SubagentLane, request: &SubagentRequest) -> String {
    let preset = lane.preset();
    let mut instructions = format!(
        "Subagent lane: {}. Lane purpose: {} {}",
        lane.as_str(),
        preset.description,
        SUBAGENT_SUMMARY_CONTRACT
    );
    if let Some(parent_deadline_seconds) = request.parent_deadline_seconds {
        instructions.push_str(&format!(
            " Parent deadline budget: {parent_deadline_seconds} seconds."
        ));
    }
    instructions
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

    async fn dispatch_traced(
        &self,
        parent_thread_id: ThreadId,
        parent_turn_id: TurnId,
        request: SubagentRequest,
        trace_sink: Option<Arc<dyn SubagentTraceSink>>,
    ) -> anyhow::Result<SubagentResult> {
        let depth = DISPATCH_DEPTH.try_with(|depth| *depth).unwrap_or(0);
        self.dispatch_at_depth_with_trace(
            depth,
            parent_thread_id,
            parent_turn_id,
            request,
            trace_sink,
        )
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
            ToolExecutionContext::new(thread_id.clone(), turn_id.clone(), PolicyMode::Default),
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
        display_payload: tool_display_payload(None, None, Some(&result.data)),
        is_error: result.is_error,
    })
}
