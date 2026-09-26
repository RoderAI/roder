use futures::stream;
use roder_api::{extension::ExtensionRegistryBuilder, inference::*, transcript::TranscriptItem};
use roder_core::{Runtime, RuntimeConfig, StartTurnRequest, default_instructions};
use roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory;
use serde_json::json;
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

#[derive(Default)]
struct Engine {
    entered: Notify,
    release: Notify,
    requests: Mutex<Vec<AgentInferenceRequest>>,
}
#[async_trait::async_trait]
impl InferenceEngine for Engine {
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
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        self.requests.lock().unwrap().push(request);
        Ok(Box::pin(stream::iter([Ok(InferenceEvent::Completed(
            CompletionMetadata {
                stop_reason: Some("completed".into()),
                provider_response_id: None,
            },
        ))])))
    }
    async fn compact_turn(
        &self,
        _: InferenceTurnContext<'_>,
        _: AgentInferenceRequest,
    ) -> anyhow::Result<Option<InferenceEventStream>> {
        self.entered.notify_one();
        self.release.notified().await;
        Ok(Some(Box::pin(stream::iter([
            Ok(InferenceEvent::ProviderMetadata(
                json!({"output":[{"type":"compaction","id":"boundary","encrypted_content":"opaque"}],"compacted_input":[{"type":"compaction","id":"boundary","encrypted_content":"opaque"}]}),
            )),
            Ok(InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some("completed".into()),
                provider_response_id: None,
            })),
        ]))))
    }
}
fn request(thread: String, workspace: String) -> StartTurnRequest {
    StartTurnRequest {
        thread_id: thread,
        message: "fresh input".into(),
        images: vec![],
        provider_override: None,
        model_override: None,
        reasoning_override: None,
        workspace,
        instructions: default_instructions(),
        developer_context: None,
        task_ledger_required: false,
        service_tier_override: None,
    }
}
#[tokio::test]
async fn manual_compaction_serializes_new_input_for_its_task_without_blocking_other_tasks() {
    let dir = tempfile::tempdir().unwrap();
    let engine = Arc::new(Engine::default());
    let mut registry = ExtensionRegistryBuilder::new();
    registry.inference_engine(engine.clone());
    registry.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
        base_path: dir.path().into(),
    }));
    let runtime =
        Arc::new(Runtime::new(registry.build().unwrap(), RuntimeConfig::default()).unwrap());
    let task = runtime.create_thread(None).await.unwrap().thread_id;
    let other = runtime.create_thread(None).await.unwrap().thread_id;
    let workspace = dir.path().to_string_lossy().into_owned();
    let mut events = runtime.subscribe_events();
    let mut seed = request(task.clone(), workspace.clone());
    seed.message = "seed history".into();
    let seed_turn = runtime.start_turn(seed).await.unwrap();
    loop {
        let event = tokio::time::timeout(std::time::Duration::from_secs(3), events.recv())
            .await
            .unwrap()
            .unwrap();
        if event.turn_id.as_deref() == Some(&seed_turn) && event.kind == "turn.completed" {
            break;
        }
    }
    while runtime.active_turn_for_thread(&task).await.is_some() {
        tokio::task::yield_now().await;
    }
    let compact = {
        let runtime = runtime.clone();
        let task = task.clone();
        tokio::spawn(async move {
            runtime
                .force_compact_thread(&task, &"compact".into(), None)
                .await
        })
    };
    tokio::time::timeout(std::time::Duration::from_secs(3), engine.entered.notified())
        .await
        .unwrap();
    let mut queued = {
        let runtime = runtime.clone();
        let request = request(task.clone(), workspace.clone());
        tokio::spawn(async move { runtime.start_turn(request).await })
    };
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), &mut queued)
            .await
            .is_err()
    );
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        runtime.start_turn(request(other, workspace)),
    )
    .await
    .expect("other task was blocked by compaction")
    .unwrap();
    engine.release.notify_one();
    assert!(compact.await.unwrap().unwrap().compacted);
    let turn = queued.await.unwrap().unwrap();
    loop {
        let event = tokio::time::timeout(std::time::Duration::from_secs(3), events.recv())
            .await
            .unwrap()
            .unwrap();
        if event.turn_id.as_deref() == Some(&turn) && event.kind == "turn.completed" {
            break;
        }
    }
    let requests = engine.requests.lock().unwrap();
    let resumed=requests.iter().find(|request|request.transcript.iter().any(|item|matches!(item,TranscriptItem::ProviderMetadata(metadata) if metadata.get("compacted_input").is_some()))).expect("new input sampled before compaction was committed");
    assert!(resumed.transcript.iter().any(
        |item| matches!(item,TranscriptItem::UserMessage(message) if message.text=="fresh input")
    ));
}
