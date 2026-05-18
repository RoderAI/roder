use roder_api::conversation::ConversationItem;
use roder_api::session::ThreadSnapshot;

use super::*;

pub(super) async fn sessions_list(
    client: &LocalAppClient,
) -> anyhow::Result<Vec<roder_api::session::SessionMetadata>> {
    let res = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("sessions/list")),
            method: "sessions/list".to_string(),
            params: None,
        })
        .await;
    let mut sessions = decode_response::<SessionsListResult>(res)?.sessions;
    sessions.sort_by_key(|session| std::cmp::Reverse(session.updated_at));
    Ok(sessions)
}

pub(super) async fn commands_list(
    client: &LocalAppClient,
) -> anyhow::Result<Vec<CommandDescriptor>> {
    let res = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("commands/list")),
            method: "commands/list".to_string(),
            params: None,
        })
        .await;
    Ok(decode_response::<CommandsListResult>(res)?.commands)
}

impl TuiApp {
    pub(super) async fn load_session(&mut self, thread_id: String) {
        match load_snapshot(&self.client, &thread_id).await {
            Ok(Some(snapshot)) => self.apply_snapshot(thread_id, snapshot),
            Ok(None) => self.record_error(format!("session not found: {}", short_id(&thread_id))),
            Err(err) => self.record_error(format!("sessions/load failed: {err}")),
        }
    }

    pub(super) fn apply_snapshot(&mut self, thread_id: String, snapshot: ThreadSnapshot) {
        self.thread_id = thread_id.clone();
        self.active_turn_id = None;
        self.active_turn_timer = TurnTimer::default();
        self.current_turn_input_tokens = 0;
        self.current_turn_output_tokens = 0;
        self.current_turn_reasoning_tokens = None;
        self.current_turn_total_tokens = 0;
        self.context_counter_hovered = false;
        self.tool_names.clear();
        self.queued_prompts = PromptQueue::default();
        self.last_user_prompt = None;
        self.image_attachments.clear();
        self.timeline = TimelineState::default();

        if let Some(metadata) = snapshot.metadata.as_ref() {
            if let Some(provider) = metadata
                .provider
                .as_ref()
                .filter(|value| !value.trim().is_empty())
            {
                self.provider = provider.clone();
            }
            if let Some(model) = metadata
                .model
                .as_ref()
                .filter(|value| !value.trim().is_empty())
            {
                self.model = model.clone();
                self.model_context_window = context_window_for_model(model);
            }
            self.session_title = metadata.title.clone();
            self.session_message_count = metadata.message_count as usize;
        } else {
            self.session_title = None;
            self.session_message_count = 0;
        }

        let mut item_count = 0usize;
        for turn in &snapshot.turns {
            for item in &turn.items {
                item_count += 1;
                self.push_snapshot_item(item);
            }
        }

        if self.session_title.is_none() {
            self.session_title = title_from_snapshot(&snapshot);
        }
        if self.session_message_count == 0 {
            self.session_message_count = message_count_from_snapshot(&snapshot);
        }

        self.timeline.push_system(format!(
            "resumed session {} with {} saved item{}.",
            short_id(&thread_id),
            item_count,
            if item_count == 1 { "" } else { "s" }
        ));
        self.push_event(format!("resumed session {}", short_id(&thread_id)));
    }

    fn push_snapshot_item(&mut self, item: &ConversationItem) {
        match item {
            ConversationItem::UserMessage(message) => {
                let display = user_snapshot_text(message);
                self.last_user_prompt = Some(PendingPrompt::with_images(
                    display.clone(),
                    message.text.clone(),
                    message.images.clone(),
                ));
                self.timeline.push_user(display);
            }
            ConversationItem::AssistantMessage(message) => {
                self.timeline
                    .push_assistant_delta(&message.text, message.phase.clone());
            }
            ConversationItem::ReasoningSummary(summary) => {
                self.timeline.push_reasoning_delta(&summary.text);
            }
            ConversationItem::ToolCall(call) => {
                self.record_tool_requested_with_id(
                    call.id.clone(),
                    ToolTimelineEntry::new(call.name.clone(), call.arguments.clone()),
                );
            }
            ConversationItem::ToolResult(result) => {
                if !self.tool_names.contains_key(&result.id) {
                    let name = result
                        .name
                        .clone()
                        .unwrap_or_else(|| format!("tool {}", short_id(&result.id)));
                    self.record_tool_requested_with_id(
                        result.id.clone(),
                        ToolTimelineEntry::new(name, ""),
                    );
                }
                self.record_tool_completed(
                    &result.id,
                    result.is_error,
                    Some(result.result.clone()),
                );
            }
            ConversationItem::FileChange(change) => {
                self.timeline
                    .push_system(format!("file {}: {}", change.change_type, change.path));
            }
            ConversationItem::ContextCompaction(compaction) => {
                self.timeline.push_system(format!(
                    "context compacted: {}",
                    truncate(&compaction.summary, 160)
                ));
            }
            ConversationItem::Error(error) => self.timeline.push_error(error.message.clone()),
            ConversationItem::ProviderMetadata(_) => {}
        }
    }

