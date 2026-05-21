use roder_api::artifacts::{
    ContextArtifact, ContextArtifactKind, ContextArtifactReference, format_artifact_reference,
};
use roder_api::catalog::lookup_model;
use roder_api::context::{ContextBlockKind, ContextPlan, ContextQuery};
use roder_api::conversation::{
    ContextCompactionRecord, ConversationItem, ToolResultRecord, UserMessage,
};
use roder_api::events::*;
use time::OffsetDateTime;

use crate::runtime::{Runtime, StartTurnRequest};

impl Runtime {
    pub(crate) async fn conversation_for_turn(
        &self,
        req: &StartTurnRequest,
        turn_id: &TurnId,
        model: &str,
    ) -> anyhow::Result<Vec<ConversationItem>> {
        let context_plan = self.assemble_context(req, turn_id).await?;
        let mut conversation = Vec::new();
        for block in context_plan.blocks {
            if matches!(
                block.kind,
                ContextBlockKind::Instruction
                    | ContextBlockKind::Memory
                    | ContextBlockKind::RepositoryFact
                    | ContextBlockKind::PriorSummary
            ) {
                conversation.push(ConversationItem::UserMessage(UserMessage {
                    text: block.text,
                    images: Vec::new(),
                }));
            }
        }
        conversation.extend(self.prior_conversation(&req.thread_id, turn_id).await?);
        conversation.push(ConversationItem::UserMessage(UserMessage {
            text: req.message.clone(),
            images: req.images.clone(),
        }));
        self.compact_conversation_if_needed(&req.thread_id, turn_id, model, conversation)
            .await
    }

