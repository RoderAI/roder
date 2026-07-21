//! Offline tests for the process-extension host (roadmap phase 64 Task 2
//! and phase 93 Task 5) against the Python fake child in
//! `tests/fixtures/fake_child.py`. No provider credentials or network
//! access.

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
use roder_api::policy_mode::PolicyMode;
use roder_api::process_extension::{ProcessEventFilter, ProcessExtensionConfig};
use roder_api::subagents::SubagentDispatcher as _;
use roder_api::tasks::TaskExecutor as _;
use roder_api::tools::{ToolCall, ToolChoice, ToolExecutionContext, ToolRegistry};
use roder_ext_process_host::{
    ProcessEventSink, ProcessHost, ProcessHostExtension, ProcessInferenceEngine,
    ProcessSubagentDispatcher, ProcessTaskExecutor, load_process_extension,
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
            developer_context: None,
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
    assert_eq!(manifest.provides.len(), 5);

    let mut builder = roder_api::extension::ExtensionRegistryBuilder::new();
    builder.install(extension).unwrap();
    let registry = builder.build().unwrap();
    assert!(registry.inference_engine("fake-process-engine").is_some());
    assert_eq!(registry.event_sinks.len(), 1);
    assert_eq!(registry.event_sinks[0].id(), "fake-process-events");
    assert!(
        registry
            .subagent_dispatcher("fake-process-dispatcher")
            .is_some()
    );
    assert_eq!(registry.task_executors.len(), 1);
    assert_eq!(registry.task_executors[0].id(), "fake-process-task");
    assert_eq!(registry.tools.len(), 1);
    assert_eq!(registry.tools[0].id(), "fake-process-tools");
    host.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn subagent_dispatcher_streams_status_and_returns_result() {
    let host = host_for(fake_config(BTreeMap::new()));
    let dispatcher =
        ProcessSubagentDispatcher::new(host.clone(), "fake-process-dispatcher".to_string());

    // Definitions come from the child and are cached for the sync accessor.
    let definitions = dispatcher.fetch_definitions().await.unwrap();
    assert_eq!(definitions.len(), 1);
    assert_eq!(definitions[0].agent_type, "fake-remote");
    assert_eq!(
        roder_api::subagents::SubagentDispatcher::definitions(&dispatcher)[0].agent_type,
        "fake-remote"
    );

    let trace = Arc::new(RecordingTraceSink::default());
    let result = dispatcher
        .dispatch_traced(
            "parent-thread".to_string(),
            "parent-turn".to_string(),
            sample_subagent_request("do remote work", Some(30)),
            Some(trace.clone()),
        )
        .await
        .unwrap();

    assert_eq!(result.final_message, "remote work finished");
    assert_eq!(result.thread_id, "bc-fake-agent");
    assert_eq!(
        result.exit_reason,
        roder_api::subagents::SubagentExitReason::Completed
    );
    assert_eq!(result.metadata["agentId"], "bc-fake-agent");

    let statuses = trace.statuses.lock().unwrap().clone();
    assert_eq!(
        statuses,
        vec!["CREATING: provisioning".to_string(), "RUNNING".to_string()],
        "child status events must surface through the trace sink"
    );
    host.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn subagent_dispatch_failure_and_timeout_are_explicit() {
    // Child-reported failure.
    let host = host_for(fake_config(BTreeMap::new()));
    let dispatcher =
        ProcessSubagentDispatcher::new(host.clone(), "fake-process-dispatcher".to_string());
    let error = dispatcher
        .dispatch(
            "parent-thread".to_string(),
            "parent-turn".to_string(),
            sample_subagent_request("fail please", Some(30)),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("fake dispatch failure"), "{error}");
    host.shutdown().await;

    // Host-side timeout: the dispatch never gets a terminal event, so the
    // host cancels through `subagents/cancel` and fails the dispatch.
    let host = host_for(fake_config(BTreeMap::from([(
        "FAKE_CHILD_DISPATCH_HANG".to_string(),
        "1".to_string(),
    )])));
    let dispatcher =
        ProcessSubagentDispatcher::new(host.clone(), "fake-process-dispatcher".to_string());
    let error = dispatcher
        .dispatch(
            "parent-thread".to_string(),
            "parent-turn".to_string(),
            sample_subagent_request("hang forever", Some(1)),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("timed out after 1s"), "{error}");
    let cancelled = host.drain_extension_events().await;
    assert!(
        cancelled
            .iter()
            .any(|event| event.event_kind == "fake.dispatch_cancelled"),
        "the child must observe the cancellation: {cancelled:?}"
    );
    host.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn subagent_dispatch_child_death_fails_without_hanging() {
    let host = host_for(fake_config(BTreeMap::from([(
        "FAKE_CHILD_DISPATCH_EXIT".to_string(),
        "1".to_string(),
    )])));
    let dispatcher =
        ProcessSubagentDispatcher::new(host.clone(), "fake-process-dispatcher".to_string());
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        dispatcher.dispatch(
            "parent-thread".to_string(),
            "parent-turn".to_string(),
            sample_subagent_request("crash mid-dispatch", Some(30)),
        ),
    )
    .await
    .expect("child death must fail the dispatch promptly")
    .unwrap_err()
    .to_string();
    assert!(error.contains("failed mid-dispatch"), "{error}");
    assert!(
        !error.contains("crash mid-dispatch"),
        "prompt text must not leak into the failure: {error}"
    );
    host.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn task_executor_serves_spec_output_result_and_failure() {
    let host = host_for(fake_config(BTreeMap::new()));
    let executor = ProcessTaskExecutor::new(host.clone(), "fake-process-task".to_string());

    let spec = executor.fetch_spec().await.unwrap();
    assert_eq!(spec.kind, "fake-process-task");
    assert_eq!(spec.default_timeout_seconds, Some(120));
    assert_eq!(
        roder_api::tasks::TaskExecutor::spec(&executor).kind,
        "fake-process-task",
        "the sync accessor serves the cached child spec"
    );

    let output = Arc::new(RecordingTaskOutput::default());
    let ctx = roder_api::tasks::TaskExecutionContext {
        task_id: "task-1".to_string(),
        thread_id: Some("thread-1".to_string()),
        turn_id: None,
        workspace_root: None,
        runner_destination: None,
        runner_session: None,
        deadline: None,
        process_grace_timeout: std::time::Duration::from_millis(250),
        process_kill_timeout: std::time::Duration::from_secs(1),
        metadata: serde_json::json!({}),
        process_registry: None,
        output: roder_api::tasks::TaskOutputSink::new(output.clone()),
    };
    let result = executor
        .execute(ctx.clone(), serde_json::json!({ "prompt": "go" }))
        .await
        .unwrap();
    assert_eq!(result.payload["taskId"], "task-1");
    assert_eq!(result.payload["agentId"], "bc-fake-agent");
    assert_eq!(result.payload["echo"]["prompt"], "go");
    assert_eq!(
        output.chunks.lock().unwrap().clone(),
        vec!["status: RUNNING".to_string()],
        "child output events must reach the task output sink"
    );

    let error = executor
        .execute(ctx, serde_json::json!({ "fail": true }))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("fake task failure"), "{error}");
    host.shutdown().await;
}

fn sample_subagent_request(
    prompt: &str,
    timeout_seconds: Option<u64>,
) -> roder_api::subagents::SubagentRequest {
    roder_api::subagents::SubagentRequest {
        description: "host test".to_string(),
        prompt: prompt.to_string(),
        subagent_type: Some("fake-remote".to_string()),
        model: Some("fake-model".to_string()),
        tools: None,
        lane: None,
        max_concurrent: None,
        allowed_tools: None,
        parent_deadline_seconds: None,
        inputs: None,
        timeout_seconds,
    }
}

#[derive(Default)]
struct RecordingTraceSink {
    statuses: std::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl roder_api::trace::SubagentTraceSink for RecordingTraceSink {
    async fn trace_created(&self, _summary: roder_api::trace::SubagentTraceSummary) {}

    async fn trace_delta(&self, _delta: roder_api::trace::SubagentTraceDelta) {}

    async fn trace_status_changed(
        &self,
        _trace_id: roder_api::trace::SubagentTraceId,
        _parent: roder_api::trace::ParentTurnRef,
        _status: roder_api::trace::SubagentTraceStatus,
        detail: Option<String>,
    ) {
        self.statuses
            .lock()
            .unwrap()
            .push(detail.unwrap_or_default());
    }

    async fn trace_completed(&self, _summary: roder_api::trace::SubagentTraceSummary) {}

    async fn trace_failed(&self, _summary: roder_api::trace::SubagentTraceSummary, _error: String) {
    }
}

#[derive(Default)]
struct RecordingTaskOutput {
    chunks: std::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl roder_api::tasks::TaskOutputWriter for RecordingTaskOutput {
    async fn write(
        &self,
        _stream: roder_api::tasks::TaskOutputStream,
        chunk: String,
    ) -> anyhow::Result<()> {
        self.chunks.lock().unwrap().push(chunk);
        Ok(())
    }
}

fn tool_call(arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        id: "call-1".to_string(),
        name: "word_count".to_string(),
        raw_arguments: arguments.to_string(),
        arguments,
        thread_id: "thread-1".to_string(),
        turn_id: "turn-1".to_string(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn tool_provider_contributes_declared_tools_and_executes_through_the_child() {
    let loaded = load_process_extension(fake_config(BTreeMap::new()), &fixtures_dir()).unwrap();
    let extension = ProcessHostExtension::new(loaded);
    let host = extension.host();

    let mut builder = roder_api::extension::ExtensionRegistryBuilder::new();
    builder.install(extension).unwrap();
    let registry = builder.build().unwrap();

    // The manifest-declared schema reaches the tool registry without
    // spawning the child (contribution is static).
    let mut tools = ToolRegistry::default();
    registry.tools[0].contribute(&mut tools).unwrap();
    let tool = tools.get("word_count").expect("word_count registered");
    let spec = tool.spec();
    assert_eq!(spec.description, "Count whitespace-separated words.");
    assert_eq!(spec.parameters["type"], "object");
    assert_eq!(spec.parameters["required"], serde_json::json!(["text"]));

    // Executing the registered handler round-trips tools/call to the child.
    let result = tool
        .execute(
            ToolExecutionContext::new("thread-1", "turn-1", PolicyMode::Default),
            tool_call(serde_json::json!({ "text": "one two three" })),
        )
        .await
        .unwrap();
    assert_eq!(result.id, "call-1");
    assert_eq!(result.name, "word_count");
    assert_eq!(result.text, "3 words");
    assert!(!result.is_error);
    assert_eq!(result.data["wordCount"], 3);
    assert_eq!(result.data["callId"], "call-1");
    host.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tool_call_child_errors_become_failed_tool_results() {
    let host = host_for(fake_config(BTreeMap::from([(
        "FAKE_CHILD_TOOL_ERROR".to_string(),
        "word_count exploded".to_string(),
    )])));
    let contributor = roder_ext_process_host::ProcessToolContributor::new(
        host.clone(),
        "fake-process-tools".to_string(),
        load_process_extension(fake_config(BTreeMap::new()), &fixtures_dir())
            .unwrap()
            .manifest
            .provides
            .iter()
            .find_map(|service| match service {
                roder_api::process_extension::ProcessProvidedService::ToolProvider {
                    tools,
                    ..
                } => Some(tools.clone()),
                _ => None,
            })
            .expect("tool provider declared"),
    );
    let mut tools = ToolRegistry::default();
    roder_api::tools::ToolContributor::contribute(&contributor, &mut tools).unwrap();

    let result = tools
        .get("word_count")
        .expect("word_count registered")
        .execute(
            ToolExecutionContext::new("thread-1", "turn-1", PolicyMode::Default),
            tool_call(serde_json::json!({ "text": "boom" })),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(
        result.text.contains("word_count exploded"),
        "{}",
        result.text
    );
    assert_eq!(result.data["error"]["kind"], "tool_execution_failed");
    host.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn python_tools_example_package_serves_word_count_through_the_host() {
    // The shipped example must satisfy the same echo validation as the
    // fixture: identity, services (tomllib parse vs Rust toml parse), and
    // manifest checksum.
    let example_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/non-rust-extensions/python-tools");
    let manifest = example_dir.join("roder-extension.toml");
    let config = ProcessExtensionConfig {
        id: "python-tools".to_string(),
        enabled: true,
        manifest: manifest.display().to_string(),
        command: "python3".to_string(),
        args: vec![example_dir.join("main.py").display().to_string()],
        cwd: None,
        env: BTreeMap::from([(
            "RODER_EXTENSION_MANIFEST".to_string(),
            manifest.display().to_string(),
        )]),
        startup_timeout_ms: 10_000,
        event_filter: ProcessEventFilter::default(),
    };
    let loaded = load_process_extension(config, &example_dir).unwrap();
    let extension = ProcessHostExtension::new(loaded);
    let host = extension.host();

    let mut builder = roder_api::extension::ExtensionRegistryBuilder::new();
    builder.install(extension).unwrap();
    let registry = builder.build().unwrap();
    let mut tools = ToolRegistry::default();
    registry.tools[0].contribute(&mut tools).unwrap();

    let result = tools
        .get("word_count")
        .expect("word_count registered")
        .execute(
            ToolExecutionContext::new("thread-1", "turn-1", PolicyMode::Default),
            tool_call(serde_json::json!({ "text": "roder hosts python tools now" })),
        )
        .await
        .unwrap();
    assert_eq!(result.text, "5 words");
    assert!(!result.is_error);
    host.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tool_registration_never_spawns_the_child_and_spawn_failures_fail_the_call() {
    // A config whose command cannot spawn still builds the registry and
    // registers the declared tool — proof the schema comes statically from
    // the manifest. Only execution touches the child.
    let mut config = fake_config(BTreeMap::new());
    config.command = "roder-definitely-missing-binary".to_string();
    let loaded = load_process_extension(config, &fixtures_dir()).unwrap();
    let extension = ProcessHostExtension::new(loaded);

    let mut builder = roder_api::extension::ExtensionRegistryBuilder::new();
    builder.install(extension).unwrap();
    let registry = builder.build().unwrap();
    let mut tools = ToolRegistry::default();
    registry.tools[0].contribute(&mut tools).unwrap();

    let result = tools
        .get("word_count")
        .expect("word_count registered")
        .execute(
            ToolExecutionContext::new("thread-1", "turn-1", PolicyMode::Default),
            tool_call(serde_json::json!({ "text": "unreachable" })),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.text.contains("spawn"), "{}", result.text);
}
