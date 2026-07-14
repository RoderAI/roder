use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures::stream;
use roder_api::catalog::PROVIDER_MOCK;
use roder_api::events::RoderEvent;
use roder_api::extension::{ExtensionRegistryBuilder, InferenceEngineId, ToolProviderId};
use roder_api::inference::{
    AgentInferenceRequest, CompletionMetadata, HostedWebSearchConfig, InferenceCapabilities,
    InferenceEngine, InferenceEvent, InferenceEventStream, InferenceProviderContext,
    InferenceTurnContext, MessageDelta, ModelDescriptor, RuntimeProfile, ToolCallCompleted,
};
use roder_api::reliability::{ReliabilityLimitDecision, ReliabilityLimitKind};
use roder_api::tools::{
    ToolCall, ToolContributor, ToolExecutionContext, ToolExecutor, ToolResult, ToolSpec,
};
use roder_core::{
    Runtime, RuntimeConfig, RuntimeReliabilityConfig, StartTurnRequest, default_instructions,
};
use serde_json::json;

#[tokio::test]
async fn reliability_limits_stop_eval_turn_after_consecutive_tool_failures() {
    let runtime = runtime_with_failing_tool(RuntimeReliabilityConfig {
        max_consecutive_tool_failures: 2,
        ..RuntimeReliabilityConfig::default()
    });
    let mut events = runtime.subscribe_events();

    runtime
        .start_turn(turn("thread-reliability-failures"))
        .await
        .unwrap();

    let mut saw_limit = false;
    loop {
        let envelope = tokio::time::timeout(std::time::Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        if let RoderEvent::ReliabilityLimitRecorded(event) = &envelope.event {
            saw_limit = true;
            assert_eq!(
                event.limit_kind,
                ReliabilityLimitKind::ConsecutiveToolFailures
            );
            assert_eq!(event.decision, ReliabilityLimitDecision::StopTurn);
            assert_eq!(event.current, 2);
            assert_eq!(event.limit, 2);
            assert_eq!(event.context.thread_id, "thread-reliability-failures");
            assert_eq!(event.context.provider.as_deref(), Some(PROVIDER_MOCK));
        }
        if let RoderEvent::TurnFailed(failed) = &envelope.event {
            assert_eq!(failed.error_kind.as_deref(), Some("reliability_limit"));
            break;
        }
    }

    assert!(saw_limit);
}

#[tokio::test]
async fn reliability_limits_model_call_emits_interactive_continuation_decision() {
    let runtime = runtime_with_final_engine(RuntimeReliabilityConfig {
        max_model_calls_per_turn: 0,
        ..RuntimeReliabilityConfig::default()
    });
    let mut events = runtime.subscribe_events();

    runtime
        .start_turn(StartTurnRequest {
            thread_id: "thread-reliability-model-calls".to_string(),
            message: "answer".to_string(),
            images: Vec::new(),
            provider_override: None,
            model_override: None,
            reasoning_override: None,
            workspace: std::env::current_dir().unwrap().display().to_string(),
            instructions: default_instructions(),
            developer_context: None,
            task_ledger_required: false,
        })
        .await
        .unwrap();

    let mut saw_partial = false;
    loop {
        let envelope = tokio::time::timeout(std::time::Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        match &envelope.event {
            RoderEvent::ReliabilityLimitRecorded(event) => {
                assert_eq!(event.limit_kind, ReliabilityLimitKind::ModelCallsPerTurn);
                assert_eq!(
                    event.decision,
                    ReliabilityLimitDecision::RequestContinuation
                );
            }
            RoderEvent::TurnPartialResult(_) => saw_partial = true,
            RoderEvent::TurnFailed(failed) => {
                assert_eq!(failed.error_kind.as_deref(), Some("reliability_limit"));
                break;
            }
            _ => {}
        }
    }
    assert!(saw_partial);
}

#[tokio::test]
async fn continue_on_failure_limit_recovers_eval_turn_instead_of_stopping() {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FailThenFinishEngine {
        requests_by_turn: Mutex::new(HashMap::new()),
    }));
    builder.tool_contributor(Arc::new(FailingToolContributor));
    let runtime = runtime(
        builder,
        RuntimeReliabilityConfig {
            max_consecutive_tool_failures: 2,
            continue_on_failure_limit: true,
            ..RuntimeReliabilityConfig::default()
        },
        RuntimeProfile::Eval,
    );
    let mut events = runtime.subscribe_events();
    let thread_id = "thread-continue-on-failure";

    runtime.start_turn(turn(thread_id)).await.unwrap();

    let mut saw_continuation = false;
    loop {
        let envelope = tokio::time::timeout(std::time::Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        match &envelope.event {
            RoderEvent::ReliabilityLimitRecorded(event) => {
                assert_eq!(
                    event.limit_kind,
                    ReliabilityLimitKind::ConsecutiveToolFailures
                );
                assert_eq!(
                    event.decision,
                    ReliabilityLimitDecision::RequestContinuation
                );
                saw_continuation = true;
            }
            RoderEvent::TurnFailed(failed) => panic!("turn unexpectedly failed: {failed:?}"),
            RoderEvent::TurnCompleted(completed) if completed.thread_id == thread_id => break,
            _ => {}
        }
    }

    assert!(
        saw_continuation,
        "expected a RequestContinuation reliability limit before completion"
    );
}

#[test]
fn reliability_default_tool_failure_limit_allows_extended_recovery() {
    assert_eq!(
        RuntimeReliabilityConfig::default().max_tool_failures_per_turn,
        128
    );
}

#[tokio::test]
async fn reliability_tool_failure_count_resets_between_turns() {
    let runtime = runtime_with_once_failing_tool(RuntimeReliabilityConfig {
        max_consecutive_tool_failures: 10,
        max_tool_failures_per_turn: 2,
        ..RuntimeReliabilityConfig::default()
    });
    let mut events = runtime.subscribe_events();
    let thread_id = "thread-reliability-reset";

    runtime.start_turn(turn(thread_id)).await.unwrap();
    wait_for_completed_turn(&mut events, thread_id).await;

    runtime.start_turn(turn(thread_id)).await.unwrap();
    wait_for_completed_turn(&mut events, thread_id).await;
}

fn runtime_with_failing_tool(reliability: RuntimeReliabilityConfig) -> Arc<Runtime> {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(RepeatingToolEngine {
        requests: Mutex::new(0),
    }));
    builder.tool_contributor(Arc::new(FailingToolContributor));
    runtime(builder, reliability, RuntimeProfile::Eval)
}

