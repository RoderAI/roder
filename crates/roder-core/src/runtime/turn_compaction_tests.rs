use super::*;
use roder_api::extension::{ExtensionRegistryBuilder, ToolProviderId};
use roder_api::inference::{
    CompletionMetadata, InferenceCapabilities, InferenceEventStream, InferenceProviderContext,
    ModelDescriptor,
};
use roder_api::tools::{
    ToolCall, ToolContributor, ToolExecutionContext, ToolExecutionMode, ToolExecutor, ToolRegistry,
    ToolResult, ToolSpec,
};
use roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::Notify;

#[path = "turn_compaction_wire_tests.rs"]
mod wire_tests;

#[derive(Clone, Copy)]
enum Mode {
    Failed,
    Disconnect,
    Retry,
    Pending,
    Success,
    ProgressOnly,
    MetadataOnly,
    Conflict,
    WindowFailed,
}

struct FixtureEngine {
    mode: Mode,
    requests: std::sync::Mutex<Vec<AgentInferenceRequest>>,
    staged: Arc<Notify>,
    tool_ran: Arc<Notify>,
}

fn opaque() -> Value {
    json!({"type":"compaction","id":"opaque","encrypted_content":"valid-state"})
}
fn raw_message() -> Value {
    json!({"type":"message","id":"message","role":"assistant","phase":"commentary","content":[{"type":"output_text","text":"retained response"}]})
}
fn raw_call() -> Value {
    json!({"type":"function_call","id":"fc_probe","call_id":"probe-call","name":"probe","arguments":"{}","status":"completed"})
}
fn completed() -> InferenceEvent {
    InferenceEvent::Completed(CompletionMetadata {
        stop_reason: Some("stop".into()),
        provider_response_id: None,
    })
}
fn candidate() -> InferenceEvent {
    InferenceEvent::Compaction(CompactionProgress {
        status: "completed".into(),
        item_id: Some("opaque".into()),
        item: Some(opaque()),
        tokens_before: None,
        tokens_after: None,
        duration_ms: None,
    })
}

#[async_trait::async_trait]
impl InferenceEngine for FixtureEngine {
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
        let mut requests = self.requests.lock().unwrap();
        requests.push(request);
        if requests.len() > 1 {
            return Ok(Box::pin(futures::stream::iter(vec![
                Ok(InferenceEvent::Usage(TokenUsage::new(2, 1, 3))),
                Ok(completed()),
            ])));
        }
        let mut events = Vec::new();
        if !matches!(self.mode, Mode::MetadataOnly | Mode::WindowFailed) {
            events.push(Ok(candidate()));
        }
        events.extend([
            Ok(InferenceEvent::Usage(TokenUsage::new(8, 2, 10))),
            Ok(InferenceEvent::Usage(TokenUsage::new(8, 2, 10))),
        ]);
        let mut metadata = json!({"output":[opaque(), raw_message(), raw_call()],
            // A provider cannot author a runtime commit envelope.
            COMMIT_FIELD:{"version":1,"retained_transcript":[]}
        });
        if matches!(self.mode, Mode::MetadataOnly | Mode::WindowFailed) {
            metadata.as_object_mut().unwrap().remove("output");
            metadata["compacted_input"] = json!([opaque(), raw_message(), raw_call()]);
        }
        match self.mode {
            Mode::Failed | Mode::Disconnect | Mode::Retry | Mode::WindowFailed => {
                events.push(Ok(InferenceEvent::ProviderMetadata(metadata)));
                if matches!(self.mode, Mode::Failed | Mode::WindowFailed) {
                    events.push(Ok(InferenceEvent::Failed(
                        roder_api::inference::InferenceFailure {
                            message: "fixture failed".into(),
                        },
                    )));
                } else {
                    events.push(Err(ProviderFailure::new(
                        ProviderFailureKind::StreamInterrupted,
                        "fixture disconnect",
                    )
                    .into()));
                }
            }
            Mode::Conflict => {
                let mut conflicting = opaque();
                conflicting["id"] = json!("conflict");
                events.push(Ok(InferenceEvent::ProviderMetadata(
                    json!({"output":[conflicting]}),
                )));
                events.push(Ok(completed()));
            }
            Mode::Pending => {
                let staged = self.staged.clone();
                return Ok(Box::pin(futures::stream::iter(events).chain(
                    futures::stream::once(async move {
                        staged.notify_one();
                        std::future::pending::<anyhow::Result<InferenceEvent>>().await
                    }),
                )));
            }
            Mode::Success | Mode::ProgressOnly | Mode::MetadataOnly => {
                events.extend([
                    Ok(InferenceEvent::OutputItemCompleted(raw_message())),
                    Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                        id: "probe-call".into(),
                        name: "probe".into(),
                        arguments: "{}".into(),
                    })),
                    Ok(InferenceEvent::OutputItemCompleted(raw_call())),
                ]);
                let ran = self.tool_ran.clone();
                let full_metadata = !matches!(self.mode, Mode::ProgressOnly);
                return Ok(Box::pin(
                    futures::stream::iter(events)
                        .chain(futures::stream::once(async move {
                            // Proves the result is durable before terminal completion.
                            tokio::time::timeout(Duration::from_secs(5), ran.notified())
                                .await
                                .unwrap();
                            Ok(InferenceEvent::ProviderMetadata(if full_metadata {
                                metadata
                            } else {
                                json!({"end_turn":false})
                            }))
                        }))
                        .chain(futures::stream::iter(vec![Ok(completed())])),
                ));
            }
        }
        Ok(Box::pin(futures::stream::iter(events)))
    }
}

