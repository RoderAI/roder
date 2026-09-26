use roder_api::extension::{ExtensionRegistryBuilder, ToolProviderId};
use roder_api::inference::*;
use roder_api::tools::*;
use roder_api::transcript::TranscriptItem;
use roder_core::{Runtime, RuntimeConfig, StartTurnRequest, default_instructions};
use roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory;
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::time::Duration;
use tokio::sync::Notify;

struct ProbeTool {
    ran: Arc<Notify>,
    executions: Arc<AtomicUsize>,
    model_completed: Arc<AtomicBool>,
}
#[async_trait::async_trait]
impl ToolExecutor for ProbeTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "probe".into(),
            description: "read-only probe".into(),
            parameters: json!({"type":"object","properties":{},"additionalProperties":false}),
        }
    }
    fn supports_eager_execution(&self) -> bool {
        true
    }
    async fn execute(&self, _: ToolExecutionContext, call: ToolCall) -> anyhow::Result<ToolResult> {
        assert!(
            !self.model_completed.load(Ordering::SeqCst),
            "tool waited for response.completed"
        );
        self.executions.fetch_add(1, Ordering::SeqCst);
        self.ran.notify_one();
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text: "read result".into(),
            data: json!({}),
            is_error: false,
        })
    }
}
struct Contributor(Arc<ProbeTool>);
impl ToolContributor for Contributor {
    fn id(&self) -> ToolProviderId {
        "probe".into()
    }
    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        registry.register(self.0.clone())
    }
}
struct ProbeEngine {
    ran: Arc<Notify>,
    calls: AtomicUsize,
    model_completed: Arc<AtomicBool>,
    executions: Arc<AtomicUsize>,
}
#[async_trait::async_trait]
impl InferenceEngine for ProbeEngine {
    fn id(&self) -> String {
        "mock".into()
    }
    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities::coding_agent_default()
    }
    async fn list_models(
        &self,
        _: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(vec![])
    }
    async fn stream_turn(
        &self,
        _: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            let ran = self.ran.clone();
            let completed = self.model_completed.clone();
            return Ok(Box::pin(async_stream_fixture(
                ran,
                completed,
                self.executions.clone(),
            )));
        }
        assert_eq!(
            request
                .transcript
                .iter()
                .filter(|item| matches!(item,TranscriptItem::ToolCall(call) if call.id.starts_with("probe")))
                .count(),
            2
        );
        assert_eq!(request.transcript.iter().filter(|item| matches!(item,TranscriptItem::ToolResult(result) if result.id.starts_with("probe"))).count(),2);
        Ok(Box::pin(futures::stream::iter(vec![Ok(
            InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some("completed".into()),
                provider_response_id: None,
            }),
        )])))
    }
}
fn async_stream_fixture(
    ran: Arc<Notify>,
    completed: Arc<AtomicBool>,
    executions: Arc<AtomicUsize>,
) -> impl futures::Stream<Item = anyhow::Result<InferenceEvent>> + Send {
    use futures::{StreamExt, stream};
    stream::iter(["probe1", "probe2"].map(|id| {
        Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
            id: id.into(),
            name: "probe".into(),
            arguments: "{}".into(),
        }))
    }))
    .chain(stream::once(async move {
        tokio::time::timeout(Duration::from_secs(2), async {
            while executions.load(Ordering::SeqCst) < 2 {
                ran.notified().await;
            }
        })
        .await
        .expect("tool executes while sampling is unfinished");
        completed.store(true, Ordering::SeqCst);
        Ok(InferenceEvent::Completed(CompletionMetadata {
            stop_reason: Some("completed".into()),
            provider_response_id: None,
        }))
    }))
}

#[tokio::test]
async fn concurrent_eager_reads_run_before_response_completed_and_replay_once() {
    let directory = tempfile::tempdir().unwrap();
    let ran = Arc::new(Notify::new());
    let executions = Arc::new(AtomicUsize::new(0));
    let model_completed = Arc::new(AtomicBool::new(false));
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(ProbeEngine {
        ran: ran.clone(),
        calls: AtomicUsize::new(0),
        model_completed: model_completed.clone(),
        executions: executions.clone(),
    }));
    builder.tool_contributor(Arc::new(Contributor(Arc::new(ProbeTool {
        ran,
        executions: executions.clone(),
        model_completed,
    }))));
    builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
        base_path: directory.path().to_path_buf(),
    }));
    let mut config = RuntimeConfig::default();
    config.model_parallel_tool_calls.insert("mock".into(), true);
    config.tool_allowlist = vec!["probe".into()];
    config.policy_mode = roder_api::policy_mode::PolicyMode::Bypass;
    let runtime = Arc::new(Runtime::new(builder.build().unwrap(), config).unwrap());
    let thread = runtime.create_thread(None).await.unwrap().thread_id;
    let mut events = runtime.subscribe_events();
    runtime
        .start_turn(StartTurnRequest {
            thread_id: thread,
            message: "go".into(),
            images: vec![],
            provider_override: None,
            model_override: None,
            reasoning_override: None,
            workspace: directory.path().to_string_lossy().into(),
            instructions: default_instructions(),
            developer_context: None,
            task_ledger_required: false,
            service_tier_override: None,
        })
        .await
        .unwrap();
    loop {
        let event = tokio::time::timeout(Duration::from_secs(3), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert_ne!(event.kind, "turn.failed", "{event:?}");
        if event.kind == "turn.completed" {
            break;
        }
    }
    assert_eq!(executions.load(Ordering::SeqCst), 2);
}
