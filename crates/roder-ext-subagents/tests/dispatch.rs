use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::stream;
use roder_api::extension::InferenceEngineId;
use roder_api::inference::*;
use roder_api::subagents::{
    SubagentDefinition, SubagentDispatcher, SubagentPermissionMode, SubagentRequest,
};
use roder_api::tools::{
    ToolCall, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult, ToolSpec,
};
use roder_api::trace::{
    ParentTurnRef, SubagentTraceDelta, SubagentTraceId, SubagentTraceSink, SubagentTraceStatus,
    SubagentTraceSummary,
};
use roder_ext_subagents::{
    AgentLoadConfig, InProcessDispatcher, InProcessDispatcherConfig, InferenceEngineRegistry,
    load_agent_definitions, parse_agent_definition,
};
use serde_json::json;

#[test]
fn parses_markdown_frontmatter_definition() {
    let definition = parse_agent_definition(
        r#"---
name: explore
description: Read-only repository exploration.
model: claude-haiku-4-5
tools: [Read, Grep, Glob]
permission_mode: read_only
max_turns: 8
max_result_chars: 4000
---

You are an exploration subagent.
"#,
    )
    .unwrap();

    assert_eq!(definition.agent_type, "explore");
    assert_eq!(definition.description, "Read-only repository exploration.");
    assert_eq!(definition.model.as_deref(), Some("claude-haiku-4-5"));
    assert_eq!(definition.tools, ["Read", "Grep", "Glob"]);
    assert_eq!(definition.permission_mode, SubagentPermissionMode::ReadOnly);
    assert_eq!(definition.max_turns, Some(8));
    assert_eq!(definition.max_result_chars, Some(4000));
    assert_eq!(
        definition.system_prompt.as_deref(),
        Some("You are an exploration subagent.")
    );

    let block_list = parse_agent_definition(
        "---\nname: review\ndescription: Review code\ntools:\n  - Read\n  - Grep\n---\nBody\n",
    )
    .unwrap();
    assert_eq!(block_list.tools, ["Read", "Grep"]);
}

#[tokio::test]
async fn loader_applies_deterministic_workspace_override() {
    let base = unique_temp_dir("loader");
    let user_dir = base.join("user");
    let workspace_dir = base.join("workspace");
    tokio::fs::create_dir_all(&user_dir).await.unwrap();
    tokio::fs::create_dir_all(&workspace_dir).await.unwrap();
    tokio::fs::write(
        user_dir.join("b.md"),
        agent_markdown("review", "User review", "[Read]", "user review"),
    )
    .await
    .unwrap();
    tokio::fs::write(
        user_dir.join("a.md"),
        agent_markdown("explore", "User explore", "[Read]", "user explore"),
    )
    .await
    .unwrap();
    tokio::fs::write(
        workspace_dir.join("z.md"),
        agent_markdown(
            "explore",
            "Workspace explore",
            "[Read, Grep]",
            "workspace explore",
        ),
    )
    .await
    .unwrap();

    let definitions = load_agent_definitions(&AgentLoadConfig {
        user_dir: Some(user_dir),
        workspace_dir: Some(workspace_dir),
    })
    .await
    .unwrap();

    let names = definitions
        .iter()
        .map(|definition| definition.agent_type.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, ["explore", "review"]);
    let explore = definitions
        .iter()
        .find(|definition| definition.agent_type == "explore")
        .unwrap();
    assert_eq!(explore.description, "Workspace explore");
    assert_eq!(explore.tools, ["Read", "Grep"]);
}

