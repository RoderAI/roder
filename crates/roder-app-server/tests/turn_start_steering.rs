use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures::stream;
use roder_api::catalog::PROVIDER_MOCK;
use roder_api::events::{EventEnvelope, ThreadId};
use roder_api::extension::{ExtensionRegistryBuilder, InferenceEngineId, ToolProviderId};
use roder_api::inference::{
    AgentInferenceRequest, CompletionMetadata, InferenceCapabilities, InferenceEngine,
    InferenceEvent, InferenceEventStream, InferenceProviderContext, InferenceTurnContext,
    MessageDelta, ModelDescriptor, ToolCallCompleted,
};
use roder_api::thread::{ThreadMetadata, ThreadSnapshot, ThreadStore, ThreadStoreFactory};
use roder_api::tools::{
    ToolCall, ToolContributor, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult,
    ToolSpec,
};
use roder_api::transcript::TranscriptItem;
use roder_app_server::{AppServer, LocalAppClient};
use roder_core::{Runtime, RuntimeConfig};
use roder_protocol::{
    JsonRpcRequest, ThreadStartParams, ThreadStartResult, TurnInputItem, TurnStartParams,
    TurnStartResult, WorkspaceCreateParams, WorkspaceCreateResult, WorkspaceRootInput,
};
use serde_json::json;
use tokio::sync::{Mutex, Notify, broadcast};

struct ToolThenFinalEngine {
    requests: Mutex<Vec<AgentInferenceRequest>>,
}

#[async_trait::async_trait]
impl InferenceEngine for ToolThenFinalEngine {
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
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let mut requests = self.requests.lock().await;
        requests.push(request);
        let request_number = requests.len();
        drop(requests);

        let events = if request_number == 1 {
            vec![
                Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "call_blocking_echo".to_string(),
                    name: "blocking_echo".to_string(),
                    arguments: json!({ "text": "from tool" }).to_string(),
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("tool_calls".to_string()),
                    provider_response_id: None,
                })),
            ]
        } else {
            vec![
                Ok(InferenceEvent::MessageDelta(MessageDelta {
                    text: "done".to_string(),
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

#[derive(Default)]
struct MetadataDroppingThreadStoreFactory {
    snapshots: Arc<Mutex<HashMap<String, ThreadSnapshot>>>,
}

struct MetadataDroppingThreadStore {
    snapshots: Arc<Mutex<HashMap<String, ThreadSnapshot>>>,
}

impl ThreadStoreFactory for MetadataDroppingThreadStoreFactory {
    fn id(&self) -> roder_api::thread::ThreadStoreId {
        "metadata-dropping".to_string()
    }

    fn create(&self) -> Arc<dyn ThreadStore> {
        Arc::new(MetadataDroppingThreadStore {
            snapshots: Arc::clone(&self.snapshots),
        })
    }
}

#[async_trait::async_trait]
impl ThreadStore for MetadataDroppingThreadStore {
    fn id(&self) -> roder_api::thread::ThreadStoreId {
        "metadata-dropping".to_string()
    }

    async fn create_thread(&self, metadata: ThreadMetadata) -> anyhow::Result<ThreadMetadata> {
        self.snapshots.lock().await.insert(
            metadata.thread_id.clone(),
            ThreadSnapshot {
                metadata: None,
                events: Vec::new(),
                turns: Vec::new(),
                item_events: Vec::new(),
                extension_states: Vec::new(),
            },
        );
        Ok(metadata)
    }

    async fn list_threads(&self) -> anyhow::Result<Vec<ThreadMetadata>> {
        Ok(Vec::new())
    }

    async fn load_thread(&self, thread_id: &ThreadId) -> anyhow::Result<Option<ThreadSnapshot>> {
        Ok(self.snapshots.lock().await.get(thread_id).cloned())
    }

    async fn append_event(
        &self,
        thread_id: &ThreadId,
        envelope: &EventEnvelope,
    ) -> anyhow::Result<()> {
        if let Some(snapshot) = self.snapshots.lock().await.get_mut(thread_id) {
            snapshot.events.push(envelope.clone());
        }
        Ok(())
    }
}

struct BlockingEchoContributor {
    release: Arc<Notify>,
}

impl ToolContributor for BlockingEchoContributor {
    fn id(&self) -> ToolProviderId {
        "test-blocking-echo".to_string()
    }

    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        registry.register(Arc::new(BlockingEcho {
            release: Arc::clone(&self.release),
        }))
    }
}

struct BlockingEcho {
    release: Arc<Notify>,
}

#[async_trait::async_trait]
impl ToolExecutor for BlockingEcho {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "blocking_echo".to_string(),
            description: "Echo text after the test releases the tool.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "text": { "type": "string" }
                },
                "required": ["text"]
            }),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        self.release.notified().await;
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text: "from tool".to_string(),
            data: json!({ "text": "from tool" }),
            is_error: false,
        })
    }
}

