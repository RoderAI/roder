use std::sync::{Arc, Mutex};

use futures::stream;
use roder_api::catalog::PROVIDER_MOCK;
use roder_api::events::{
    RoderEvent, ToolCallValidationFailureClass, ToolCallValidationRepairStatus,
};
use roder_api::extension::{ExtensionRegistryBuilder, InferenceEngineId, ToolProviderId};
use roder_api::inference::{
    AgentInferenceRequest, CompletionMetadata, HostedWebSearchConfig, InferenceCapabilities,
    InferenceEngine, InferenceEvent, InferenceEventStream, InferenceProviderContext,
    InferenceTurnContext, MessageDelta, ModelDescriptor, ToolCallCompleted,
};
use roder_api::tools::{
    ToolCall, ToolContributor, ToolExecutionContext, ToolExecutor, ToolResult, ToolSpec,
};
use roder_core::{Runtime, RuntimeConfig, StartTurnRequest, default_instructions};
use serde_json::json;

#[tokio::test]
async fn tool_validation_rejects_unexpected_fields_before_executor() {
    assert_rejected_before_executor(
        r#"{"text":"hi","extra":true}"#.to_string(),
        "thread-validation-extra",
        ToolCallValidationFailureClass::UnexpectedProperty,
    )
    .await;
}

#[tokio::test]
async fn tool_validation_rejects_invalid_json_before_executor() {
    assert_rejected_before_executor(
        "{".to_string(),
        "thread-validation-invalid-json",
        ToolCallValidationFailureClass::InvalidJson,
    )
    .await;
}

#[tokio::test]
async fn tool_validation_rejects_missing_required_before_executor() {
    assert_rejected_before_executor(
        "{}".to_string(),
        "thread-validation-missing",
        ToolCallValidationFailureClass::MissingRequired,
    )
    .await;
}

#[tokio::test]
async fn tool_validation_rejects_wrong_types_before_executor() {
    assert_rejected_before_executor(
        r#"{"text":false}"#.to_string(),
        "thread-validation-wrong-type",
        ToolCallValidationFailureClass::WrongType,
    )
    .await;
}

async fn assert_rejected_before_executor(
    arguments: String,
    thread_id: &str,
    expected_class: ToolCallValidationFailureClass,
) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let runtime = runtime_with_tool_call(arguments, calls.clone());
    let mut events = runtime.subscribe_events();

    runtime.start_turn(turn(thread_id)).await.unwrap();

    let mut saw_validation = false;
    loop {
        let event = tokio::time::timeout(std::time::Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        if let RoderEvent::ToolCallValidationRecorded(recorded) = &event.event {
            saw_validation = true;
            assert_eq!(recorded.failure_class, expected_class);
            assert_eq!(
                recorded.repair_status,
                ToolCallValidationRepairStatus::NotNeeded
            );
        }
        if event.kind == "turn.completed" && event.thread_id.as_deref() == Some(thread_id) {
            break;
        }
    }

    assert!(saw_validation);
    assert!(calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn tool_validation_repairs_stringified_json_object_arguments() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let stringified = serde_json::to_string(r#"{"text":"hi"}"#).unwrap();
    let runtime = runtime_with_tool_call(stringified, calls.clone());
    let mut events = runtime.subscribe_events();

    runtime
        .start_turn(turn("thread-validation-repair"))
        .await
        .unwrap();

    let mut saw_repair = false;
    loop {
        let event = tokio::time::timeout(std::time::Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        if let RoderEvent::ToolCallValidationRecorded(recorded) = &event.event {
            saw_repair = true;
            assert_eq!(
                recorded.failure_class,
                ToolCallValidationFailureClass::SchemaRepairApplied
            );
            assert_eq!(
                recorded.repair_status,
                ToolCallValidationRepairStatus::Applied
            );
        }
        if event.kind == "turn.completed"
            && event.thread_id.as_deref() == Some("thread-validation-repair")
        {
            break;
        }
    }

    assert!(saw_repair);
    assert_eq!(calls.lock().unwrap().as_slice(), &[json!({ "text": "hi" })]);
}

fn runtime_with_tool_call(
    arguments: String,
    calls: Arc<Mutex<Vec<serde_json::Value>>>,
) -> Arc<Runtime> {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(ValidationEngine {
        arguments,
        requests: Mutex::new(0),
    }));
    builder.tool_contributor(Arc::new(EchoContributor { calls }));
    Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                hosted_web_search: HostedWebSearchConfig::disabled(),
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    )
}

fn turn(thread_id: &str) -> StartTurnRequest {
    StartTurnRequest {
        thread_id: thread_id.to_string(),
        message: "use echo".to_string(),
        images: Vec::new(),
        provider_override: None,
        model_override: None,
        workspace: std::env::current_dir().unwrap().display().to_string(),
        instructions: default_instructions(),
        task_ledger_required: false,
    }
}

struct ValidationEngine {
    arguments: String,
    requests: Mutex<usize>,
}

#[async_trait::async_trait]
impl InferenceEngine for ValidationEngine {
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
        if *requests == 1 {
            Ok(Box::pin(stream::iter(vec![
                Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "call-validation".to_string(),
                    name: "echo".to_string(),
                    arguments: self.arguments.clone(),
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("tool_calls".to_string()),
                    provider_response_id: None,
                })),
            ])))
        } else {
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
}

struct EchoContributor {
    calls: Arc<Mutex<Vec<serde_json::Value>>>,
}

impl ToolContributor for EchoContributor {
    fn id(&self) -> ToolProviderId {
        "test-echo-validation".to_string()
    }

    fn contribute(&self, registry: &mut roder_api::tools::ToolRegistry) -> anyhow::Result<()> {
        registry.register(Arc::new(EchoTool {
            calls: self.calls.clone(),
        }))
    }
}

struct EchoTool {
    calls: Arc<Mutex<Vec<serde_json::Value>>>,
}

#[async_trait::async_trait]
impl ToolExecutor for EchoTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "echo".to_string(),
            description: "Echo text".to_string(),
            parameters: json!({
                "type": "object",
                "required": ["text"],
                "properties": { "text": { "type": "string" } },
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        self.calls.lock().unwrap().push(call.arguments.clone());
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text: "ok".to_string(),
            data: json!({}),
            is_error: false,
        })
    }
}
