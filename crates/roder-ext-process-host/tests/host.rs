//! Offline tests for the process-extension host (roadmap phase 64, Task 2)
//! against the Python fake child in `tests/fixtures/fake_child.py`. No
//! provider credentials or network access.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use futures::StreamExt;
use roder_api::events::{EventEnvelope, EventSource, RoderEvent, RuntimeStarted};
use roder_api::extension::{EventSink, RoderExtension};
use roder_api::inference::{
    AgentInferenceRequest, InferenceEngine, InferenceEvent, InferenceProviderContext,
    InferenceTurnContext, InstructionBundle, ModelSelection, OutputConfig, ReasoningConfig,
    RuntimeHints,
};
use roder_api::process_extension::{ProcessEventFilter, ProcessExtensionConfig};
use roder_api::tools::ToolChoice;
use roder_ext_process_host::{
    ProcessEventSink, ProcessHost, ProcessHostExtension, ProcessInferenceEngine,
    load_process_extension,
};
use time::OffsetDateTime;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn fake_config(extra_env: BTreeMap<String, String>) -> ProcessExtensionConfig {
    let manifest = fixtures_dir().join("fake-extension.toml");
    let mut env = BTreeMap::from([(
        "FAKE_CHILD_MANIFEST".to_string(),
        manifest.display().to_string(),
    )]);
    env.extend(extra_env);
    ProcessExtensionConfig {
        id: "fake-child".to_string(),
        enabled: true,
        manifest: manifest.display().to_string(),
        command: "python3".to_string(),
        args: vec![fixtures_dir().join("fake_child.py").display().to_string()],
        cwd: None,
        env,
        startup_timeout_ms: 10_000,
        event_filter: ProcessEventFilter {
            kinds: vec!["runtime.".to_string(), "turn.".to_string()],
        },
    }
}

fn host_for(config: ProcessExtensionConfig) -> Arc<ProcessHost> {
    let loaded = load_process_extension(config, &fixtures_dir()).unwrap();
    Arc::new(ProcessHost::new(loaded))
}

fn sample_request() -> AgentInferenceRequest {
    AgentInferenceRequest {
        model: ModelSelection {
            provider: "fake-process-engine".to_string(),
            model: "fake-model".to_string(),
        },
        instructions: InstructionBundle {
            system: Some("offline host test".to_string()),
            developer: None,
        },
        transcript: vec![roder_api::transcript::TranscriptItem::UserMessage(
            roder_api::transcript::UserMessage::text("hello child"),
        )],
        tools: Vec::new(),
        tool_choice: ToolChoice::Auto,
        reasoning: ReasoningConfig {
            enabled: false,
            level: None,
        },
        output: OutputConfig {
            max_tokens: Some(32),
            temperature: None,
            top_p: None,
            response_format: None,
        },
        runtime: RuntimeHints::default(),
        metadata: serde_json::json!({}),
    }
}

