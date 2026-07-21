use std::sync::Arc;

use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::lifecycle::{
    TurnCleanupState, TurnLifecycleReason, TurnLifecycleRecord, TurnLifecycleState,
};
use roder_api::thread::ThreadStoreFactory;
use roder_core::fake_provider::FakeInferenceEngine;
use roder_core::{Runtime, RuntimeConfig};
use roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory;

#[tokio::test]
async fn fresh_runtime_reconciles_incomplete_lifecycle_record_to_recovery_needed() {
    let thread_root =
        std::env::temp_dir().join(format!("roder-lifecycle-recovery-{}", uuid::Uuid::new_v4()));
    let factory = Arc::new(JsonlThreadStoreFactory {
        base_path: thread_root.clone(),
    });
    let mut initial_builder = ExtensionRegistryBuilder::new();
    initial_builder.inference_engine(Arc::new(FakeInferenceEngine));
    initial_builder.thread_store_factory(factory.clone());
    let initial =
        Arc::new(Runtime::new(initial_builder.build().unwrap(), RuntimeConfig::default()).unwrap());
    let thread_id = initial
        .create_thread(Some("Interrupted before restart".to_string()))
        .await
        .unwrap()
        .thread_id;
    let record = TurnLifecycleRecord::new(
        thread_id.clone(),
        "unfinished-turn".to_string(),
        TurnLifecycleState::InterruptRequested,
        TurnCleanupState::Requested,
        Some(TurnLifecycleReason::Shutdown),
        time::OffsetDateTime::UNIX_EPOCH,
    );
    factory
        .create()
        .append_extension_state(&thread_id, &record.extension_state().unwrap())
        .await
        .unwrap();
    drop(initial);

    let mut recovery_builder = ExtensionRegistryBuilder::new();
    recovery_builder.inference_engine(Arc::new(FakeInferenceEngine));
    recovery_builder.thread_store_factory(factory);
    let recovery =
        Runtime::new(recovery_builder.build().unwrap(), RuntimeConfig::default()).unwrap();
    recovery.load_thread(&thread_id).await.unwrap().unwrap();

    let lifecycle = recovery.turn_lifecycle_snapshot(&thread_id).await.unwrap();
    assert!(lifecycle.records.iter().any(|record| {
        record.turn_id == "unfinished-turn"
            && record.state == TurnLifecycleState::RecoveryNeeded
            && record.reason == Some(TurnLifecycleReason::RuntimeRestart)
    }));
    assert_eq!(recovery.lifecycle_metrics().restart_reconciliation_count, 1);

    let _ = std::fs::remove_dir_all(thread_root);
}
