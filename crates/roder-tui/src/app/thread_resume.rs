use roder_protocol::{
    Item, Thread, ThreadItemStatus, ThreadListParams, ThreadListResult, ThreadReadParams,
    ThreadReadResult,
};

use super::*;

pub(super) async fn threads_list<C>(client: &C) -> anyhow::Result<Vec<Thread>>
where
    C: AppClient,
{
    let res = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("thread/list")),
            method: "thread/list".to_string(),
            params: Some(serde_json::to_value(ThreadListParams { limit: None }).unwrap()),
        })
        .await;
    let mut threads = Vec::new();
    for thread in decode_response::<ThreadListResult>(res)?.data {
        if let Ok(Some(full_thread)) = load_thread(client, &thread.id).await
            && thread_has_user_message(&full_thread)
        {
            threads.push(full_thread);
        }
    }
    threads.sort_by_key(|thread| std::cmp::Reverse(thread.updated_at));
    Ok(threads)
}

pub(super) async fn commands_list<C>(client: &C) -> anyhow::Result<Vec<CommandDescriptor>>
where
    C: AppClient,
{
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

impl<C> TuiApp<C>
where
    C: AppClient,
{
    pub(super) async fn load_thread(&mut self, thread_id: String) {
        match load_thread(&self.client, &thread_id).await {
            Ok(Some(thread)) => self.apply_thread(thread),
            Ok(None) => self.record_error(format!("thread not found: {}", short_id(&thread_id))),
            Err(err) => self.record_error(format!("thread/read failed: {err}")),
        }
    }

    pub(super) fn apply_thread(&mut self, thread: Thread) {
        let thread_id = thread.id.clone();
        let active_turn_id = thread.status.active_turn_id.clone();
        self.thread_id = thread_id.clone();
        self.active_turn_id = active_turn_id;
        self.active_turn_timer = TurnTimer::default();
        if self.active_turn_id.is_some() {
            self.active_turn_timer.start(Instant::now());
        }
        self.current_turn_input_tokens = 0;
        self.current_turn_output_tokens = 0;
        self.current_turn_reasoning_tokens = None;
        self.current_turn_total_tokens = 0;
        self.context_counter_hovered = false;
        self.tool_names.clear();
        self.exec_session_tools.clear();
        self.stdin_tool_sessions.clear();
        self.hidden_stdin_tools.clear();
        self.queued_prompts = PromptQueue::default();
        self.last_user_prompt = None;
        self.image_attachments.clear();
        self.timeline = TimelineState::default();

        if !thread.model_provider.trim().is_empty() {
            self.provider = thread.model_provider.clone();
        }
        if !thread.model.trim().is_empty() {
            self.model = thread.model.clone();
        }
        self.thread_title = thread
            .name
            .clone()
            .filter(|title| !title.trim().is_empty())
            .or_else(|| (!thread.preview.trim().is_empty()).then(|| thread.preview.clone()));
        self.thread_message_count = message_count_from_thread(&thread);

        let mut item_count = 0usize;
        for turn in thread.turns.as_deref().unwrap_or_default() {
            for item in &turn.items {
                item_count += 1;
                self.push_item(item);
            }
        }

        if self.thread_title.is_none() {
            self.thread_title = title_from_thread(&thread);
        }

        self.timeline.push_system(format!(
            "resumed thread {} with {} saved item{}.",
            short_id(&thread_id),
            item_count,
            if item_count == 1 { "" } else { "s" }
        ));
        self.push_event(format!("resumed thread {}", short_id(&thread_id)));
    }

    fn push_item(&mut self, item: &Item) {
        match item {
            Item::UserMessage { text, images, .. } => {
                let display = text.clone();
                self.last_user_prompt = Some(PendingPrompt::with_images(
                    display.clone(),
                    display.clone(),
                    images.clone(),
                ));
                self.timeline.push_user(display);
            }
            Item::AgentMessage { text, phase, .. } => {
                self.timeline
                    .push_assistant_delta_immediate(text, phase.clone());
            }
            Item::Reasoning {
                content, summary, ..
            } => {
                let text = if content.is_empty() {
                    reasoning_blocks_text(summary)
                } else {
                    reasoning_blocks_text(content)
                };
                if !text.trim().is_empty() {
                    self.timeline.push_reasoning_delta(&text);
                }
            }
            Item::ToolExecution {
                tool_call_id,
                tool_name,
                status,
                input,
                output,
                error,
                ..
            } => {
                let tool_id = tool_call_id.clone();
                if !self.tool_names.contains_key(&tool_id) {
                    let arguments = input.as_ref().map(ToString::to_string).unwrap_or_default();
                    self.record_tool_requested_with_id(
                        tool_id.clone(),
                        ToolTimelineEntry::new(tool_name.clone(), arguments),
                    );
                }
                if !matches!(status, ThreadItemStatus::InProgress) {
                    self.record_tool_completed(
                        &tool_id,
                        matches!(status, ThreadItemStatus::Failed),
                        error.clone().or_else(|| output.clone()),
                    );
                }
            }
            Item::Compaction { summary, .. } => {
                if !summary.trim().is_empty() {
                    self.timeline.push_system(summary.clone());
                }
            }
            Item::Error { message, .. } => {
                self.timeline.push_error(message.clone());
            }
            Item::Raw { payload, .. } => {
                if let Some(text) = payload.as_str().filter(|text| !text.trim().is_empty()) {
                    self.timeline.push_system(text.to_string());
                }
            }
        }
    }

    pub(super) fn thread_exit_summary(&self) -> TuiExitSummary {
        let title = self
            .thread_title
            .clone()
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| format!("Thread {}", short_id(&self.thread_id)));
        TuiExitSummary {
            thread_id: self.thread_id.clone(),
            title,
            model: self.model.clone(),
            message_count: self.thread_message_count,
            resume_command: format!("roder resume {}", self.thread_id),
        }
    }
}

