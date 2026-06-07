use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use futures::stream;
use roder_api::inference::{
    AgentInferenceRequest, CompletionMetadata, InferenceCapabilities, InferenceEngine,
    InferenceEvent, InferenceEventStream, InferenceProviderContext, InferenceProviderMetadata,
    InferenceTurnContext, MessageDelta, ModelDescriptor, ToolCallCompleted,
};
use roder_api::memory::MemoryScope;
use roder_api::tools::{ToolChoice, ToolRegistry};
use roder_ext_gbrain::{
    AgenticToolRunnerConfig, CaptureInput, Embedder, EngineAgenticToolRunner, GbrainStore,
    GbrainToolContributor,
};
use serde_json::json;

fn store() -> Arc<GbrainStore> {
    Arc::new(GbrainStore::open_in_memory(Embedder::new(None)).unwrap())
}

fn read_only_registry(store: Arc<GbrainStore>) -> ToolRegistry {
    let mut registry = ToolRegistry::default();
    GbrainToolContributor::new(store)
        .contribute_read_only(&mut registry)
        .unwrap();
    registry
}

#[tokio::test]
async fn provider_tool_runner_continues_past_three_tool_turns() {
    let store = store();
    let registry = read_only_registry(store);
    let engine = Arc::new(ScriptedEngine::new(Script::FiveNotesThenFinal));
    let runner = EngineAgenticToolRunner::new(engine.clone(), "fake", "fake-agent", None)
        .with_config(AgenticToolRunnerConfig {
            max_tool_calls: Some(8),
            ..AgenticToolRunnerConfig::default()
        });

    let answer = runner
        .answer_with_tools(
            registry,
            "Keep searching until evidence is sufficient.",
            None,
            None,
        )
        .await
        .unwrap();

    assert_eq!(answer.answer, "Final answer after five tool turns.");
    assert_eq!(answer.trace.tool_observations.len(), 5);
    assert_eq!(answer.trace.responded_via, "final_text");
    assert_eq!(answer.trace.stop_reason.as_deref(), Some("stop"));
    let requests = engine.requests.lock().unwrap();
    assert_eq!(requests.len(), 6);
    assert!(matches!(requests[0].tool_choice, ToolChoice::Auto));
    assert_eq!(requests[0].runtime.parallel_tool_calls, Some(true));
    assert!(
        requests[0]
            .tools
            .iter()
            .any(|spec| spec.name == "gbrain_retrieval_note")
    );
    assert_eq!(
        requests[5]
            .transcript
            .iter()
            .filter(|item| matches!(item, roder_api::transcript::TranscriptItem::ToolResult(_)))
            .count(),
        5
    );
}

#[tokio::test]
async fn provider_tool_runner_can_finish_via_respond_to_query() {
    let store = store();
    let registry = read_only_registry(store.clone());
    let engine = Arc::new(ScriptedEngine::new(Script::RespondToQuery));
    let runner = EngineAgenticToolRunner::new(engine.clone(), "fake", "fake-agent", None);

    let answer = runner
        .answer_with_tools(registry, "Who owns Acme?", None, None)
        .await
        .unwrap();
    let mut trace = answer.trace.clone();
    trace.record_memory_snapshot(
        store
            .memory_snapshot(Some(MemoryScope::Global))
            .await
            .unwrap(),
    );

    assert_eq!(answer.answer, "Maya owns Acme.");
    assert_eq!(trace.responded_via, "respond_to_query");
    assert_eq!(trace.tool_observations.len(), 1);
    let trace_json = serde_json::to_value(&trace).unwrap();
    for key in [
        "rawSnapshotHighWatermark",
        "selectedDreamRunId",
        "selectedOntologyVersion",
        "derivedSnapshotVersion",
        "providerTurns",
        "toolCalls",
        "toolObservations",
        "retrievalNotes",
        "respondedVia",
        "parallelToolCalls",
        "claims",
        "rejectedClaims",
        "unsupportedClaimCount",
        "quoteSpanCoverage",
        "citationPrecision",
        "stopReason",
    ] {
        assert!(
            trace_json.as_object().unwrap().contains_key(key)
                || matches!(
                    key,
                    "rawSnapshotHighWatermark"
                        | "selectedDreamRunId"
                        | "selectedOntologyVersion"
                        | "derivedSnapshotVersion"
                        | "quoteSpanCoverage"
                        | "citationPrecision"
                ),
            "missing trace key {key}: {trace_json:#}"
        );
    }
    let feedback = trace.to_query_feedback_input(
        "Who owns Acme?",
        &answer.answer,
        Some(MemoryScope::Global),
        Some("owner_lookup".to_string()),
        Some("eval-1".to_string()),
        Some(12),
    );
    let appended = store.append_query_feedback(feedback).await.unwrap();
    assert_eq!(appended.tool_call_count, 1);
    assert_eq!(appended.stop_reason.as_deref(), Some("respond_to_query"));
    assert_eq!(appended.used_cards, vec!["card:artifact-1".to_string()]);
    assert_eq!(appended.question_kind.as_deref(), Some("owner_lookup"));
    assert_eq!(appended.eval_result_id.as_deref(), Some("eval-1"));
    assert_eq!(engine.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn provider_tool_runner_rejects_mutating_tool_without_mutating_store() {
    let store = store();
    store
        .capture(CaptureInput::new(
            MemoryScope::Global,
            "Existing fact should remain the only fact.",
        ))
        .await
        .unwrap();
    let registry = read_only_registry(store.clone());
    let engine = Arc::new(ScriptedEngine::new(Script::MutatingThenFinal));
    let runner = EngineAgenticToolRunner::new(engine, "fake", "fake-agent", None);

    let answer = runner
        .answer_with_tools(registry, "Try to write during retrieval.", None, None)
        .await
        .unwrap();

    assert_eq!(answer.answer, "I cannot mutate memory during retrieval.");
    assert!(answer.trace.tool_observations[0].result.is_error);
    assert!(
        answer.trace.tool_observations[0]
            .result
            .text
            .contains("non-read-only")
    );
    let recall = store
        .recall(roder_ext_gbrain::RecallParams {
            query: "Existing fact".to_string(),
            as_of: roder_ext_gbrain::AsOf::now(),
            scope: Some(MemoryScope::Global),
            include_global: false,
            limit: 10,
            expand: false,
        })
        .await
        .unwrap();
    assert_eq!(recall.hits.len(), 1);
}

#[tokio::test]
async fn provider_tool_runner_executes_tool_batches_with_parallel_hint() {
    let store = store();
    let registry = read_only_registry(store);
    let engine = Arc::new(ScriptedEngine::new(Script::ParallelNotesThenFinal));
    let runner = EngineAgenticToolRunner::new(engine.clone(), "fake", "fake-agent", None);

    let answer = runner
        .answer_with_tools(registry, "Check two branches before answering.", None, None)
        .await
        .unwrap();

    assert_eq!(answer.answer, "Final answer after the parallel batch.");
    assert!(answer.trace.parallel_tool_calls);
    assert_eq!(answer.trace.tool_calls.len(), 2);
    assert_eq!(answer.trace.tool_observations.len(), 2);
    let requests = engine.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].runtime.parallel_tool_calls, Some(true));
    assert_eq!(
        requests[1]
            .transcript
            .iter()
            .filter(|item| matches!(item, roder_api::transcript::TranscriptItem::ToolCall(_)))
            .count(),
        2
    );
    assert_eq!(
        requests[1]
            .transcript
            .iter()
            .filter(|item| matches!(item, roder_api::transcript::TranscriptItem::ToolResult(_)))
            .count(),
        2
    );
}

