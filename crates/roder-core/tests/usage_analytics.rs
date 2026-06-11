//! Runtime integration proof for local usage analytics (roadmap phase 73,
//! Task 2): the analytics extension records a real fake-provider turn
//! passively through the bounded event-sink dispatch, and disabling
//! analytics (not installing the extension) leaves runtime behavior
//! unchanged. Offline only.

use std::sync::Arc;
use std::time::Duration;

use roder_api::events::RoderEvent;
use roder_api::extension::ExtensionRegistryBuilder;
use roder_core::fake_provider::FakeInferenceEngine;
use roder_core::{Runtime, RuntimeConfig, StartTurnRequest};
use roder_usage_analytics::{
    AnalyticsStore, StatsFilter, UsageAnalyticsExtension, WorkspaceLabelMode,
};

fn temp_dir(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "roder-core-analytics-{label}-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

async fn run_fake_turn(runtime: &Arc<Runtime>) -> (String, String) {
    let metadata = runtime
        .create_thread(Some("analytics".to_string()))
        .await
        .unwrap();
    let mut events = runtime.bus.subscribe();
    let turn_id = runtime
        .start_turn(StartTurnRequest {
            thread_id: metadata.thread_id.clone(),
            message: "hello analytics".to_string(),
            images: Vec::new(),
            provider_override: None,
            model_override: None,
            reasoning_override: None,
            workspace: std::env::current_dir().unwrap().display().to_string(),
            instructions: roder_core::default_instructions(),
            task_ledger_required: false,
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let envelope = events.recv().await.unwrap();
            if let RoderEvent::TurnCompleted(completed) = envelope.event
                && completed.turn_id == turn_id
            {
                break;
            }
        }
    })
    .await
    .expect("turn completes");
    (metadata.thread_id, turn_id)
}

#[tokio::test(flavor = "multi_thread")]
async fn analytics_extension_records_a_real_fake_provider_turn() {
    let data_dir = temp_dir("enabled");
    let store = Arc::new(
        AnalyticsStore::open(
            &AnalyticsStore::default_path(&data_dir),
            WorkspaceLabelMode::FullPath,
        )
        .unwrap(),
    );

    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FakeInferenceEngine));
    builder
        .install(UsageAnalyticsExtension::new(store.clone()))
        .unwrap();
    let runtime =
        Arc::new(Runtime::new(builder.build().unwrap(), RuntimeConfig::default()).unwrap());

    let (thread_id, _turn_id) = run_fake_turn(&runtime).await;

    // Dispatch is async; wait for the terminal turn projection to land.
    let filter = StatsFilter {
        thread_id: Some(thread_id),
        ..StatsFilter::default()
    };
    let mut summary = store.usage_summary(&filter).unwrap();
    for _ in 0..200 {
        if summary.completed_turn_count >= 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        summary = store.usage_summary(&filter).unwrap();
    }
    let counts = store.counts().unwrap();
    assert!(counts.turns >= 1, "turn row recorded: {counts:?}");
    assert!(counts.sessions >= 1, "session row recorded: {counts:?}");
    assert_eq!(summary.turn_count, 1);
    assert_eq!(summary.completed_turn_count, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_behaves_identically_without_the_analytics_extension() {
    let data_dir = temp_dir("disabled");
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FakeInferenceEngine));
    let runtime =
        Arc::new(Runtime::new(builder.build().unwrap(), RuntimeConfig::default()).unwrap());

    // The turn completes exactly as before, and no analytics database is
    // created anywhere under the data dir.
    let _ = run_fake_turn(&runtime).await;
    assert!(
        !AnalyticsStore::default_path(&data_dir).exists(),
        "no analytics artifacts without the extension"
    );
}
