use super::*;
use roder_api::events::{EventEnvelope, EventSource, RoderEvent, TranscriptItemAppended};
use roder_api::provider_error::{ProviderFailure, ProviderFailureKind};
use roder_api::transcript::{TranscriptItem, UserMessage};
use roder_ext_jsonl_thread_store::store::JsonlThreadStore;
use serde_json::json;

struct NativeCompactionEngine {
    fail: bool,
}
#[async_trait::async_trait]
impl InferenceEngine for NativeCompactionEngine {
    fn id(&self) -> String {
        "openai".into()
    }
    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities::text_only()
    }
    fn requires_native_compaction(&self) -> bool {
        true
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
        _: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        panic!("custom summary called through thread/compact")
    }
    async fn compact_turn(
        &self,
        _: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<Option<InferenceEventStream>> {
        assert_eq!(request.model.provider, "openai");
        assert!(
            request
                .instructions
                .developer
                .unwrap()
                .contains("keep BLUE")
        );
        if self.fail {
            return Err(ProviderFailure::new(
                ProviderFailureKind::ContextWindowExceeded,
                "native window too large",
            )
            .into());
        }
        Ok(Some(Box::pin(stream::iter(vec![
            Ok(InferenceEvent::ProviderMetadata(
                json!({"output":[{"type":"compaction","encrypted_content":"opaque"}],"compacted_input":[{"type":"message","role":"user","content":"BLUE"},{"type":"compaction","encrypted_content":"opaque"}]}),
            )),
            Ok(InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some("completed".into()),
                provider_response_id: None,
            })),
        ]))))
    }
}

#[tokio::test]
async fn thread_compact_preserves_native_state_and_reports_native_errors() {
    for fail in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = ExtensionRegistryBuilder::new();
        registry.inference_engine(Arc::new(NativeCompactionEngine { fail }));
        registry.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
            base_path: dir.path().into(),
        }));
        let runtime = Arc::new(
            Runtime::new(
                registry.build().unwrap(),
                RuntimeConfig {
                    default_provider: "openai".into(),
                    default_model: "gpt-5.5".into(),
                    ..RuntimeConfig::default()
                },
            )
            .unwrap(),
        );
        let thread = runtime.create_thread(None).await.unwrap().thread_id;
        let now = OffsetDateTime::now_utc();
        let item = TranscriptItemAppended {
            thread_id: thread.clone(),
            turn_id: "history".into(),
            item_type: "user_message".into(),
            item_index: Some(0),
            item: Some(TranscriptItem::UserMessage(UserMessage::text(
                "BLUE ".repeat(1_000),
            ))),
            timestamp: now,
        };
        JsonlThreadStore {
            base_path: dir.path().into(),
        }
        .append_event(
            &thread,
            &EventEnvelope {
                event_id: "seed".into(),
                seq: 1,
                timestamp: now,
                source: EventSource::Core,
                kind: "turn.transcript_item_appended".into(),
                thread_id: Some(thread.clone()),
                turn_id: Some("history".into()),
                event: RoderEvent::TranscriptItemAppended(item),
            },
        )
        .await
        .unwrap();
        let client = LocalAppClient::new(Arc::new(AppServer::new(runtime.clone())));
        let response = client
            .send_request(roder_protocol::JsonRpcRequest {
                jsonrpc: "2.0".into(),
                id: Some(json!(1)),
                method: "thread/compact".into(),
                params: Some(
                    json!({"threadId":thread,"turnId":"compact","preserveHint":"keep BLUE"}),
                ),
            })
            .await;
        let snapshot = runtime.load_thread(&thread).await.unwrap().unwrap();
        let items: Vec<_> = snapshot.turns.iter().flat_map(|turn| &turn.items).collect();
        assert!(
            !items
                .iter()
                .any(|item| matches!(item, TranscriptItem::ContextCompaction(_)))
        );
        if fail {
            assert!(
                response
                    .error
                    .unwrap()
                    .message
                    .contains("native window too large")
            );
            assert!(!items.iter().any(|item| matches!(item,TranscriptItem::ProviderMetadata(metadata) if metadata.get("compacted_input").is_some())));
        } else {
            assert!(response.error.is_none());
            assert_eq!(response.result.unwrap()["compacted"], true);
            assert!(items.iter().any(|item| matches!(item,TranscriptItem::ProviderMetadata(metadata) if metadata["compacted_input"][0]["content"] == "BLUE")));
        }
    }
}
