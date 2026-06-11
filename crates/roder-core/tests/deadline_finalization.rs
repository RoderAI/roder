use std::sync::{Arc, Mutex};

use futures::stream;
use roder_api::events::RoderEvent;
use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::inference::{
    AgentInferenceRequest, CompletionMetadata, InferenceCapabilities, InferenceEngine,
    InferenceEvent, InferenceEventStream, InferenceProviderContext, InferenceTurnContext,
    InstructionBundle, MessageDelta, RuntimeProfile, ToolCallCompleted,
};
use roder_api::policy_mode::PolicyMode;
use roder_api::tools::{
    ToolCall, ToolChoice, ToolContributor, ToolExecutionContext, ToolExecutor, ToolRegistry,
    ToolResult, ToolSpec,
};
use roder_api::transcript::TranscriptItem;
use roder_core::{Runtime, RuntimeConfig, StartTurnRequest};
use serde_json::json;

#[tokio::test]
async fn eval_deadline_finalization_disables_tools_and_completes_turn() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(DeadlineFinalizationEngine {
        requests: requests.clone(),
    }));
    builder.tool_contributor(Arc::new(SlowToolContributor));
    let runtime = Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                default_provider: roder_api::catalog::PROVIDER_MOCK.to_string(),
                default_model: "mock".to_string(),
                policy_mode: PolicyMode::Bypass,
                runtime_profile: RuntimeProfile::Eval,
                turn_deadline_seconds: Some(3),
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    );
    let mut rx = runtime.subscribe_events();

    let turn_id = runtime
        .start_turn(StartTurnRequest {
            thread_id: "thread-deadline-finalization".to_string(),
            message: "do slow work and finish".to_string(),
            images: Vec::new(),
            provider_override: None,
            model_override: None,
            reasoning_override: None,
            workspace: std::env::current_dir().unwrap().display().to_string(),
            instructions: InstructionBundle::default(),
            developer_context: None,
            task_ledger_required: false,
        })
        .await
        .unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(8), async {
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
    assert_eq!(requests.len(), 2);
    assert!(
        requests[0]
            .tools
            .iter()
            .any(|tool| tool.name == "slow_tool")
    );
    assert!(requests[1].tools.is_empty());
    assert_eq!(requests[1].tool_choice, ToolChoice::None);
    assert!(
        requests[1].transcript.iter().any(|item| {
            matches!(
                item,
                TranscriptItem::UserMessage(message)
                    if message.text.contains("Eval deadline finalization")
            )
        }),
        "finalization request was not injected into the model conversation"
    );
    assert_eq!(
        requests[1]
            .metadata
            .pointer("/deadlineFinalization/reserveSeconds")
            .and_then(serde_json::Value::as_u64),
        Some(1)
    );
}