fn envelope(kind: &str) -> EventEnvelope {
    EventEnvelope {
        event_id: format!("event-{kind}"),
        seq: 1,
        timestamp: OffsetDateTime::UNIX_EPOCH,
        source: EventSource::Core,
        kind: kind.to_string(),
        thread_id: None,
        turn_id: None,
        event: RoderEvent::RuntimeStarted(RuntimeStarted {
            timestamp: OffsetDateTime::UNIX_EPOCH,
        }),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn fake_child_registers_lists_models_streams_and_shuts_down() {
    let host = host_for(fake_config(BTreeMap::new()));
    let engine = ProcessInferenceEngine::new(host.clone(), "fake-process-engine".to_string());

    // Models come from the child over JSON-RPC.
    let models = engine
        .list_models(InferenceProviderContext {
            provider_id: "fake-process-engine",
        })
        .await
        .unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].id, "fake-model");
    assert_eq!(models[0].context_window, Some(4096));

    // The child receives a canonical event before the turn and reflects it
    // in provider metadata.
    let sink = ProcessEventSink::new(host.clone(), "fake-process-events".to_string());
    sink.handle_event(&envelope("turn.started")).await.unwrap();

    let stream = engine
        .stream_turn(
            InferenceTurnContext {
                thread_id: "thread-1",
                turn_id: "turn-1",
                tool_executor: None,
            },
            sample_request(),
        )
        .await
        .unwrap();
    let events: Vec<InferenceEvent> = stream.map(|event| event.unwrap()).collect().await;

    let text: String = events
        .iter()
        .filter_map(|event| match event {
            InferenceEvent::MessageDelta(delta) => Some(delta.text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "hello from the fake child");

    let metadata = events
        .iter()
        .find_map(|event| match event {
            InferenceEvent::ProviderMetadata(value) => Some(value.clone()),
            _ => None,
        })
        .expect("provider metadata event");
    assert_eq!(metadata["transcript_items"], 1);
    assert_eq!(
        metadata["events_seen"], 1,
        "the child must have observed the forwarded turn.started event"
    );

    let usage = events.iter().find_map(|event| match event {
        InferenceEvent::Usage(usage) => Some(usage.clone()),
        _ => None,
    });
    assert_eq!(usage.map(|usage| usage.total_tokens), Some(11));
    assert!(matches!(events.last(), Some(InferenceEvent::Completed(_))));

    // The child reported the handled events through an extension-owned event.
    let extension_events = host.drain_extension_events().await;
    assert_eq!(extension_events.len(), 1);
    assert_eq!(extension_events[0].event_kind, "fake.events_observed");
    assert_eq!(
        extension_events[0].payload["kinds"],
        serde_json::json!(["turn.started"])
    );

    host.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn event_sink_filters_unmatched_kinds_without_spawning_work() {
    let host = host_for(fake_config(BTreeMap::new()));
    let sink = ProcessEventSink::new(host.clone(), "fake-process-events".to_string());

    // tool.* is not in the filter: handled as a no-op.
    sink.handle_event(&envelope("tool.call_started"))
        .await
        .unwrap();
    sink.handle_event(&envelope("turn.started")).await.unwrap();

    let engine = ProcessInferenceEngine::new(host.clone(), "fake-process-engine".to_string());
    let stream = engine
        .stream_turn(
            InferenceTurnContext {
                thread_id: "thread-1",
                turn_id: "turn-1",
                tool_executor: None,
            },
            sample_request(),
        )
        .await
        .unwrap();
    let events: Vec<InferenceEvent> = stream.map(|event| event.unwrap()).collect().await;
    let metadata = events
        .iter()
        .find_map(|event| match event {
            InferenceEvent::ProviderMetadata(value) => Some(value.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        metadata["events_seen"], 1,
        "only the filtered-in event may reach the child"
    );
    host.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn initialize_echo_mismatches_fail_closed() {
    // Wrong extension id.
    let host = host_for(fake_config(BTreeMap::from([(
        "FAKE_CHILD_ID".to_string(),
        "imposter".to_string(),
    )])));
    let engine = ProcessInferenceEngine::new(host.clone(), "fake-process-engine".to_string());
    let error = engine
        .list_models(InferenceProviderContext {
            provider_id: "fake-process-engine",
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("echoed id"), "{error}");
    host.shutdown().await;

    // Manifest checksum drift.
    let host = host_for(fake_config(BTreeMap::from([(
        "FAKE_CHILD_BAD_CHECKSUM".to_string(),
        "deadbeefdeadbeef".to_string(),
    )])));
    let engine = ProcessInferenceEngine::new(host.clone(), "fake-process-engine".to_string());
    let error = engine
        .list_models(InferenceProviderContext {
            provider_id: "fake-process-engine",
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("different manifest"), "{error}");
    host.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn extension_installs_manifest_backed_services_into_registry() {
    let loaded = load_process_extension(fake_config(BTreeMap::new()), &fixtures_dir()).unwrap();
    let extension = ProcessHostExtension::new(loaded);
    let host = extension.host();
    let manifest = extension.manifest();
    assert_eq!(manifest.id, "roder-ext-fake-child");
    assert_eq!(manifest.provides.len(), 2);

    let mut builder = roder_api::extension::ExtensionRegistryBuilder::new();
    builder.install(extension).unwrap();
    let registry = builder.build().unwrap();
    assert!(registry.inference_engine("fake-process-engine").is_some());
    assert_eq!(registry.event_sinks.len(), 1);
    assert_eq!(registry.event_sinks[0].id(), "fake-process-events");
    host.shutdown().await;
}