    async fn assemble_context(
        &self,
        req: &StartTurnRequest,
        turn_id: &TurnId,
    ) -> anyhow::Result<ContextPlan> {
        self.emit(RoderEvent::ContextAssemblyStarted(ContextAssemblyStarted {
            thread_id: req.thread_id.clone(),
            turn_id: turn_id.clone(),
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;

        let query = ContextQuery {
            thread_id: req.thread_id.clone(),
            turn_id: turn_id.clone(),
            prompt: req.message.clone(),
            token_budget: None,
        };
        let mut blocks = Vec::new();
        for provider in &self.registry.context_providers {
            for block in provider.blocks(&query).await? {
                self.emit(RoderEvent::ContextBlockAdded(ContextBlockAdded {
                    thread_id: req.thread_id.clone(),
                    turn_id: turn_id.clone(),
                    block_type: format!("{:?}", block.kind),
                    timestamp: OffsetDateTime::now_utc(),
                }))
                .await;
                blocks.push(block);
            }
        }
        let plan = if let Some(planner) = self.registry.context_planners.first() {
            planner.plan(&query, blocks).await?
        } else {
            ContextPlan { blocks }
        };
        self.emit(RoderEvent::ContextAssemblyCompleted(
            ContextAssemblyCompleted {
                thread_id: req.thread_id.clone(),
                turn_id: turn_id.clone(),
                timestamp: OffsetDateTime::now_utc(),
            },
        ))
        .await;
        Ok(plan)
    }

    async fn prior_conversation(
        &self,
        thread_id: &ThreadId,
        current_turn_id: &TurnId,
    ) -> anyhow::Result<Vec<ConversationItem>> {
        let Some(store) = &self.session_store else {
            return Ok(Vec::new());
        };
        let Some(snapshot) = store.load_session(thread_id).await? else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for turn in snapshot.turns {
            if &turn.turn_id == current_turn_id {
                continue;
            }
            out.extend(turn.items);
        }
        Ok(out)
    }

    pub(crate) async fn compact_conversation_if_needed(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        model: &str,
        conversation: Vec<ConversationItem>,
    ) -> anyhow::Result<Vec<ConversationItem>> {
        let cfg = self.status().await;
        if lookup_model(model).is_some_and(|entry| entry.supports_compaction) {
            return Ok(conversation);
        }
        let threshold = cfg
            .auto_compact_token_limit
            .or_else(|| lookup_model(model).map(|entry| entry.auto_compact_token_limit))
            .unwrap_or(0);
        if threshold == 0 || estimate_tokens(&conversation) < threshold {
            return Ok(conversation);
        }
        let suffix = conversation
            .last()
            .cloned()
            .into_iter()
            .collect::<Vec<ConversationItem>>();
        let prior_window: Vec<ConversationItem> = conversation
            .iter()
            .take(conversation.len().saturating_sub(1))
            .cloned()
            .collect();
        let history_artifact = if prior_window.is_empty() {
            None
        } else {
            let artifact_id = format!("history_{turn_id}");
            let store = self.context_artifact_store_for_thread(thread_id);
            let body = format_conversation_window(&prior_window);
            let artifact = store.write(
                thread_id,
                turn_id,
                ContextArtifactKind::ChatHistory,
                &artifact_id,
                None,
                "chat_history",
                body.as_bytes(),
            )?;
            self.emit(RoderEvent::ContextArtifactCreated(ContextArtifactCreated {
                thread_id: thread_id.clone(),
                turn_id: turn_id.clone(),
                artifact: artifact.clone(),
                timestamp: OffsetDateTime::now_utc(),
            }))
            .await;
            Some(artifact)
        };
        let summary = summarize_conversation(&conversation, history_artifact.as_ref());
        let compaction = ConversationItem::ContextCompaction(ContextCompactionRecord {
            summary,
            artifact_id: history_artifact.as_ref().map(|artifact| artifact.id.clone()),
        });
        self.persist_turn_item(thread_id, turn_id, &compaction)
            .await?;
        let mut compacted = vec![compaction];
        compacted.extend(suffix);
        Ok(compacted)
    }
}

fn estimate_tokens(items: &[ConversationItem]) -> u32 {
    let chars: usize = items.iter().map(item_text_len).sum();
    u32::try_from(chars.div_ceil(4)).unwrap_or(u32::MAX)
}

fn item_text_len(item: &ConversationItem) -> usize {
    match item {
        ConversationItem::UserMessage(message) => message.text.len(),
        ConversationItem::AssistantMessage(message) => message.text.len(),
        ConversationItem::ReasoningSummary(summary) => summary.text.len(),
        ConversationItem::ToolCall(call) => call.arguments.len() + call.name.len(),
        ConversationItem::ToolResult(result) => result.result.len(),
        ConversationItem::FileChange(change) => change.path.len() + change.change_type.len(),
        ConversationItem::ContextCompaction(compaction) => compaction.summary.len(),
        ConversationItem::Error(error) => error.message.len(),
        ConversationItem::ProviderMetadata(value) => value.to_string().len(),
    }
}

fn format_conversation_window(items: &[ConversationItem]) -> String {
    items
        .iter()
        .filter_map(|item| serde_json::to_string(item).ok())
        .collect::<Vec<_>>()
        .join("\n")
}

fn summarize_conversation(
    items: &[ConversationItem],
    history_artifact: Option<&ContextArtifact>,
) -> String {
    let mut lines = vec!["Previous conversation was compacted. Key retained facts:".to_string()];
    for item in items.iter().take(items.len().saturating_sub(1)) {
        match item {
            ConversationItem::UserMessage(message) => {
                lines.push(format!("- user: {}", truncate(&message.text)));
            }
            ConversationItem::AssistantMessage(message) => {
                lines.push(format!("- assistant: {}", truncate(&message.text)));
            }
            ConversationItem::ToolResult(ToolResultRecord { name, result, .. }) => {
                let name = name.as_deref().unwrap_or("tool");
                lines.push(format!("- {name} result: {}", truncate(result)));
            }
            ConversationItem::ContextCompaction(compaction) => {
                lines.push(format!(
                    "- prior summary: {}",
                    truncate(&compaction.summary)
                ));
            }
            _ => {}
        }
    }
    if let Some(artifact) = history_artifact {
        lines.push(String::new());
        lines.push(format_artifact_reference(&ContextArtifactReference::from_artifact(
            artifact,
            "chat_history",
        )));
    }
    lines.join("\n")
}

fn truncate(text: &str) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    const LIMIT: usize = 240;
    if normalized.len() <= LIMIT {
        normalized
    } else {
        format!("{}...", &normalized[..LIMIT])
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use roder_api::artifacts::ContextArtifactKind;
    use roder_api::catalog::PROVIDER_MOCK;
    use roder_api::conversation::{AssistantMessage, UserMessage};
    use roder_api::extension::ExtensionRegistryBuilder;

    use crate::fake_provider::FakeInferenceEngine;
    use crate::runtime::{Runtime, RuntimeConfig};

    use super::*;

    #[tokio::test]
    async fn compact_conversation_keeps_summary_and_current_prompt() {
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(FakeInferenceEngine));
        let runtime = Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                default_provider: PROVIDER_MOCK.to_string(),
                default_model: "mock".to_string(),
                reasoning: None,
                auto_compact_token_limit: Some(1),
                hosted_web_search: roder_api::inference::HostedWebSearchConfig::disabled(),
                model_edit_tools: std::collections::HashMap::new(),
                model_parallel_tool_calls: std::collections::HashMap::new(),
                workspace: None,
                policy_mode: roder_api::policy_mode::PolicyMode::Default,
                remote_runner_destination: None,
                team_data_dir: None,
                session_dir: None,
                context_artifact_dir: None,
            },
        )
        .unwrap();
        let compacted = runtime
            .compact_conversation_if_needed(
                &"thread".to_string(),
                &"turn".to_string(),
                "mock",
                vec![
                    ConversationItem::UserMessage(UserMessage {
                        text: "very large old context".repeat(20),
                        images: Vec::new(),
                    }),
                    ConversationItem::AssistantMessage(AssistantMessage {
                        text: "old answer".to_string(),
                        phase: None,
                    }),
                    ConversationItem::UserMessage(UserMessage {
                        text: "current prompt".to_string(),
                        images: Vec::new(),
                    }),
                ],
            )
            .await
            .unwrap();

        assert!(matches!(
            &compacted[0],
            ConversationItem::ContextCompaction(summary)
                if summary.summary.contains("Previous conversation was compacted")
        ));
        assert!(matches!(
            &compacted[1],
            ConversationItem::UserMessage(message) if message.text == "current prompt"
        ));
    }