#[tokio::test]
async fn turn_start_uses_protocol_thread_workspace_when_snapshot_metadata_missing() {
    let engine = Arc::new(ToolThenFinalEngine {
        requests: Mutex::new(Vec::new()),
    });
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(engine.clone());
    builder.thread_store_factory(Arc::new(MetadataDroppingThreadStoreFactory::default()));
    let runtime =
        Arc::new(Runtime::new(builder.build().unwrap(), RuntimeConfig::default()).unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);
    let workspace_ref = create_workspace_for_current_dir(&client).await;

    let started: ThreadStartResult = request(
        &client,
        "thread/start",
        Some(
            serde_json::to_value(ThreadStartParams {
                selection: None,
                workspace_id: workspace_ref.workspace_id,
                root_id: Some(workspace_ref.root_id),
                model: Some("mock".to_string()),
                model_provider: Some(PROVIDER_MOCK.to_string()),
                reasoning: None,
                cwd: None,
                tool_allowlist: None,
                developer_instructions: None,
                ephemeral: false,
            })
            .unwrap(),
        ),
    )
    .await;

    let turn: TurnStartResult = request(
        &client,
        "turn/start",
        Some(
            serde_json::to_value(TurnStartParams {
                thread_id: started.thread.id,
                input: text_input("hello"),
                prompt: None,
                model_provider: None,
                model: None,
                reasoning: None,
                policy_mode: None,
                task_ledger_required: false,
            })
            .unwrap(),
        ),
    )
    .await;

    assert!(!turn.turn_id.is_empty());
}

#[tokio::test]
async fn turn_start_during_active_tool_call_steers_same_turn_after_tool_result() {
    let engine = Arc::new(ToolThenFinalEngine {
        requests: Mutex::new(Vec::new()),
    });
    let release_tool = Arc::new(Notify::new());
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(engine.clone());
    builder.tool_contributor(Arc::new(BlockingEchoContributor {
        release: Arc::clone(&release_tool),
    }));
    let runtime =
        Arc::new(Runtime::new(builder.build().unwrap(), RuntimeConfig::default()).unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);
    let mut events = client.subscribe_events();
    let workspace_ref = create_workspace_for_current_dir(&client).await;

    let thread: ThreadStartResult = request(
        &client,
        "thread/start",
        Some(
            serde_json::to_value(ThreadStartParams {
                selection: None,
                workspace_id: workspace_ref.workspace_id,
                root_id: Some(workspace_ref.root_id),
                model: None,
                model_provider: None,
                reasoning: None,
                cwd: None,
                tool_allowlist: None,
                developer_instructions: None,
                ephemeral: false,
            })
            .unwrap(),
        ),
    )
    .await;
    let started: TurnStartResult = request(
        &client,
        "turn/start",
        Some(
            serde_json::to_value(TurnStartParams {
                thread_id: thread.thread.id.clone(),
                input: text_input("run the tool"),
                prompt: None,
                model_provider: None,
                model: None,
                reasoning: None,
                policy_mode: None,
                task_ledger_required: false,
            })
            .unwrap(),
        ),
    )
    .await;

    wait_for_event(&mut events, &thread.thread.id, "tool.call_requested").await;

    let steered: TurnStartResult = request(
        &client,
        "turn/start",
        Some(
            serde_json::to_value(TurnStartParams {
                thread_id: thread.thread.id.clone(),
                input: text_input("use this extra constraint"),
                prompt: None,
                model_provider: None,
                model: None,
                reasoning: None,
                policy_mode: None,
                task_ledger_required: false,
            })
            .unwrap(),
        ),
    )
    .await;

    release_tool.notify_waiters();
    wait_for_event(&mut events, &thread.thread.id, "turn.completed").await;

    assert_eq!(
        steered.turn_id, started.turn_id,
        "turn/start submitted during an active turn should steer the active turn"
    );

    let requests = engine.requests.lock().await;
    assert_eq!(
        requests.len(),
        2,
        "steering should continue the same tool loop instead of starting another inference"
    );
    let tool_result_index = requests[1]
        .transcript
        .iter()
        .position(|item| {
            matches!(
                item,
                TranscriptItem::ToolResult(result)
                    if result.id == "call_blocking_echo" && result.result == "from tool"
            )
        })
        .expect("follow-up request should include the tool result");
    let steer_index = requests[1]
        .transcript
        .iter()
        .position(|item| {
            matches!(
                item,
                TranscriptItem::UserMessage(message)
                    if message.text == "use this extra constraint"
            )
        })
        .expect("follow-up request should include the steered user message");
    assert!(
        tool_result_index < steer_index,
        "steer message must be inserted after the tool result: {:?}",
        requests[1].transcript
    );
}

struct TestWorkspaceRef {
    workspace_id: String,
    root_id: String,
}

async fn create_workspace_for_current_dir(client: &LocalAppClient) -> TestWorkspaceRef {
    let cwd = std::env::current_dir().unwrap().display().to_string();
    let result: WorkspaceCreateResult = request(
        client,
        "workspace/create",
        Some(
            serde_json::to_value(WorkspaceCreateParams {
                name: None,
                roots: vec![WorkspaceRootInput {
                    path: cwd.clone(),
                    name: None,
                }],
                default_root_path: Some(cwd),
            })
            .unwrap(),
        ),
    )
    .await;
    TestWorkspaceRef {
        workspace_id: result.workspace.id,
        root_id: result.workspace.default_root_id,
    }
}

async fn request<T: serde::de::DeserializeOwned>(
    client: &LocalAppClient,
    method: &str,
    params: Option<serde_json::Value>,
) -> T {
    let res = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(method)),
            method: method.to_string(),
            params,
        })
        .await;
    assert!(
        res.error.is_none(),
        "RPC error for {method}: {:?}",
        res.error
    );
    serde_json::from_value(res.result.unwrap()).unwrap()
}

async fn wait_for_event(
    events: &mut broadcast::Receiver<roder_api::events::EventEnvelope>,
    thread_id: &str,
    kind: &str,
) {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        if event.kind == kind && event.thread_id.as_deref() == Some(thread_id) {
            return;
        }
    }
}

fn text_input(text: &str) -> Vec<TurnInputItem> {
    vec![TurnInputItem {
        kind: "text".to_string(),
        text: Some(text.to_string()),
        path: None,
        image_url: None,
    }]
}