fn runtime_with_final_engine(reliability: RuntimeReliabilityConfig) -> Arc<Runtime> {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FinalEngine));
    runtime(builder, reliability, RuntimeProfile::Interactive)
}

fn runtime_with_once_failing_tool(reliability: RuntimeReliabilityConfig) -> Arc<Runtime> {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(OncePerTurnToolEngine {
        requests_by_turn: Mutex::new(HashMap::new()),
    }));
    builder.tool_contributor(Arc::new(FailingToolContributor));
    runtime(builder, reliability, RuntimeProfile::Eval)
}

fn runtime(
    builder: ExtensionRegistryBuilder,
    reliability: RuntimeReliabilityConfig,
    runtime_profile: RuntimeProfile,
) -> Arc<Runtime> {
    Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                hosted_web_search: HostedWebSearchConfig::disabled(),
                runtime_profile,
                reliability,
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    )
}

fn turn(thread_id: &str) -> StartTurnRequest {
    StartTurnRequest {
        thread_id: thread_id.to_string(),
        message: "use unstable".to_string(),
        images: Vec::new(),
        provider_override: None,
        model_override: None,
        reasoning_override: None,
        workspace: std::env::current_dir().unwrap().display().to_string(),
        instructions: default_instructions(),
        developer_context: None,
        task_ledger_required: false,
    }
}