#[tokio::test]
async fn filters_child_tools_through_definition_whitelist() {
    let dispatcher = dispatcher_with_engine(Arc::new(ScriptedEngine::new(vec![vec![
        Ok(InferenceEvent::MessageDelta(MessageDelta {
            text: "done".to_string(),
            phase: None,
        })),
        Ok(InferenceEvent::Completed(CompletionMetadata {
            stop_reason: Some("stop".to_string()),
            provider_response_id: None,
        })),
    ]])));
    let definition = dispatcher
        .definitions()
        .into_iter()
        .find(|definition| definition.agent_type == "explore")
        .unwrap();

    let tools = dispatcher
        .filtered_tool_registry(&definition, Some(&["Read".to_string()]))
        .unwrap();
    assert_eq!(
        tools
            .specs()
            .into_iter()
            .map(|spec| spec.name)
            .collect::<Vec<_>>(),
        ["Read"]
    );
    let err = match dispatcher.filtered_tool_registry(&definition, Some(&["Shell".to_string()])) {
        Ok(_) => panic!("Shell should be rejected by the definition whitelist"),
        Err(err) => err,
    };
    assert!(err.to_string().contains("not allowed"));
}

#[tokio::test]
async fn depth_limit_is_enforced_before_engine_invocation() {
    let engine = Arc::new(BlockingEngine::default());
    let dispatcher = dispatcher_with_config_and_engine(
        InProcessDispatcherConfig {
            max_depth: 1,
            ..InProcessDispatcherConfig::default()
        },
        engine.clone(),
    );

    let err = dispatcher
        .dispatch_at_depth(1, "parent".to_string(), "turn".to_string(), request())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("max_depth"));
    assert_eq!(engine.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn max_concurrent_dispatches_reject_extra_work_before_engine_invocation() {
    let engine = Arc::new(BlockingEngine::default());
    let dispatcher = Arc::new(dispatcher_with_config_and_engine(
        InProcessDispatcherConfig {
            max_concurrent: 1,
            default_timeout_seconds: 10,
            ..InProcessDispatcherConfig::default()
        },
        engine.clone(),
    ));

    let first = {
        let dispatcher = dispatcher.clone();
        tokio::spawn(async move {
            dispatcher
                .dispatch("parent".to_string(), "turn-1".to_string(), request())
                .await
        })
    };
    while engine.calls.load(Ordering::SeqCst) == 0 {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    let err = dispatcher
        .dispatch("parent".to_string(), "turn-2".to_string(), request())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("max_concurrent"));
    engine.release.store(true, Ordering::SeqCst);
    first.await.unwrap().unwrap();
    assert_eq!(engine.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn timeout_cancels_child_engine_future() {
    let engine = Arc::new(CancellableEngine::default());
    let dispatcher = dispatcher_with_config_and_engine(
        InProcessDispatcherConfig {
            default_timeout_seconds: 1,
            ..InProcessDispatcherConfig::default()
        },
        engine.clone(),
    );

    let result = dispatcher
        .dispatch(
            "parent".to_string(),
            "turn".to_string(),
            SubagentRequest {
                timeout_seconds: Some(1),
                ..request()
            },
        )
        .await
        .unwrap();

    assert_eq!(
        result.exit_reason,
        roder_api::subagents::SubagentExitReason::Timeout
    );
    assert_eq!(engine.calls.load(Ordering::SeqCst), 1);
    assert!(engine.dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn fake_provider_child_run_returns_deterministic_result_and_truncates_transcript() {
    let engine = Arc::new(ScriptedEngine::new(vec![
        vec![
            Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                id: "call_1".to_string(),
                name: "Read".to_string(),
                arguments: r#"{"text":"tool output"}"#.to_string(),
            })),
            Ok(InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some("tool_calls".to_string()),
                provider_response_id: None,
            })),
        ],
        vec![
            Ok(InferenceEvent::MessageDelta(MessageDelta {
                text: "final child answer that is intentionally long".to_string(),
                phase: None,
            })),
            Ok(InferenceEvent::Usage(TokenUsage {
                prompt_tokens: 1,
                completion_tokens: 2,
                total_tokens: 3,
            })),
            Ok(InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some("stop".to_string()),
                provider_response_id: None,
            })),
        ],
    ]));
    let dispatcher = dispatcher_with_config_and_engine(
        InProcessDispatcherConfig {
            include_child_transcript: true,
            default_max_result_chars: 12,
            ..InProcessDispatcherConfig::default()
        },
        engine.clone(),
    );

    let result = dispatcher
        .dispatch("parent".to_string(), "turn".to_string(), request())
        .await
        .unwrap();

    assert_eq!(result.final_message.chars().count(), 12);
    assert_eq!(result.usage.unwrap().total_tokens, 3);
    assert!(result.transcript.unwrap()["truncated"].as_bool().unwrap());
    assert_eq!(engine.requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn trace_sink_receives_child_status_and_tool_deltas() {
    let engine = Arc::new(ScriptedEngine::new(vec![
        vec![
            Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                id: "call_1".to_string(),
                name: "Read".to_string(),
                arguments: r#"{"text":"tool output"}"#.to_string(),
            })),
            Ok(InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some("tool_calls".to_string()),
                provider_response_id: None,
            })),
        ],
        vec![
            Ok(InferenceEvent::ReasoningDelta(ReasoningDelta {
                text: "thinking".to_string(),
            })),
            Ok(InferenceEvent::MessageDelta(MessageDelta {
                text: "done".to_string(),
                phase: None,
            })),
            Ok(InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some("stop".to_string()),
                provider_response_id: None,
            })),
        ],
    ]));
    let dispatcher = dispatcher_with_engine(engine);
    let sink = Arc::new(CapturingTraceSink::default());

    let result = dispatcher
        .dispatch_traced(
            "parent-thread".to_string(),
            "parent-turn".to_string(),
            request(),
            Some(sink.clone()),
        )
        .await
        .unwrap();

    assert_eq!(
        result.exit_reason,
        roder_api::subagents::SubagentExitReason::Completed
    );
    let events = sink.events.lock().unwrap().clone();
    assert!(matches!(
        events.first().map(String::as_str),
        Some("created:queued")
    ));
    assert!(events.contains(&"status:running".to_string()));
    assert!(events.contains(&"delta:toolCall".to_string()));
    assert!(events.contains(&"delta:toolResult".to_string()));
    assert!(events.contains(&"delta:reasoning".to_string()));
    assert!(events.contains(&"delta:message".to_string()));
    assert!(matches!(
        events.last().map(String::as_str),
        Some("completed")
    ));
}