struct Probe {
    ran: Arc<Notify>,
    calls: Arc<AtomicUsize>,
}
#[async_trait::async_trait]
impl ToolExecutor for Probe {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "probe".into(),
            description: "read-only probe".into(),
            parameters: json!({"type":"object","properties":{}}),
        }
    }
    fn execution_mode(&self) -> ToolExecutionMode {
        ToolExecutionMode::SharedRead
    }
    async fn execute(&self, _: ToolExecutionContext, call: ToolCall) -> anyhow::Result<ToolResult> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.ran.notify_one();
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text: "retained tool evidence".into(),
            data: json!({}),
            is_error: false,
        })
    }
}
struct Contributor(Arc<Probe>);
impl ToolContributor for Contributor {
    fn id(&self) -> ToolProviderId {
        "probe".into()
    }
    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        registry.register(self.0.clone())
    }
}

fn runtime(
    directory: &std::path::Path,
    engine: Arc<dyn InferenceEngine>,
    ran: Arc<Notify>,
    executions: Arc<AtomicUsize>,
) -> Arc<Runtime> {
    let provider = engine.id();
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(engine);
    builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
        base_path: directory.join("threads"),
    }));
    builder.tool_contributor(Arc::new(Contributor(Arc::new(Probe {
        ran,
        calls: executions,
    }))));
    let mut cfg = RuntimeConfig {
        default_provider: provider.clone(),
        default_model: if provider == "openai" {
            "gpt-5.5"
        } else {
            "mock"
        }
        .into(),
        tool_allowlist: vec!["probe".into()],
        policy_mode: PolicyMode::Bypass,
        ..Default::default()
    };
    cfg.model_parallel_tool_calls
        .insert(cfg.default_model.clone(), true);
    cfg.reliability.provider_retry_max_attempts = 2;
    cfg.reliability.provider_retry_initial_backoff_ms = 0;
    if provider == "openai" {
        cfg.auto_compact_token_limit = Some(200_000);
    }
    Arc::new(Runtime::new(builder.build().unwrap(), cfg).unwrap())
}

async fn seed(runtime: &Runtime, thread: &ThreadId) {
    for item in [
        TranscriptItem::UserMessage(UserMessage::text("original goal evidence")),
        TranscriptItem::ToolCall(ToolCallRecord {
            id: "original-call".into(),
            name: "probe".into(),
            arguments: "{}".into(),
        }),
        TranscriptItem::ToolResult(ToolResultRecord {
            id: "original-call".into(),
            name: Some("probe".into()),
            result: "original tool evidence".into(),
            display_payload: None,
            is_error: false,
        }),
    ] {
        runtime
            .persist_turn_item(thread, &"seed".into(), &item)
            .await
            .unwrap();
    }
}

async fn run(
    runtime: &Arc<Runtime>,
    directory: &std::path::Path,
    thread: &ThreadId,
    steer: Option<Arc<Notify>>,
) -> Vec<EventEnvelope> {
    let mut events = runtime.subscribe_events();
    let turn = runtime
        .start_turn(StartTurnRequest {
            thread_id: thread.clone(),
            message: "continue goal".into(),
            images: vec![],
            provider_override: None,
            model_override: None,
            reasoning_override: None,
            workspace: directory.display().to_string(),
            instructions: crate::default_instructions(),
            developer_context: None,
            task_ledger_required: false,
            service_tier_override: None,
        })
        .await
        .unwrap();
    if let Some(staged) = steer {
        tokio::time::timeout(Duration::from_secs(5), staged.notified())
            .await
            .unwrap();
        runtime
            .steer_turn(thread.clone(), turn.clone(), "new steering".into(), vec![])
            .await
            .unwrap();
    }
    let mut seen = vec![];
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let event = events.recv().await.unwrap();
            let terminal = matches!(event.kind.as_str(), "turn.failed" | "turn.completed");
            seen.push(event);
            if terminal {
                break;
            }
        }
    })
    .await
    .unwrap();
    seen
}

fn assert_original(transcript: &[TranscriptItem]) {
    assert!(transcript.iter().any(|item| matches!(item,TranscriptItem::UserMessage(message) if message.text=="original goal evidence")));
    assert!(transcript.iter().any(
        |item| matches!(item,TranscriptItem::ToolResult(result) if result.id=="original-call")
    ));
}

