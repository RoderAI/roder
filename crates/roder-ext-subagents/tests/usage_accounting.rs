use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use futures::{StreamExt, stream};
use roder_api::extension::InferenceEngineId;
use roder_api::inference::*;
use roder_api::subagents::{
    SubagentDefinition, SubagentDispatcher, SubagentExitReason, SubagentPermissionMode,
    SubagentRequest,
};
use roder_api::tools::{
    ToolCall, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult, ToolSpec,
};
use roder_api::trace::{
    ParentTurnRef, SubagentTraceDelta, SubagentTraceId, SubagentTraceSink, SubagentTraceStatus,
    SubagentTraceSummary,
};
use roder_ext_subagents::{
    InProcessDispatcher, InProcessDispatcherConfig, InferenceEngineRegistry,
};
use serde_json::json;
use tokio::sync::Notify;

struct Round {
    events: Vec<anyhow::Result<InferenceEvent>>,
    pending: bool,
    start_error: bool,
    stalled: Option<Arc<Notify>>,
}

impl Round {
    fn events(events: Vec<InferenceEvent>) -> Self {
        Self {
            events: events.into_iter().map(Ok).collect(),
            pending: false,
            start_error: false,
            stalled: None,
        }
    }
}

struct ScriptedEngine(Mutex<VecDeque<Round>>);

#[async_trait::async_trait]
impl InferenceEngine for ScriptedEngine {
    fn id(&self) -> InferenceEngineId {
        "mock".to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities::coding_agent_default()
    }

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
        let round = self.0.lock().unwrap().pop_front().unwrap();
        anyhow::ensure!(!round.start_error, "stream startup failed");
        let events = stream::iter(round.events);
        if round.pending {
            Ok(Box::pin(events.chain(stream::poll_fn(move |_| {
                if let Some(stalled) = &round.stalled {
                    stalled.notify_one();
                }
                std::task::Poll::Pending
            }))))
        } else {
            Ok(Box::pin(events))
        }
    }
}

struct ReadTool;

#[async_trait::async_trait]
impl ToolExecutor for ReadTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "Read".to_string(),
            description: "Read test data".to_string(),
            parameters: json!({"type": "object"}),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        anyhow::ensure!(call.arguments["fail"] != true, "tool execution failed");
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text: "test data".to_string(),
            data: json!({}),
            is_error: false,
        })
    }
}

#[derive(Default)]
struct TraceSink {
    summaries: Mutex<Vec<SubagentTraceSummary>>,
    finished: Notify,
}

#[async_trait::async_trait]
impl SubagentTraceSink for TraceSink {
    async fn trace_created(&self, _summary: SubagentTraceSummary) {}
    async fn trace_delta(&self, _delta: SubagentTraceDelta) {}
    async fn trace_status_changed(
        &self,
        _trace_id: SubagentTraceId,
        _parent: ParentTurnRef,
        _status: SubagentTraceStatus,
        _detail: Option<String>,
    ) {
    }

    async fn trace_completed(&self, summary: SubagentTraceSummary) {
        self.summaries.lock().unwrap().push(summary);
        self.finished.notify_one();
    }

    async fn trace_failed(&self, summary: SubagentTraceSummary, _error: String) {
        self.summaries.lock().unwrap().push(summary);
        self.finished.notify_one();
    }
}

fn fixture(rounds: Vec<Round>, max_turns: u32) -> (InProcessDispatcher, Arc<TraceSink>) {
    let mut engines = InferenceEngineRegistry::new();
    engines.insert(Arc::new(ScriptedEngine(Mutex::new(rounds.into()))));
    let mut tools = ToolRegistry::default();
    tools.register(Arc::new(ReadTool)).unwrap();
    let definition = SubagentDefinition {
        agent_type: "explore".to_string(),
        description: "Usage test".to_string(),
        tools: vec!["Read".to_string()],
        model: Some("mock".to_string()),
        system_prompt: None,
        permission_mode: SubagentPermissionMode::ReadOnly,
        max_turns: Some(max_turns),
        max_result_chars: None,
    };
    let dispatcher = InProcessDispatcher::new(
        InProcessDispatcherConfig {
            default_timeout_seconds: 1,
            ..InProcessDispatcherConfig::default()
        },
        vec![definition],
        engines,
        tools,
    )
    .unwrap();
    (dispatcher, Arc::new(TraceSink::default()))
}

