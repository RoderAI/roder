use std::collections::HashSet;

use roder_api::transcript::{ToolResultRecord, TranscriptItem};

/// A stopped/crashed turn may retain a call without its result. Close that
/// call in inference context only: missing output is unknown, never permission
/// to execute the old operation again. Durable history remains unchanged.
pub(crate) fn close_unanswered_calls(items: &mut Vec<TranscriptItem>) {
    let mut answered: HashSet<String> = items
        .iter()
        .filter_map(|item| match item {
            TranscriptItem::ToolResult(result) => Some(result.id.clone()),
            _ => None,
        })
        .collect();
    let unknown: Vec<_> = items.iter().filter_map(|item| match item {
        TranscriptItem::ToolCall(call) if answered.insert(call.id.clone()) =>
            Some(TranscriptItem::ToolResult(ToolResultRecord {
                id: call.id.clone(), name: Some(call.name.clone()),
                result: "The previous turn ended before this tool result was recorded. Its outcome is unknown; side effects may already have happened. Do not repeat this call. Follow the latest user request.".to_string(),
                display_payload: None, is_error: true,
            })),
        _ => None,
    }).collect();
    items.extend(unknown);
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use roder_api::extension::ExtensionRegistryBuilder;
    use roder_api::transcript::{ToolCallRecord, ToolResultRecord, TranscriptItem, UserMessage};
    use roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory;

    use crate::fake_provider::FakeInferenceEngine;
    use crate::runtime::{Runtime, RuntimeConfig, StartTurnRequest};

    #[tokio::test]
    async fn resumed_context_closes_unanswered_old_calls_without_rewriting_history() {
        let root = std::env::temp_dir().join(format!(
            "roder-interrupted-history-{}",
            uuid::Uuid::new_v4()
        ));
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(FakeInferenceEngine));
        builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
            base_path: root.clone(),
        }));
        let runtime = Runtime::new(builder.build().unwrap(), RuntimeConfig::default()).unwrap();
        let thread_id = runtime.create_thread(None).await.unwrap().thread_id;
        let old = "stopped-turn".to_string();
        let current = "new-turn".to_string();
        let call = |id: &str| {
            TranscriptItem::ToolCall(ToolCallRecord {
                id: id.to_string(),
                name: "exec_command".to_string(),
                arguments: "{}".to_string(),
            })
        };
        let completed = TranscriptItem::ToolResult(ToolResultRecord {
            id: "finished-call".to_string(),
            name: Some("exec_command".to_string()),
            result: "real result".to_string(),
            display_payload: None,
            is_error: false,
        });
        let prior = vec![
            TranscriptItem::UserMessage(UserMessage::text("old task")),
            call("finished-call"),
            completed.clone(),
            call("interrupted-call"),
        ];
        for item in &prior {
            runtime
                .persist_turn_item(&thread_id, &old, item)
                .await
                .unwrap();
        }
        runtime
            .persist_turn_item(&thread_id, &current, &call("current-call"))
            .await
            .unwrap();
        let request = StartTurnRequest {
            thread_id: thread_id.clone(),
            message: "new task".to_string(),
            images: vec![],
            provider_override: None,
            model_override: None,
            reasoning_override: None,
            workspace: root.display().to_string(),
            instructions: Default::default(),
            developer_context: None,
            task_ledger_required: false,
            service_tier_override: None,
        };
        for _ in 0..2 {
            let context = runtime
                .transcript_for_turn(&request, &current)
                .await
                .unwrap();
            let results: Vec<_> = context
                .iter()
                .filter_map(|item| match item {
                    TranscriptItem::ToolResult(result) => Some(result),
                    _ => None,
                })
                .collect();
            assert_eq!(
                results.len(),
                2,
                "old unanswered call needs an explicit unknown result"
            );
            assert!(context.contains(&completed));
            let unknown = results
                .iter()
                .find(|result| result.id == "interrupted-call")
                .unwrap();
            assert!(unknown.is_error);
            assert!(unknown.result.contains("unknown"));
            assert!(unknown.result.contains("Do not repeat"));
            assert!(!context.contains(&call("current-call")));
            assert_eq!(
                context.last(),
                Some(&TranscriptItem::UserMessage(UserMessage::text("new task")))
            );
        }
        let snapshot = runtime
            .thread_store
            .as_ref()
            .unwrap()
            .load_thread(&thread_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            snapshot
                .turns
                .iter()
                .find(|turn| turn.turn_id == old)
                .unwrap()
                .items,
            prior
        );
        drop(runtime);
        std::fs::remove_dir_all(root).unwrap();
    }
}