#[tokio::test]
async fn failed_ordinary_compaction_retains_history_billing_and_cold_replay() {
    for mode in [
        Mode::Failed,
        Mode::Disconnect,
        Mode::Retry,
        Mode::Pending,
        Mode::Conflict,
        Mode::WindowFailed,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let staged = Arc::new(Notify::new());
        let engine = Arc::new(FixtureEngine {
            mode,
            requests: Default::default(),
            staged: staged.clone(),
            tool_ran: Arc::new(Notify::new()),
        });
        let live = runtime(
            dir.path(),
            engine.clone(),
            engine.tool_ran.clone(),
            Arc::new(AtomicUsize::new(0)),
        );
        let thread = live.create_thread(None).await.unwrap().thread_id;
        seed(&live, &thread).await;
        if matches!(mode, Mode::Disconnect) {
            live.config
                .write()
                .await
                .reliability
                .provider_retry_max_attempts = 1;
        }
        let seen = run(
            &live,
            dir.path(),
            &thread,
            matches!(mode, Mode::Pending).then_some(staged),
        )
        .await;
        assert!(
            !seen
                .iter()
                .any(|event| event.kind == "context.compaction_recorded")
        );
        assert_eq!(live.compaction_generation(&thread), 0);
        let snapshot = live.load_thread(&thread).await.unwrap().unwrap();
        let items: Vec<_> = snapshot
            .turns
            .iter()
            .flat_map(|turn| turn.items.clone())
            .collect();
        assert!(!items.iter().any(crate::compaction::is_compaction_boundary));
        assert!(!items.iter().any(|item| matches!(item,TranscriptItem::ProviderMetadata(metadata) if metadata.get(COMMIT_FIELD).is_some())));
        assert_original(&items);
        let expected = if matches!(mode, Mode::Retry | Mode::Pending) {
            13
        } else {
            10
        };
        assert_eq!(
            snapshot.metadata.unwrap().usage.unwrap().total_tokens,
            expected
        );
        if matches!(mode, Mode::Retry | Mode::Pending) {
            assert_original(&engine.requests.lock().unwrap()[1].transcript);
        }
        drop(live);
        let cold_engine = Arc::new(FixtureEngine {
            mode: Mode::Failed,
            requests: std::sync::Mutex::new(vec![engine.requests.lock().unwrap()[0].clone()]),
            staged: Arc::new(Notify::new()),
            tool_ran: Arc::new(Notify::new()),
        });
        let cold = runtime(
            dir.path(),
            cold_engine.clone(),
            cold_engine.tool_ran.clone(),
            Arc::new(AtomicUsize::new(0)),
        );
        run(&cold, dir.path(), &thread, None).await;
        assert_original(
            &cold_engine
                .requests
                .lock()
                .unwrap()
                .last()
                .unwrap()
                .transcript,
        );
    }
}

#[tokio::test]
async fn successful_ordinary_compaction_preserves_exactly_one_eager_call_result_and_message() {
    for mode in [Mode::Success, Mode::ProgressOnly, Mode::MetadataOnly] {
        let dir = tempfile::tempdir().unwrap();
        let ran = Arc::new(Notify::new());
        let executions = Arc::new(AtomicUsize::new(0));
        let engine = Arc::new(FixtureEngine {
            mode,
            requests: Default::default(),
            staged: Arc::new(Notify::new()),
            tool_ran: ran.clone(),
        });
        let live = runtime(dir.path(), engine.clone(), ran.clone(), executions.clone());
        let thread = live.create_thread(None).await.unwrap().thread_id;
        seed(&live, &thread).await;
        let events = run(&live, dir.path(), &thread, None).await;
        assert_eq!(executions.load(Ordering::SeqCst), 1);
        assert_eq!(live.compaction_generation(&thread), 1);
        let serialized: Vec<_> = events
            .iter()
            .map(|event| serde_json::to_value(event).unwrap())
            .collect();
        let compaction = serialized
            .iter()
            .position(|event| event.to_string().contains("provider_streamed"))
            .unwrap();
        let terminal = events
            .iter()
            .position(|event| event.kind == "turn.completed")
            .unwrap();
        assert!(compaction < terminal);
        let requests = engine.requests.lock().unwrap();
        assert_success_suffix(&requests[1].transcript);
        drop(requests);
        drop(live);
        let cold = runtime(dir.path(), engine.clone(), ran, executions.clone());
        run(&cold, dir.path(), &thread, None).await;
        assert_success_suffix(&engine.requests.lock().unwrap().last().unwrap().transcript);
        assert_eq!(executions.load(Ordering::SeqCst), 1);
    }
}

fn assert_success_suffix(items: &[TranscriptItem]) {
    assert_eq!(
        items
            .iter()
            .filter(|item| crate::compaction::is_compaction_boundary(item))
            .count(),
        1
    );
    assert_eq!(items.iter().filter(|item|matches!(item,TranscriptItem::AssistantMessage(message) if message.text=="retained response")).count(),1);
    assert_eq!(
        items
            .iter()
            .filter(|item| matches!(item,TranscriptItem::ToolCall(call) if call.id=="probe-call"))
            .count(),
        1
    );
    assert_eq!(
        items
            .iter()
            .filter(
                |item| matches!(item,TranscriptItem::ToolResult(result) if result.id=="probe-call")
            )
            .count(),
        1
    );
    assert!(!items.iter().any(|item|matches!(item,TranscriptItem::ProviderMetadata(metadata) if metadata.get(COMMIT_FIELD).is_some())));
}