#[derive(Debug, Clone, Copy)]
enum Script {
    FiveNotesThenFinal,
    RespondToQuery,
    MutatingThenFinal,
    ParallelNotesThenFinal,
}

struct ScriptedEngine {
    script: Script,
    calls: AtomicUsize,
    requests: Mutex<Vec<AgentInferenceRequest>>,
}

impl ScriptedEngine {
    fn new(script: Script) -> Self {
        Self {
            script,
            calls: AtomicUsize::new(0),
            requests: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl InferenceEngine for ScriptedEngine {
    fn id(&self) -> roder_api::extension::InferenceEngineId {
        "fake".to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities::coding_agent_default()
    }

    fn metadata(&self) -> InferenceProviderMetadata {
        InferenceProviderMetadata::local("fake")
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(Vec::new())
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let call_index = self.calls.fetch_add(1, Ordering::SeqCst);
        self.requests.lock().unwrap().push(request);
        let events = match self.script {
            Script::FiveNotesThenFinal if call_index < 5 => {
                vec![Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: format!("note-{call_index}"),
                    name: "gbrain_retrieval_note".to_string(),
                    arguments: json!({
                        "note": format!("checking pass {call_index}"),
                        "openQuestions": ["still checking"]
                    })
                    .to_string(),
                }))]
            }
            Script::FiveNotesThenFinal => final_events("Final answer after five tool turns."),
            Script::RespondToQuery => {
                vec![Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "respond-1".to_string(),
                    name: "respond_to_query".to_string(),
                    arguments: json!({
                        "message": "Maya owns Acme.",
                        "confidence": "high",
                        "citedEvidenceIds": ["card:artifact-1"]
                    })
                    .to_string(),
                }))]
            }
            Script::MutatingThenFinal if call_index == 0 => {
                vec![Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "capture-1".to_string(),
                    name: "gbrain_capture".to_string(),
                    arguments: json!({"text": "do not write"}).to_string(),
                }))]
            }
            Script::MutatingThenFinal => final_events("I cannot mutate memory during retrieval."),
            Script::ParallelNotesThenFinal if call_index == 0 => {
                vec![
                    Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                        id: "note-left".to_string(),
                        name: "gbrain_retrieval_note".to_string(),
                        arguments: json!({
                            "note": "checking left branch",
                            "openQuestions": ["left"]
                        })
                        .to_string(),
                    })),
                    Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                        id: "note-right".to_string(),
                        name: "gbrain_retrieval_note".to_string(),
                        arguments: json!({
                            "note": "checking right branch",
                            "openQuestions": ["right"]
                        })
                        .to_string(),
                    })),
                ]
            }
            Script::ParallelNotesThenFinal => {
                final_events("Final answer after the parallel batch.")
            }
        };
        Ok(Box::pin(stream::iter(events)))
    }
}

fn final_events(message: &str) -> Vec<anyhow::Result<InferenceEvent>> {
    vec![
        Ok(InferenceEvent::MessageDelta(MessageDelta {
            text: message.to_string(),
            phase: None,
        })),
        Ok(InferenceEvent::Completed(CompletionMetadata {
            stop_reason: Some("stop".to_string()),
            provider_response_id: None,
        })),
    ]
}