fn dispatcher_with_engine(engine: Arc<dyn InferenceEngine>) -> InProcessDispatcher {
    dispatcher_with_config_and_engine(InProcessDispatcherConfig::default(), engine)
}

fn dispatcher_with_config_and_engine(
    config: InProcessDispatcherConfig,
    engine: Arc<dyn InferenceEngine>,
) -> InProcessDispatcher {
    let mut registry = InferenceEngineRegistry::new();
    registry.insert(engine);
    InProcessDispatcher::new(config, vec![definition()], registry, tool_registry()).unwrap()
}

fn definition() -> SubagentDefinition {
    SubagentDefinition {
        agent_type: "explore".to_string(),
        description: "Explore".to_string(),
        tools: vec!["Read".to_string(), "Grep".to_string()],
        model: Some("mock".to_string()),
        system_prompt: Some("System".to_string()),
        permission_mode: SubagentPermissionMode::ReadOnly,
        max_turns: Some(4),
        max_result_chars: None,
    }
}

fn request() -> SubagentRequest {
    SubagentRequest {
        description: "check".to_string(),
        prompt: "inspect".to_string(),
        subagent_type: Some("explore".to_string()),
        model: None,
        tools: None,
        inputs: None,
        timeout_seconds: None,
    }
}

fn tool_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::default();
    registry.register(Arc::new(EchoTool("Read"))).unwrap();
    registry.register(Arc::new(EchoTool("Grep"))).unwrap();
    registry.register(Arc::new(EchoTool("Shell"))).unwrap();
    registry
}

fn agent_markdown(name: &str, description: &str, tools: &str, body: &str) -> String {
    format!("---\nname: {name}\ndescription: {description}\ntools: {tools}\n---\n\n{body}\n")
}

