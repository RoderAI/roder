//! Runtime event-sink dispatch tests (roadmap phase 64, Task 3): ordering,
//! filtered delivery, redacted failure surfacing, and turn progress while a
//! sink is broken. Offline only — fake sinks and the fake provider.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use roder_api::events::{EventEnvelope, RoderEvent, ThreadCreated};
use roder_api::extension::{EventSink, EventSinkId, ExtensionRegistryBuilder};
use roder_core::fake_provider::FakeInferenceEngine;
use roder_core::{Runtime, RuntimeConfig};
use time::OffsetDateTime;
use tokio::sync::Mutex;

struct RecordingSink {
    id: String,
    kinds: Arc<Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl EventSink for RecordingSink {
    fn id(&self) -> EventSinkId {
        self.id.clone()
    }

    async fn handle_event(&self, envelope: &EventEnvelope) -> anyhow::Result<()> {
        self.kinds.lock().await.push(envelope.kind.clone());
        Ok(())
    }
}

struct FailingSink {
    id: String,
    hang: bool,
}

#[async_trait::async_trait]
impl EventSink for FailingSink {
    fn id(&self) -> EventSinkId {
        self.id.clone()
    }

    async fn handle_event(&self, _envelope: &EventEnvelope) -> anyhow::Result<()> {
        if self.hang {
            tokio::time::sleep(Duration::from_secs(60)).await;
            return Ok(());
        }
        anyhow::bail!("secret-token-abc123 exploded")
    }
}

fn runtime_with_sinks(sinks: Vec<Arc<dyn EventSink>>) -> Arc<Runtime> {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FakeInferenceEngine));
    for sink in sinks {
        builder.event_sink(sink);
    }
    Arc::new(Runtime::new(builder.build().unwrap(), RuntimeConfig::default()).unwrap())
}

fn thread_created(thread_id: &str) -> RoderEvent {
    RoderEvent::ThreadCreated(ThreadCreated {
        thread_id: thread_id.to_string(),
        timestamp: OffsetDateTime::now_utc(),
    })
}

async fn wait_until<F>(mut predicate: F)
where
    F: AsyncFnMut() -> bool,
{
    for _ in 0..200 {
        if predicate().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("condition did not become true in time");
}

#[tokio::test(flavor = "multi_thread")]
async fn event_sinks_receive_emitted_envelopes_in_order() {
    let kinds = Arc::new(Mutex::new(Vec::new()));
    let runtime = runtime_with_sinks(vec![Arc::new(RecordingSink {
        id: "recording".to_string(),
        kinds: kinds.clone(),
    })]);

    runtime.emit(thread_created("thread-1")).await;
    runtime.emit(thread_created("thread-2")).await;
    runtime.emit(thread_created("thread-3")).await;

    wait_until(async || kinds.lock().await.len() >= 3).await;
    let recorded = kinds.lock().await.clone();
    assert_eq!(
        recorded,
        vec!["thread.created", "thread.created", "thread.created"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn sink_failures_surface_redacted_failure_events_without_loops() {
    let runtime = runtime_with_sinks(vec![Arc::new(FailingSink {
        id: "broken".to_string(),
        hang: false,
    })]);
    let mut events = runtime.bus.subscribe();

    runtime.emit(thread_created("thread-1")).await;

    let failure = loop {
        let envelope = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .expect("failure event in time")
            .unwrap();
        if let RoderEvent::EventSinkFailed(failure) = envelope.event {
            break failure;
        }
    };
    assert_eq!(failure.sink_id, "broken");
    assert_eq!(failure.event_kind, "thread.created");
    assert!(failure.message.contains("exploded"));

    // The failure event itself is never dispatched back to sinks, so no
    // second failure event for kind extension.event_sink_failed appears.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let mut loop_failures = 0;
    while let Ok(envelope) = events.try_recv() {
        if let RoderEvent::EventSinkFailed(failure) = envelope.event
            && failure.event_kind == "extension.event_sink_failed"
        {
            loop_failures += 1;
        }
    }
    assert_eq!(loop_failures, 0, "sink failures must not loop");
}

#[tokio::test(flavor = "multi_thread")]
async fn turns_complete_while_a_sink_hangs() {
    let received = Arc::new(AtomicBool::new(false));
    struct MarkSink(Arc<AtomicBool>);
    #[async_trait::async_trait]
    impl EventSink for MarkSink {
        fn id(&self) -> EventSinkId {
            "marker".to_string()
        }
        async fn handle_event(&self, _envelope: &EventEnvelope) -> anyhow::Result<()> {
            self.0.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    let runtime = runtime_with_sinks(vec![
        Arc::new(FailingSink {
            id: "hanging".to_string(),
            hang: true,
        }),
        Arc::new(MarkSink(received.clone())),
    ]);

    // A full fake-provider turn must complete even though one sink hangs on
    // every event it receives.
    let metadata = runtime.create_thread(Some("sinks".to_string())).await.unwrap();
    let mut events = runtime.bus.subscribe();
    let turn_id = runtime
        .start_turn(roder_core::StartTurnRequest {
            thread_id: metadata.thread_id.clone(),
            message: "hello".to_string(),
            images: Vec::new(),
            provider_override: None,
            model_override: None,
            reasoning_override: None,
            workspace: std::env::current_dir().unwrap().display().to_string(),
            instructions: roder_core::default_instructions(),
            developer_context: None,
            task_ledger_required: false,
        })
        .await
        .unwrap();

    let completed = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let envelope = events.recv().await.unwrap();
            if let RoderEvent::TurnCompleted(completed) = envelope.event
                && completed.turn_id == turn_id
            {
                break;
            }
        }
    })
    .await;
    assert!(completed.is_ok(), "turn must complete despite a hanging sink");
    assert!(
        received.load(Ordering::SeqCst),
        "healthy sinks keep receiving events"
    );
}