async fn wait_for_completed_turn(
    events: &mut tokio::sync::broadcast::Receiver<roder_api::events::EventEnvelope>,
    thread_id: &str,
) {
    loop {
        let envelope = tokio::time::timeout(std::time::Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        match &envelope.event {
            RoderEvent::ReliabilityLimitRecorded(event) if event.context.thread_id == thread_id => {
                panic!("unexpected reliability limit: {event:?}");
            }
            RoderEvent::TurnFailed(failed) if failed.thread_id == thread_id => {
                panic!("turn failed: {failed:?}");
            }
            RoderEvent::TurnCompleted(completed) if completed.thread_id == thread_id => break,
            _ => {}
        }
    }
}

struct RepeatingToolEngine {
    requests: Mutex<usize>,
}

#[async_trait::async_trait]
impl InferenceEngine for RepeatingToolEngine {
    fn id(&self) -> InferenceEngineId {
        PROVIDER_MOCK.to_string()
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
        let mut requests = self.requests.lock().unwrap();
        *requests += 1;
        let call_id = format!("call-{}", *requests);
        Ok(Box::pin(stream::iter(vec![
            Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                id: call_id,
                name: "unstable".to_string(),
                arguments: "{}".to_string(),
            })),
            Ok(InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some("tool_calls".to_string()),
                provider_response_id: None,
            })),
        ])))
    }
}

struct OncePerTurnToolEngine {
    requests_by_turn: Mutex<HashMap<String, usize>>,
}

#[async_trait::async_trait]
impl InferenceEngine for OncePerTurnToolEngine {
    fn id(&self) -> InferenceEngineId {
        PROVIDER_MOCK.to_string()
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
        ctx: InferenceTurnContext<'_>,
        _request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let mut requests = self.requests_by_turn.lock().unwrap();
        let request_count = requests.entry(ctx.turn_id.to_string()).or_default();
        *request_count += 1;
        if *request_count == 1 {
            let call_id = format!("call-{}", ctx.turn_id);
            return Ok(Box::pin(stream::iter(vec![
                Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: call_id,
                    name: "unstable".to_string(),
                    arguments: "{}".to_string(),
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("tool_calls".to_string()),
                    provider_response_id: None,
                })),
            ])));
        }

        Ok(Box::pin(stream::iter(vec![
            Ok(InferenceEvent::MessageDelta(MessageDelta {
                text: "recovered".to_string(),
                phase: None,
            })),
            Ok(InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some("stop".to_string()),
                provider_response_id: None,
            })),
        ])))
    }
}

/// Emits a failing tool call for the first two model calls of a turn, then a
/// final message. With `continue_on_failure_limit` the consecutive-failure limit
/// (2) is reached on the second call and continued, letting the third call
/// finalize the turn.
struct FailThenFinishEngine {
    requests_by_turn: Mutex<HashMap<String, usize>>,
}

#[async_trait::async_trait]
impl InferenceEngine for FailThenFinishEngine {
    fn id(&self) -> InferenceEngineId {
        PROVIDER_MOCK.to_string()
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
        ctx: InferenceTurnContext<'_>,
        _request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let mut requests = self.requests_by_turn.lock().unwrap();
        let request_count = requests.entry(ctx.turn_id.to_string()).or_default();
        *request_count += 1;
        if *request_count <= 2 {
            let call_id = format!("call-{}-{}", ctx.turn_id, *request_count);
            return Ok(Box::pin(stream::iter(vec![
                Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: call_id,
                    name: "unstable".to_string(),
                    arguments: "{}".to_string(),
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("tool_calls".to_string()),
                    provider_response_id: None,
                })),
            ])));
        }

        Ok(Box::pin(stream::iter(vec![
            Ok(InferenceEvent::MessageDelta(MessageDelta {
                text: "recovered".to_string(),
                phase: None,
            })),
            Ok(InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some("stop".to_string()),
                provider_response_id: None,
            })),
        ])))
    }
}

struct FinalEngine;

#[async_trait::async_trait]
impl InferenceEngine for FinalEngine {
    fn id(&self) -> InferenceEngineId {
        PROVIDER_MOCK.to_string()
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

struct FailingToolContributor;

impl ToolContributor for FailingToolContributor {
    fn id(&self) -> ToolProviderId {
        "test-failing-tool".to_string()
    }

    fn contribute(&self, registry: &mut roder_api::tools::ToolRegistry) -> anyhow::Result<()> {
        registry.register(Arc::new(FailingTool))
    }
}

struct FailingTool;

#[async_trait::async_trait]
impl ToolExecutor for FailingTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "unstable".to_string(),
            description: "Always fails".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {},
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
            text: "simulated failure".to_string(),
            data: json!({ "error": { "kind": "simulated" } }),
            is_error: true,
        })
    }
}
