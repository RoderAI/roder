//! Proves prompt-time knowledge recall end to end: a saved knowledge
//! document is injected into the provider request as a `Knowledge` context
//! block when its project scope matches the turn workspace.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::stream;
use roder_api::catalog::PROVIDER_MOCK;
use roder_api::extension::{ExtensionRegistryBuilder, InferenceEngineId};
use roder_api::inference::*;
use roder_api::knowledge::{KnowledgeKind, KnowledgeSaveRequest, KnowledgeSource, KnowledgeStore};
use roder_api::memory::MemoryScope;
use roder_api::transcript::TranscriptItem;
use roder_core::{Runtime, RuntimeConfig, StartTurnRequest, default_instructions};
use roder_ext_knowledge_md::{KnowledgeMdExtension, MarkdownKnowledgeStore};

struct CaptureEngine {
    requests: Mutex<Vec<AgentInferenceRequest>>,
}

#[async_trait::async_trait]
impl InferenceEngine for CaptureEngine {
    fn id(&self) -> InferenceEngineId {
        PROVIDER_MOCK.to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities::coding_agent_default()
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
        self.requests.lock().unwrap().push(request);
        Ok(Box::pin(stream::iter(vec![
            Ok(InferenceEvent::MessageDelta(MessageDelta {
                text: "done".to_string(),
                phase: None,
            })),
            Ok(InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some("stop".to_string()),
                provider_response_id: Some("resp_1".to_string()),
            })),
        ])))
    }
}

#[tokio::test]
async fn saved_knowledge_is_recalled_into_provider_requests() {
    let workspace = std::env::temp_dir().join(format!("knowledge-recall-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&workspace).unwrap();
    let project_key = workspace.file_name().unwrap().to_string_lossy().to_string();
    let knowledge_base =
        std::env::temp_dir().join(format!("knowledge-base-{}", uuid::Uuid::new_v4()));

    // Seed a document under the project scope the turn workspace resolves to.
    let store = MarkdownKnowledgeStore::new(knowledge_base.clone());
    store
        .save(KnowledgeSaveRequest {
            scope: MemoryScope::Project(project_key),
            kind: KnowledgeKind::Requirement,
            title: "Session token policy".to_string(),
            tags: Vec::new(),
            body: "All session tokens rotate every 24 hours; the marker is wren-rotation."
                .to_string(),
            source: KnowledgeSource::User,
        })
        .await
        .unwrap();

    let engine = Arc::new(CaptureEngine {
        requests: Mutex::new(Vec::new()),
    });
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(engine.clone());
    builder
        .install(KnowledgeMdExtension::new(knowledge_base))
        .unwrap();
    let runtime =
        Arc::new(Runtime::new(builder.build().unwrap(), RuntimeConfig::default()).unwrap());
    let mut events = runtime.subscribe_events();

    runtime
        .start_turn(StartTurnRequest {
            thread_id: "thread_knowledge".to_string(),
            message: "what is our session token rotation policy?".to_string(),
            images: Vec::new(),
            provider_override: None,
            model_override: None,
            reasoning_override: None,
            workspace: workspace.display().to_string(),
            instructions: default_instructions(),
            developer_context: None,
            task_ledger_required: false,
        })
        .await
        .unwrap();

    let mut saw_knowledge_block_event = false;
    loop {
        let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .unwrap()
            .unwrap();
        if let roder_api::events::RoderEvent::ContextBlockAdded(added) = &event.event
            && added.block_type == "Knowledge"
        {
            saw_knowledge_block_event = true;
        }
        if event.kind == "turn.completed" && event.thread_id.as_deref() == Some("thread_knowledge")
        {
            break;
        }
    }

    let requests = engine.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let injected = requests[0].transcript.iter().any(|item| {
        matches!(
            item,
            TranscriptItem::UserMessage(message)
                if message.text.contains("wren-rotation")
                    && message.text.contains("knowledge_read")
        )
    });
    assert!(
        injected,
        "expected the knowledge snippet to be injected as a user message; transcript: {:#?}",
        requests[0].transcript
    );
    assert!(
        saw_knowledge_block_event,
        "expected a context.block_added event with a Knowledge block"
    );

    // The agent can also see the knowledge tools.
    let tool_names = requests[0]
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();
    for tool in ["knowledge_search", "knowledge_save", "knowledge_read"] {
        assert!(tool_names.contains(&tool), "missing tool {tool}");
    }
}