    #[tokio::test]
    async fn compact_conversation_preserves_chat_history_artifact_reference() {
        let sessions_base =
            std::env::temp_dir().join(format!("roder-compaction-sessions-{}", uuid::Uuid::new_v4()));
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(FakeInferenceEngine));
        let runtime = Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                default_provider: PROVIDER_MOCK.to_string(),
                default_model: "mock".to_string(),
                reasoning: None,
                auto_compact_token_limit: Some(1),
                hosted_web_search: roder_api::inference::HostedWebSearchConfig::disabled(),
                model_edit_tools: std::collections::HashMap::new(),
                model_parallel_tool_calls: std::collections::HashMap::new(),
                workspace: None,
                policy_mode: roder_api::policy_mode::PolicyMode::Default,
                remote_runner_destination: None,
                team_data_dir: None,
                session_dir: Some(sessions_base.clone()),
                context_artifact_dir: None,
            },
        )
        .unwrap();
        let mut events = runtime.subscribe_events();
        let compacted = runtime
            .compact_conversation_if_needed(
                &"thread".to_string(),
                &"turn".to_string(),
                "mock",
                vec![
                    ConversationItem::UserMessage(UserMessage {
                        text: "needle detail only in full history".to_string(),
                        images: Vec::new(),
                    }),
                    ConversationItem::AssistantMessage(AssistantMessage {
                        text: "old answer".to_string(),
                        phase: None,
                    }),
                    ConversationItem::UserMessage(UserMessage {
                        text: "current prompt".to_string(),
                        images: Vec::new(),
                    }),
                ],
            )
            .await
            .unwrap();

        let ConversationItem::ContextCompaction(record) = &compacted[0] else {
            panic!("expected compaction item");
        };
        assert!(record.summary.contains("[artifact: chat_history"));
        assert!(record.summary.contains("read_artifact"));
        let artifact_id = record
            .artifact_id
            .as_deref()
            .expect("compaction should record chat-history artifact id");
        assert_eq!(artifact_id, "history_turn");

        let store = runtime.context_artifact_store_for_thread("thread");
        let grep = store
            .grep("thread", artifact_id, "needle detail only in full history")
            .unwrap();
        assert_eq!(grep.matches.len(), 1);

        let mut saw_created = false;
        while let Ok(envelope) = events.try_recv() {
            if matches!(
                envelope.event,
                RoderEvent::ContextArtifactCreated(ref created)
                    if created.artifact.kind == ContextArtifactKind::ChatHistory
                        && created.artifact.id == artifact_id
            ) {
                saw_created = true;
            }
        }
        assert!(saw_created, "expected ContextArtifactCreated for chat history");
    }

    #[tokio::test]
    async fn openai_server_side_compaction_models_skip_local_summary_compaction() {
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(FakeInferenceEngine));
        let runtime = Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                default_provider: PROVIDER_MOCK.to_string(),
                default_model: "mock".to_string(),
                reasoning: None,
                auto_compact_token_limit: Some(1),
                hosted_web_search: roder_api::inference::HostedWebSearchConfig::disabled(),
                model_edit_tools: std::collections::HashMap::new(),
                model_parallel_tool_calls: std::collections::HashMap::new(),
                workspace: None,
                policy_mode: roder_api::policy_mode::PolicyMode::Default,
                remote_runner_destination: None,
                team_data_dir: None,
                session_dir: None,
                context_artifact_dir: None,
            },
        )
        .unwrap();
        let conversation = vec![
            ConversationItem::UserMessage(UserMessage {
                text: "very large old context".repeat(20),
                images: Vec::new(),
            }),
            ConversationItem::AssistantMessage(AssistantMessage {
                text: "old answer".to_string(),
                phase: None,
            }),
            ConversationItem::UserMessage(UserMessage {
                text: "current prompt".to_string(),
                images: Vec::new(),
            }),
        ];

        let compacted = runtime
            .compact_conversation_if_needed(
                &"thread".to_string(),
                &"turn".to_string(),
                "gpt-5.5",
                conversation.clone(),
            )
            .await
            .unwrap();

        assert_eq!(compacted, conversation);
    }
}