#[tokio::test]
async fn eval_deadline_finalization_interrupts_model_stream_at_reserve() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(ReserveBoundaryEngine {
        requests: requests.clone(),
    }));
    let runtime = Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                default_provider: roder_api::catalog::PROVIDER_MOCK.to_string(),
                default_model: "mock".to_string(),
                policy_mode: PolicyMode::Bypass,
                runtime_profile: RuntimeProfile::Eval,
                turn_deadline_seconds: Some(3),
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    );
    let mut rx = runtime.subscribe_events();

    let turn_id = runtime
        .start_turn(StartTurnRequest {
            thread_id: "thread-deadline-reserve-finalization".to_string(),
            message: "do not run until the hard deadline".to_string(),
            images: Vec::new(),
            provider_override: None,
            model_override: None,
            reasoning_override: None,
            workspace: std::env::current_dir().unwrap().display().to_string(),
            instructions: InstructionBundle::default(),
            developer_context: None,
            task_ledger_required: false,
        })
        .await
        .unwrap();

    let mut saw_partial = false;
    tokio::time::timeout(std::time::Duration::from_secs(8), async {
        loop {
            let envelope = rx.recv().await.unwrap();
            if envelope.turn_id.as_deref() != Some(&turn_id) {
                continue;
            }
            match envelope.event {
                RoderEvent::TurnPartialResult(event) => {
                    saw_partial = event.summary.contains("partial turn state");
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
    assert!(saw_partial);
    assert_eq!(requests.len(), 2);
    assert!(
        requests[1].transcript.iter().any(|item| {
            matches!(
                item,
                TranscriptItem::UserMessage(message)
                    if message.text.contains("Eval deadline finalization")
            )
        }),
        "reserve-boundary finalization request was not injected"
    );
    assert_eq!(requests[1].tool_choice, ToolChoice::None);
}

#[tokio::test]
async fn provider_without_tool_capability_receives_no_tools() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(NoToolEngine {
        requests: requests.clone(),
    }));
    builder.tool_contributor(Arc::new(SlowToolContributor));
    let runtime = Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                default_provider: roder_api::catalog::PROVIDER_MOCK.to_string(),
                default_model: "mock".to_string(),
                policy_mode: PolicyMode::Bypass,
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    );
    let mut rx = runtime.subscribe_events();

    let turn_id = runtime
        .start_turn(StartTurnRequest {
            thread_id: "thread-no-tool-provider".to_string(),
            message: "answer without tools".to_string(),
            images: Vec::new(),
            provider_override: None,
            model_override: None,
            reasoning_override: None,
            workspace: std::env::current_dir().unwrap().display().to_string(),
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

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].tools.is_empty());
    assert_eq!(requests[0].tool_choice, ToolChoice::None);
}

struct DeadlineFinalizationEngine {
    requests: Arc<Mutex<Vec<AgentInferenceRequest>>>,
}

#[async_trait::async_trait]
impl InferenceEngine for DeadlineFinalizationEngine {
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
        let call_number = {
            let mut requests = self.requests.lock().unwrap();
            requests.push(request.clone());
            requests.len()
        };
        let events = if call_number == 1 {
            vec![
                Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "slow-1".to_string(),
                    name: "slow_tool".to_string(),
                    arguments: "{}".to_string(),
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("tool_calls".to_string()),
                    provider_response_id: None,
                })),
            ]
        } else {
            assert!(request.tools.is_empty());
            assert_eq!(request.tool_choice, ToolChoice::None);
            vec![
                Ok(InferenceEvent::MessageDelta(MessageDelta {
                    text: "finalized before deadline".to_string(),
                    phase: None,
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("stop".to_string()),
                    provider_response_id: None,
                })),
            ]
        };
        Ok(Box::pin(stream::iter(events)))
    }
}

struct ReserveBoundaryEngine {
    requests: Arc<Mutex<Vec<AgentInferenceRequest>>>,
}

#[async_trait::async_trait]
impl InferenceEngine for ReserveBoundaryEngine {
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
        let call_number = {
            let mut requests = self.requests.lock().unwrap();
            requests.push(request.clone());
            requests.len()
        };
        if call_number == 1 {
            return Ok(Box::pin(stream::once(async {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                Ok(InferenceEvent::MessageDelta(MessageDelta {
                    text: "too late".to_string(),
                    phase: None,
                }))
            })));
        }
        assert!(request.tools.is_empty());
        assert_eq!(request.tool_choice, ToolChoice::None);
        Ok(Box::pin(stream::iter(vec![
            Ok(InferenceEvent::MessageDelta(MessageDelta {
                text: "finalized at reserve boundary".to_string(),
                phase: None,
            })),
            Ok(InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some("stop".to_string()),
                provider_response_id: None,
            })),
        ])))
    }
}

struct NoToolEngine {
    requests: Arc<Mutex<Vec<AgentInferenceRequest>>>,
}

#[async_trait::async_trait]
impl InferenceEngine for NoToolEngine {
    fn id(&self) -> String {
        roder_api::catalog::PROVIDER_MOCK.to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities::text_only()
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

struct SlowToolContributor;

impl ToolContributor for SlowToolContributor {
    fn id(&self) -> String {
        "slow-tool".to_string()
    }

    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        registry.register(Arc::new(SlowTool))
    }
}

struct SlowTool;

#[async_trait::async_trait]
impl ToolExecutor for SlowTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "slow_tool".to_string(),
            description: "Sleeps long enough to enter the deadline finalization reserve."
                .to_string(),
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
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text: "slow tool finished".to_string(),
            data: json!({}),
            is_error: false,
        })
    }
}