fn request() -> SubagentRequest {
    serde_json::from_value(json!({
        "description": "accounting",
        "prompt": "Read test data and summarize"
    }))
    .unwrap()
}

fn tool_call(arguments: &str) -> InferenceEvent {
    InferenceEvent::ToolCallCompleted(ToolCallCompleted {
        id: "read".to_string(),
        name: "Read".to_string(),
        arguments: arguments.to_string(),
    })
}

fn completed() -> InferenceEvent {
    InferenceEvent::Completed(CompletionMetadata {
        stop_reason: Some("stop".to_string()),
        provider_response_id: None,
    })
}

fn first_usage() -> TokenUsage {
    TokenUsage::new(100, 10, 110)
        .with_cached_prompt_tokens(30)
        .with_cache_creation_prompt_tokens(20)
        .with_service_tier(Some("priority".to_string()))
}

fn second_usage() -> TokenUsage {
    TokenUsage::new(50, 5, 55)
        .with_cached_prompt_tokens(10)
        .with_cache_creation_prompt_tokens(5)
        .with_service_tier(Some("default".to_string()))
}

fn total_usage() -> TokenUsage {
    let mut total = first_usage();
    total.add_assign(&second_usage());
    total
}

#[tokio::test]
async fn sums_rounds_but_only_the_last_usage_snapshot_in_each_stream() {
    let (dispatcher, sink) = fixture(
        vec![
            Round::events(vec![
                InferenceEvent::Usage(TokenUsage::new(60, 2, 62)),
                InferenceEvent::Usage(first_usage()),
                tool_call("{}"),
            ]),
            Round::events(vec![
                InferenceEvent::Usage(TokenUsage::new(25, 1, 26)),
                InferenceEvent::Usage(second_usage()),
                completed(),
            ]),
        ],
        4,
    );
    let result = dispatcher
        .dispatch_traced(
            "parent".into(),
            "turn".into(),
            request(),
            Some(sink.clone()),
        )
        .await
        .unwrap();
    assert_eq!(result.exit_reason, SubagentExitReason::Completed);
    assert_eq!(result.usage, Some(total_usage()));
    let summaries = sink.summaries.lock().unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].usage, Some(total_usage()));
}

#[tokio::test]
async fn max_turns_preserves_all_rounds() {
    let (dispatcher, sink) = fixture(
        vec![
            Round::events(vec![InferenceEvent::Usage(first_usage()), tool_call("{}")]),
            Round::events(vec![InferenceEvent::Usage(second_usage()), tool_call("{}")]),
        ],
        2,
    );
    let result = dispatcher
        .dispatch_traced(
            "parent".into(),
            "turn".into(),
            request(),
            Some(sink.clone()),
        )
        .await
        .unwrap();
    assert_eq!(result.exit_reason, SubagentExitReason::MaxTurns);
    assert_eq!(result.usage, Some(total_usage()));
    assert_eq!(sink.summaries.lock().unwrap()[0].usage, Some(total_usage()));
}

#[tokio::test]
async fn errors_after_usage_preserve_all_observed_rounds_in_failed_trace() {
    for failure in ["provider", "stream", "tool", "invalid_json", "unavailable"] {
        let mut round = Round::events(vec![
            InferenceEvent::Usage(TokenUsage::new(25, 1, 26)),
            InferenceEvent::Usage(second_usage()),
        ]);
        match failure {
            "provider" => round
                .events
                .push(Ok(InferenceEvent::Failed(InferenceFailure {
                    message: "provider failed".to_string(),
                }))),
            "stream" => round.events.push(Err(anyhow::anyhow!("stream failed"))),
            "tool" => round.events.push(Ok(tool_call(r#"{"fail":true}"#))),
            "invalid_json" => round.events.push(Ok(tool_call("{"))),
            "unavailable" => {
                round
                    .events
                    .push(Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                        id: "unknown".to_string(),
                        name: "unavailable".to_string(),
                        arguments: "{}".to_string(),
                    })));
            }
            _ => unreachable!(),
        }
        let (dispatcher, sink) = fixture(
            vec![
                Round::events(vec![InferenceEvent::Usage(first_usage()), tool_call("{}")]),
                round,
            ],
            4,
        );
        assert!(
            dispatcher
                .dispatch_traced(
                    "parent".into(),
                    "turn".into(),
                    request(),
                    Some(sink.clone())
                )
                .await
                .is_err(),
            "{failure}"
        );
        let summaries = sink.summaries.lock().unwrap();
        assert_eq!(summaries.len(), 1, "{failure}");
        assert_eq!(
            summaries[0].status,
            SubagentTraceStatus::Failed,
            "{failure}"
        );
        assert_eq!(summaries[0].usage, Some(total_usage()), "{failure}");
        assert_eq!(summaries[0].exit_reason, Some(SubagentExitReason::Failed));
    }
}