fn reasoning_blocks_text(blocks: &[String]) -> String {
    blocks
        .iter()
        .filter(|block| !block.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub(super) async fn load_thread<C>(client: &C, thread_id: &str) -> anyhow::Result<Option<Thread>>
where
    C: AppClient,
{
    let res = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("thread/read")),
            method: "thread/read".to_string(),
            params: Some(
                serde_json::to_value(ThreadReadParams {
                    thread_id: thread_id.to_string(),
                    include_turns: true,
                })
                .unwrap(),
            ),
        })
        .await;
    Ok(decode_response::<ThreadReadResult>(res)?.thread)
}

fn title_from_thread(thread: &Thread) -> Option<String> {
    thread.turns.as_ref()?.iter().find_map(|turn| {
        turn.items.iter().find_map(|item| {
            if let Item::UserMessage { text, .. } = item {
                (!text.trim().is_empty()).then(|| truncate(text.trim(), 72))
            } else {
                None
            }
        })
    })
}

fn message_count_from_thread(thread: &Thread) -> usize {
    thread
        .turns
        .as_deref()
        .unwrap_or_default()
        .iter()
        .flat_map(|turn| turn.items.iter())
        .filter(|item| matches!(item, Item::UserMessage { .. } | Item::AgentMessage { .. }))
        .count()
}

fn thread_has_user_message(thread: &Thread) -> bool {
    thread
        .turns
        .as_deref()
        .unwrap_or_default()
        .iter()
        .flat_map(|turn| turn.items.iter())
        .any(|item| matches!(item, Item::UserMessage { text, .. } if !text.trim().is_empty()))
}

#[cfg(test)]
mod tests {
    use roder_protocol::{ThreadStatus, Turn};

    use super::*;

    fn test_thread(items: Vec<Item>) -> Thread {
        Thread {
            id: "thread-a".to_string(),
            preview: String::new(),
            model_provider: "mock".to_string(),
            model: "mock".to_string(),
            selection_mode: None,
            created_at: 0,
            updated_at: 0,
            status: ThreadStatus {
                kind: "idle".to_string(),
                active_turn_id: None,
                active_flags: Vec::new(),
            },
            cwd: "/tmp".to_string(),
            workspace_id: None,
            root_id: None,
            name: None,
            usage: None,
            turns: Some(vec![Turn {
                id: "turn-a".to_string(),
                items,
                items_view: "all".to_string(),
                status: "completed".to_string(),
                error: None,
                started_at: None,
                completed_at: None,
                duration_ms: None,
                usage: None,
            }]),
        }
    }

    fn user_message(text: &str) -> Item {
        Item::UserMessage {
            id: "userMessage-id".to_string(),
            text: text.to_string(),
            images: Vec::new(),
            status: Some(ThreadItemStatus::Completed),
        }
    }

    fn agent_message(text: &str) -> Item {
        Item::AgentMessage {
            id: "agentMessage-id".to_string(),
            text: text.to_string(),
            phase: None,
            status: Some(ThreadItemStatus::Completed),
        }
    }

    fn reasoning(text: &str) -> Item {
        Item::Reasoning {
            id: "reasoning-id".to_string(),
            summary: Vec::new(),
            content: vec![text.to_string()],
            status: Some(ThreadItemStatus::Completed),
        }
    }

    #[test]
    fn derives_resume_title_from_first_user_message() {
        let thread = test_thread(vec![user_message("explain this repository")]);

        assert_eq!(
            title_from_thread(&thread).as_deref(),
            Some("explain this repository")
        );
    }

    #[test]
    fn counts_user_and_assistant_messages_only() {
        let thread = test_thread(vec![
            user_message("hi"),
            agent_message("hello"),
            reasoning("thinking"),
        ]);

        assert_eq!(message_count_from_thread(&thread), 2);
    }

    #[test]
    fn detects_threads_with_user_messages() {
        let with_user = test_thread(vec![user_message("hi")]);
        let assistant_only = test_thread(vec![agent_message("hello")]);

        assert!(thread_has_user_message(&with_user));
        assert!(!thread_has_user_message(&assistant_only));
    }
}
