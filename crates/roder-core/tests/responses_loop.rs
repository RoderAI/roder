use futures::{StreamExt, stream};
use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::inference::*;
use roder_core::{Runtime, RuntimeConfig, StartTurnRequest, default_instructions};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

struct FixtureEngine {
    calls: AtomicUsize,
    pending_first: bool,
    always_fail: bool,
}
#[async_trait::async_trait]
impl InferenceEngine for FixtureEngine {
    fn id(&self) -> String {
        "mock".into()
    }
    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities::text_only()
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
        _: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if call == 0 || self.always_fail {
            if self.pending_first {
                return Ok(Box::pin(
                    stream::iter(vec![Ok(InferenceEvent::MessageDelta(MessageDelta {
                        text: "working".into(),
                        phase: Some("commentary".into()),
                    }))])
                    .chain(stream::pending()),
                ));
            }
            return Ok(Box::pin(stream::iter(vec![Err(anyhow::anyhow!(
                "stream closed before response.completed"
            ))])));
        }
        Ok(Box::pin(stream::iter(vec![Ok(InferenceEvent::Completed(
            CompletionMetadata {
                stop_reason: Some("completed".into()),
                provider_response_id: None,
            },
        ))])))
    }
}
fn request() -> StartTurnRequest {
    StartTurnRequest {
        thread_id: "audit".into(),
        message: "go".into(),
        images: vec![],
        provider_override: None,
        model_override: None,
        reasoning_override: None,
        workspace: "/tmp".into(),
        instructions: default_instructions(),
        developer_context: None,
        task_ledger_required: false,
        service_tier_override: None,
    }
}
fn runtime(engine: Arc<FixtureEngine>) -> Arc<Runtime> {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(engine);
    let mut config = RuntimeConfig::default();
    config.reliability.provider_retry_initial_backoff_ms = 0;
    Arc::new(Runtime::new(builder.build().unwrap(), config).unwrap())
}
#[tokio::test]
async fn audit_interactive_stream_disconnect_retries() {
    let engine = Arc::new(FixtureEngine {
        calls: AtomicUsize::new(0),
        pending_first: false,
        always_fail: false,
    });
    let runtime = runtime(engine.clone());
    let mut events = runtime.subscribe_events();
    runtime.start_turn(request()).await.unwrap();
    loop {
        let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        if event.kind == "turn.failed" || event.kind == "turn.completed" {
            break;
        }
    }
    assert_eq!(
        engine.calls.load(Ordering::SeqCst),
        2,
        "interactive transient stream failure never retried"
    );
}
#[tokio::test]
async fn audit_stream_failure_emits_one_terminal_failure() {
    let runtime = runtime(Arc::new(FixtureEngine {
        calls: AtomicUsize::new(0),
        pending_first: false,
        always_fail: true,
    }));
    let mut events = runtime.subscribe_events();
    runtime.start_turn(request()).await.unwrap();
    let mut failures = 0;
    while let Ok(Ok(event)) = tokio::time::timeout(Duration::from_millis(150), events.recv()).await
    {
        if event.kind == "turn.failed" {
            failures += 1;
        }
    }
    assert_eq!(
        failures, 1,
        "one transport failure must produce one turn.failed"
    );
}
#[tokio::test]
async fn audit_steer_preempts_unfinished_response() {
    let engine = Arc::new(FixtureEngine {
        calls: AtomicUsize::new(0),
        pending_first: true,
        always_fail: false,
    });
    let runtime = runtime(engine.clone());
    let mut events = runtime.subscribe_events();
    let turn = runtime.start_turn(request()).await.unwrap();
    loop {
        let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        if event.kind == "inference.event_received" {
            break;
        }
    }
    runtime
        .steer_turn("audit".into(), turn, "new constraint".into(), vec![])
        .await
        .unwrap();
    let reached_model = tokio::time::timeout(Duration::from_millis(100), async {
        while engine.calls.load(Ordering::SeqCst) < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .is_ok();
    assert!(
        reached_model,
        "steer queued behind an unfinished model response"
    );
}