    pub(super) fn session_exit_summary(&self) -> TuiExitSummary {
        let title = self
            .session_title
            .clone()
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| format!("Session {}", short_id(&self.thread_id)));
        TuiExitSummary {
            thread_id: self.thread_id.clone(),
            title,
            model: self.model.clone(),
            message_count: self.session_message_count,
            resume_command: format!("roder resume {}", self.thread_id),
        }
    }
}

pub(super) async fn load_snapshot(
    client: &LocalAppClient,
    thread_id: &str,
) -> anyhow::Result<Option<ThreadSnapshot>> {
    let res = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("sessions/load")),
            method: "sessions/load".to_string(),
            params: Some(
                serde_json::to_value(SessionLoadParams {
                    thread_id: thread_id.to_string(),
                })
                .unwrap(),
            ),
        })
        .await;
    Ok(decode_response::<SessionLoadResult>(res)?.snapshot)
}

fn user_snapshot_text(message: &roder_api::conversation::UserMessage) -> String {
    if message.images.is_empty() {
        return message.text.clone();
    }
    format!(
        "{}\n[{} image attachment{}]",
        message.text,
        message.images.len(),
        if message.images.len() == 1 { "" } else { "s" }
    )
}

fn title_from_snapshot(snapshot: &ThreadSnapshot) -> Option<String> {
    snapshot.turns.iter().find_map(|turn| {
        turn.items.iter().find_map(|item| match item {
            ConversationItem::UserMessage(message) if !message.text.trim().is_empty() => {
                Some(truncate(message.text.trim(), 72))
            }
            _ => None,
        })
    })
}

fn message_count_from_snapshot(snapshot: &ThreadSnapshot) -> usize {
    snapshot
        .turns
        .iter()
        .flat_map(|turn| turn.items.iter())
        .filter(|item| {
            matches!(
                item,
                ConversationItem::UserMessage(_) | ConversationItem::AssistantMessage(_)
            )
        })
        .count()
}

#[cfg(test)]
mod tests {
    use roder_api::conversation::{AssistantMessage, UserMessage};
    use roder_api::session::{SessionMetadata, TurnRecord};
    use time::OffsetDateTime;

    use super::*;

    #[test]
    fn derives_resume_title_from_first_user_message() {
        let snapshot = ThreadSnapshot {
            metadata: None,
            events: Vec::new(),
            extension_states: Vec::new(),
            turns: vec![TurnRecord {
                thread_id: "thread-a".to_string(),
                turn_id: "turn-a".to_string(),
                items: vec![ConversationItem::UserMessage(UserMessage::text(
                    "explain this repository",
                ))],
                created_at: OffsetDateTime::UNIX_EPOCH,
                completed_at: None,
            }],
        };

        assert_eq!(
            title_from_snapshot(&snapshot).as_deref(),
            Some("explain this repository")
        );
    }

    #[test]
    fn counts_user_and_assistant_messages_only() {
        let snapshot = ThreadSnapshot {
            metadata: Some(SessionMetadata {
                thread_id: "thread-a".to_string(),
                title: None,
                workspace: None,
                provider: None,
                model: None,
                runner_destination: None,
                runner_state: None,
                created_at: OffsetDateTime::UNIX_EPOCH,
                updated_at: OffsetDateTime::UNIX_EPOCH,
                message_count: 0,
            }),
            events: Vec::new(),
            extension_states: Vec::new(),
            turns: vec![TurnRecord {
                thread_id: "thread-a".to_string(),
                turn_id: "turn-a".to_string(),
                items: vec![
                    ConversationItem::UserMessage(UserMessage::text("hi")),
                    ConversationItem::AssistantMessage(AssistantMessage {
                        text: "hello".to_string(),
                        phase: None,
                    }),
                    ConversationItem::ProviderMetadata(serde_json::json!({"id": "resp_1"})),
                ],
                created_at: OffsetDateTime::UNIX_EPOCH,
                completed_at: None,
            }],
        };

        assert_eq!(message_count_from_snapshot(&snapshot), 2);
    }
}