fn unique_temp_dir(prefix: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "roder-ext-subagents-{prefix}-{}",
        uuid::Uuid::new_v4()
    ))
}

struct EchoTool(&'static str);

#[async_trait::async_trait]
impl ToolExecutor for EchoTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.0.to_string(),
            description: format!("{} tool", self.0),
            parameters: json!({
                "type": "object",
                "properties": { "text": { "type": "string" } },
                "required": ["text"]
            }),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let text = call
            .arguments
            .get("text")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text,
            data: json!({}),
            is_error: false,
        })
    }
}

macro_rules! common_engine_methods {
    ($capabilities:expr) => {
        fn id(&self) -> InferenceEngineId {
            "mock".to_string()
        }

        fn capabilities(&self) -> InferenceCapabilities {
            $capabilities
        }
    };
}

#[derive(Default)]
struct BlockingEngine {
    calls: AtomicUsize,
    release: AtomicBool,
}

#[async_trait::async_trait]
impl InferenceEngine for BlockingEngine {
    common_engine_methods!(InferenceCapabilities::text_only());

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(Vec::new())
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        _request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        while !self.release.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
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

#[derive(Default)]
struct CancellableEngine {
    calls: AtomicUsize,
    dropped: AtomicBool,
}

#[async_trait::async_trait]
impl InferenceEngine for CancellableEngine {
    common_engine_methods!(InferenceCapabilities::text_only());

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(Vec::new())
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        _request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let dropped = &self.dropped;
        struct SetDrop<'a>(&'a AtomicBool);
        impl Drop for SetDrop<'_> {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let _set_drop = SetDrop(dropped);
        tokio::time::sleep(Duration::from_secs(60)).await;
        Ok(Box::pin(stream::empty()))
    }
}

struct ScriptedEngine {
    scripts: Mutex<Vec<Vec<anyhow::Result<InferenceEvent>>>>,
    requests: Mutex<Vec<AgentInferenceRequest>>,
}

#[derive(Default)]
struct CapturingTraceSink {
    events: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl SubagentTraceSink for CapturingTraceSink {
    async fn trace_created(&self, summary: SubagentTraceSummary) {
        self.events
            .lock()
            .unwrap()
            .push(format!("created:{:?}", summary.status).to_lowercase());
    }

    async fn trace_delta(&self, delta: SubagentTraceDelta) {
        let kind = match delta.item {
            roder_api::trace::SubagentTraceItem::Message { .. } => "message",
            roder_api::trace::SubagentTraceItem::Reasoning { .. } => "reasoning",
            roder_api::trace::SubagentTraceItem::ToolCall { .. } => "toolCall",
            roder_api::trace::SubagentTraceItem::ToolResult { .. } => "toolResult",
            roder_api::trace::SubagentTraceItem::Status { .. } => "status",
        };
        self.events.lock().unwrap().push(format!("delta:{kind}"));
    }

    async fn trace_status_changed(
        &self,
        _trace_id: SubagentTraceId,
        _parent: ParentTurnRef,
        status: SubagentTraceStatus,
        _detail: Option<String>,
    ) {
        self.events
            .lock()
            .unwrap()
            .push(format!("status:{status:?}").to_lowercase());
    }

    async fn trace_completed(&self, _summary: SubagentTraceSummary) {
        self.events.lock().unwrap().push("completed".to_string());
    }

    async fn trace_failed(&self, _summary: SubagentTraceSummary, error: String) {
        self.events.lock().unwrap().push(format!("failed:{error}"));
    }
}

impl ScriptedEngine {
    fn new(scripts: Vec<Vec<anyhow::Result<InferenceEvent>>>) -> Self {
        Self {
            scripts: Mutex::new(scripts),
            requests: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl InferenceEngine for ScriptedEngine {
    common_engine_methods!(InferenceCapabilities::coding_agent_default());

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(Vec::new())
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        self.requests.lock().unwrap().push(request);
        let events = self.scripts.lock().unwrap().remove(0);
        Ok(Box::pin(stream::iter(events)))
    }
}