#[tokio::test]
async fn stream_startup_failure_preserves_previous_round_usage() {
    let mut failed_round = Round::events(vec![]);
    failed_round.start_error = true;
    let (dispatcher, sink) = fixture(
        vec![
            Round::events(vec![InferenceEvent::Usage(first_usage()), tool_call("{}")]),
            failed_round,
        ],
        4,
    );
    assert!(
        dispatcher
            .dispatch_traced(
                "parent".into(),
                "turn".into(),
                request(),
                Some(sink.clone())
            )
            .await
            .is_err()
    );
    let summaries = sink.summaries.lock().unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].usage, Some(first_usage()));
}

#[tokio::test(start_paused = true)]
async fn timeout_cancellation_retains_the_inflight_usage_snapshot() {
    let mut pending_round = Round::events(vec![
        InferenceEvent::Usage(TokenUsage::new(25, 1, 26)),
        InferenceEvent::Usage(second_usage()),
    ]);
    pending_round.pending = true;
    let (dispatcher, sink) = fixture(
        vec![
            Round::events(vec![InferenceEvent::Usage(first_usage()), tool_call("{}")]),
            pending_round,
        ],
        4,
    );
    let result = dispatcher
        .dispatch_traced(
            "parent".into(),
            "turn".into(),
            request(),
            Some(sink.clone()),
        )
        .await
        .unwrap();
    assert_eq!(result.exit_reason, SubagentExitReason::Timeout);
    assert_eq!(result.usage, Some(total_usage()));
    let summaries = sink.summaries.lock().unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].usage, Some(total_usage()));
}

#[tokio::test]
async fn aborted_dispatch_publishes_cancelled_trace_with_consumed_usage() {
    let stalled = Arc::new(Notify::new());
    let mut pending_round = Round::events(vec![
        InferenceEvent::Usage(TokenUsage::new(25, 1, 26)),
        InferenceEvent::Usage(second_usage()),
    ]);
    pending_round.pending = true;
    pending_round.stalled = Some(stalled.clone());
    let (dispatcher, sink) = fixture(
        vec![
            Round::events(vec![InferenceEvent::Usage(first_usage()), tool_call("{}")]),
            pending_round,
        ],
        4,
    );
    let task = tokio::spawn({
        let sink = sink.clone();
        async move {
            dispatcher
                .dispatch_traced("parent".into(), "turn".into(), request(), Some(sink))
                .await
        }
    });
    stalled.notified().await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    tokio::time::timeout(std::time::Duration::from_secs(1), sink.finished.notified())
        .await
        .expect("cancellation should publish a terminal trace");
    let summaries = sink.summaries.lock().unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].status, SubagentTraceStatus::Cancelled);
    assert_eq!(
        summaries[0].exit_reason,
        Some(SubagentExitReason::Cancelled)
    );
    assert_eq!(summaries[0].usage, Some(total_usage()));
}

#[tokio::test]
async fn a_round_without_usage_does_not_reset_previous_usage() {
    let (dispatcher, _) = fixture(
        vec![
            Round::events(vec![InferenceEvent::Usage(first_usage()), tool_call("{}")]),
            Round::events(vec![completed()]),
        ],
        4,
    );
    let result = dispatcher
        .dispatch("parent".into(), "turn".into(), request())
        .await
        .unwrap();
    assert_eq!(result.usage, Some(first_usage()));
}

#[tokio::test]
async fn missing_usage_remains_distinct_from_reported_zero() {
    for usage in [None, Some(TokenUsage::default())] {
        let mut events = Vec::new();
        if let Some(usage) = usage.clone() {
            events.push(InferenceEvent::Usage(usage));
        }
        events.push(completed());
        let (dispatcher, _) = fixture(vec![Round::events(events)], 4);
        let result = dispatcher
            .dispatch("parent".into(), "turn".into(), request())
            .await
            .unwrap();
        assert_eq!(result.usage, usage);
    }
}
