use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::stream;
use roder_api::catalog::{PROVIDER_MOCK, models_for_provider};
use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::inference::{
    AgentInferenceRequest, InferenceCapabilities, InferenceEngine, InferenceEvent,
    InferenceEventStream, InferenceProviderContext, InferenceTurnContext, ModelDescriptor,
    ProviderTurnCleanup,
};
use roder_api::lifecycle::{
    TurnCleanupOwnership, TurnCleanupState, TurnLifecycleReason, TurnLifecycleState,
};
use roder_core::{Runtime, RuntimeConfig, StartTurnRequest};
use roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory;
use tokio::sync::oneshot;

struct CleanupEngine {
    cleanup: Arc<dyn ProviderTurnCleanup>,
    started: Mutex<Option<oneshot::Sender<()>>>,
}

#[async_trait::async_trait]
impl InferenceEngine for CleanupEngine {
    fn id(&self) -> String {
        PROVIDER_MOCK.to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities::text_only()
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(models_for_provider(PROVIDER_MOCK, true))
    }

    async fn stream_turn(
        &self,
        ctx: InferenceTurnContext<'_>,
        _request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        ctx.tool_executor
            .as_ref()
            .expect("runtime supplies a tool executor")
            .register_provider_cleanup(Arc::clone(&self.cleanup));
        if let Some(started) = self.started.lock().unwrap().take() {
            let _ = started.send(());
        }
        Ok(Box::pin(stream::pending::<anyhow::Result<InferenceEvent>>()))
    }
}

struct ErrorCleanup;

#[async_trait::async_trait]
impl ProviderTurnCleanup for ErrorCleanup {
    fn ownership(&self) -> TurnCleanupOwnership {
        TurnCleanupOwnership::ProviderCleanupPending
    }

    async fn wait_for_cleanup(&self) -> anyhow::Result<()> {
        anyhow::bail!("synthetic provider cleanup failure")
    }
}

struct NeverCompletesCleanup;

#[async_trait::async_trait]
impl ProviderTurnCleanup for NeverCompletesCleanup {
    fn ownership(&self) -> TurnCleanupOwnership {
        TurnCleanupOwnership::ProviderCleanupPending
    }

    async fn wait_for_cleanup(&self) -> anyhow::Result<()> {
        std::future::pending::<anyhow::Result<()>>().await
    }
}

async fn start_cleanup_turn(
    cleanup: Arc<dyn ProviderTurnCleanup>,
) -> (
    Arc<Runtime>,
    String,
    String,
    std::path::PathBuf,
    oneshot::Receiver<()>,
) {
    let thread_root = std::env::temp_dir().join(format!(
        "roder-provider-cleanup-lifecycle-{}",
        uuid::Uuid::new_v4()
    ));
    let (started_tx, started_rx) = oneshot::channel();
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(CleanupEngine {
        cleanup,
        started: Mutex::new(Some(started_tx)),
    }));
    builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
        base_path: thread_root.clone(),
    }));
    let runtime = Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                default_provider: PROVIDER_MOCK.to_string(),
                default_model: "mock".to_string(),
                workspace: Some(thread_root.display().to_string()),
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    );
    let thread_id = runtime
        .create_thread(Some("Provider cleanup lifecycle".to_string()))
        .await
        .unwrap()
        .thread_id;
    let turn_id = runtime
        .start_turn(StartTurnRequest {
            thread_id: thread_id.clone(),
            message: "wait until interrupted".to_string(),
            images: Vec::new(),
            provider_override: None,
            model_override: None,
            reasoning_override: None,
            workspace: thread_root.display().to_string(),
            instructions: Default::default(),
            developer_context: None,
            task_ledger_required: false,
        })
        .await
        .unwrap();
    (runtime, thread_id, turn_id, thread_root, started_rx)
}

async fn interrupted_record(
    runtime: &Runtime,
    thread_id: &str,
    turn_id: &str,
    timeout: Duration,
) -> roder_api::lifecycle::TurnLifecycleRecord {
    tokio::time::timeout(timeout, async {
        loop {
            let snapshot = runtime
                .turn_lifecycle_snapshot(&thread_id.to_string())
                .await
                .unwrap();
            if let Some(record) = snapshot.records.into_iter().find(|record| {
                record.turn_id == turn_id && record.state == TurnLifecycleState::Interrupted
            }) {
                return record;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("interrupted lifecycle record")
}

#[tokio::test]
async fn provider_cleanup_error_is_persisted_as_unknown_not_confirmed() {
    let (runtime, thread_id, turn_id, thread_root, started) =
        start_cleanup_turn(Arc::new(ErrorCleanup)).await;
    started.await.expect("provider stream started");

    runtime
        .interrupt_turn(thread_id.clone(), turn_id.clone())
        .await
        .unwrap();
    let record = interrupted_record(&runtime, &thread_id, &turn_id, Duration::from_secs(1)).await;

    assert_eq!(record.cleanup, TurnCleanupState::Unknown);
    assert_eq!(
        record.ownership,
        TurnCleanupOwnership::ProviderCleanupPending
    );
    assert_eq!(record.reason, Some(TurnLifecycleReason::UserInterrupt));
    let metrics = runtime.lifecycle_metrics();
    assert_eq!(metrics.provider_cleanup_confirmed_count, 0);
    assert_eq!(metrics.provider_cleanup_unknown_count, 1);
    assert_eq!(metrics.provider_cleanup_timed_out_count, 0);

    let _ = std::fs::remove_dir_all(thread_root);
}

#[tokio::test]
async fn provider_cleanup_timeout_is_persisted_as_timed_out_not_confirmed() {
    let (runtime, thread_id, turn_id, thread_root, started) =
        start_cleanup_turn(Arc::new(NeverCompletesCleanup)).await;
    started.await.expect("provider stream started");

    runtime
        .interrupt_turn(thread_id.clone(), turn_id.clone())
        .await
        .unwrap();
    let record = interrupted_record(&runtime, &thread_id, &turn_id, Duration::from_secs(7)).await;

    assert_eq!(record.cleanup, TurnCleanupState::TimedOut);
    assert_eq!(
        record.ownership,
        TurnCleanupOwnership::ProviderCleanupPending
    );
    assert_eq!(record.reason, Some(TurnLifecycleReason::UserInterrupt));
    let metrics = runtime.lifecycle_metrics();
    assert_eq!(metrics.provider_cleanup_confirmed_count, 0);
    assert_eq!(metrics.provider_cleanup_unknown_count, 0);
    assert_eq!(metrics.provider_cleanup_timed_out_count, 1);

    let _ = std::fs::remove_dir_all(thread_root);
}
