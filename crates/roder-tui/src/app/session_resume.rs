use roder_protocol::{
    DesktopItem, DesktopThread, ThreadListParams, ThreadListResult, ThreadReadParams,
    ThreadReadResult,
};

use super::*;

pub(super) async fn threads_list(client: &LocalAppClient) -> anyhow::Result<Vec<DesktopThread>> {
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
        if let Ok(Some(full_thread)) = load_thread(client, &thread.id).await {
            if thread_has_user_message(&full_thread) {
                threads.push(full_thread);
            }
        }
    }
    threads.sort_by_key(|thread| std::cmp::Reverse(thread.updated_at));
    Ok(threads)
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
        match load_thread(&self.client, &thread_id).await {
            Ok(Some(thread)) => self.apply_thread(thread),
            Ok(None) => self.record_error(format!("session not found: {}", short_id(&thread_id))),
            Err(err) => self.record_error(format!("thread/read failed: {err}")),
        }
    }

    pub(super) fn apply_thread(&mut self, thread: DesktopThread) {
        let thread_id = thread.id.clone();
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

        if !thread.model_provider.trim().is_empty() {
            self.provider = thread.model_provider.clone();
        }
        self.session_title = thread
            .name
            .clone()
            .filter(|title| !title.trim().is_empty())
            .or_else(|| (!thread.preview.trim().is_empty()).then(|| thread.preview.clone()));
        self.session_message_count = message_count_from_thread(&thread);

        let mut item_count = 0usize;
        for turn in thread.turns.as_deref().unwrap_or_default() {
            for item in &turn.items {
                item_count += 1;
                self.push_desktop_item(item);
            }
        }

        if self.session_title.is_none() {
            self.session_title = title_from_thread(&thread);
        }

        self.timeline.push_system(format!(
            "resumed session {} with {} saved item{}.",
            short_id(&thread_id),
            item_count,
            if item_count == 1 { "" } else { "s" }
        ));
        self.push_event(format!("resumed session {}", short_id(&thread_id)));
    }

    fn push_desktop_item(&mut self, item: &DesktopItem) {
        match item.kind.as_str() {
            "userMessage" => {
                let display = item.text.clone().unwrap_or_default();
                self.last_user_prompt = Some(PendingPrompt::with_images(
                    display.clone(),
                    display.clone(),
                    Vec::new(),
                ));
                self.timeline.push_user(display);
            }
            "agentMessage" => {
                self.timeline.push_assistant_delta_immediate(
                    item.text.as_deref().unwrap_or_default(),
                    item.phase.clone(),
                );
            }
            "reasoning" => {
                self.timeline
                    .push_reasoning_delta(item.text.as_deref().unwrap_or_default());
            }
            "toolMessage" => {
                let tool_id = item.tool_call_id.clone().unwrap_or_else(|| item.id.clone());
                if !self.tool_names.contains_key(&tool_id) {
                    let name = item
                        .tool_name
                        .clone()
                        .unwrap_or_else(|| format!("tool {}", short_id(&tool_id)));
                    self.record_tool_requested_with_id(
                        tool_id.clone(),
                        ToolTimelineEntry::new(name, ""),
                    );
                }
                self.record_tool_completed(
                    &tool_id,
                    item.status.as_deref() == Some("failed"),
                    item.text
                        .clone()
                        .or_else(|| item.payload.as_ref().map(ToString::to_string)),
                );
            }
            kind if kind.starts_with("tool.") || kind == "toolCall" => {
                let tool_id = item.tool_call_id.clone().unwrap_or_else(|| item.id.clone());
                let name = item
                    .tool_name
                    .clone()
                    .or_else(|| kind.strip_prefix("tool.").map(str::to_string))
                    .unwrap_or_else(|| "tool".to_string());
                let arguments = item
                    .payload
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default();
                self.record_tool_requested_with_id(
                    tool_id,
                    ToolTimelineEntry::new(name, arguments),
                );
            }
            "error" => {
                self.timeline
                    .push_error(item.text.clone().unwrap_or_else(|| "error".to_string()));
            }
            _ => {
                if let Some(text) = item.text.as_ref().filter(|text| !text.trim().is_empty()) {
                    self.timeline.push_system(text.clone());
                }
            }
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

pub(super) async fn load_thread(
    client: &LocalAppClient,
    thread_id: &str,
) -> anyhow::Result<Option<DesktopThread>> {
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

fn title_from_thread(thread: &DesktopThread) -> Option<String> {
    thread.turns.as_ref()?.iter().find_map(|turn| {
        turn.items.iter().find_map(|item| {
            (item.kind == "userMessage")
                .then(|| item.text.as_deref())
                .flatten()
                .filter(|text| !text.trim().is_empty())
                .map(|text| truncate(text.trim(), 72))
        })
    })
}

fn message_count_from_thread(thread: &DesktopThread) -> usize {
    thread
        .turns
        .as_deref()
        .unwrap_or_default()
        .iter()
        .flat_map(|turn| turn.items.iter())
        .filter(|item| matches!(item.kind.as_str(), "userMessage" | "agentMessage"))
        .count()
}

fn thread_has_user_message(thread: &DesktopThread) -> bool {
    thread
        .turns
        .as_deref()
        .unwrap_or_default()
        .iter()
        .flat_map(|turn| turn.items.iter())
        .any(|item| {
            item.kind == "userMessage"
                && item
                    .text
                    .as_deref()
                    .is_some_and(|text| !text.trim().is_empty())
        })
}

#[cfg(test)]
mod tests {
    use roder_protocol::{DesktopThreadStatus, DesktopTurn};

    use super::*;

    fn test_thread(items: Vec<DesktopItem>) -> DesktopThread {
        DesktopThread {
            id: "thread-a".to_string(),
            session_id: "thread-a".to_string(),
            preview: String::new(),
            model_provider: "mock".to_string(),
            created_at: 0,
            updated_at: 0,
            status: DesktopThreadStatus {
                kind: "idle".to_string(),
                active_flags: Vec::new(),
            },
            cwd: "/tmp".to_string(),
            name: None,
            turns: Some(vec![DesktopTurn {
                id: "turn-a".to_string(),
                items,
                items_view: "all".to_string(),
                status: "completed".to_string(),
                error: None,
                started_at: None,
                completed_at: None,
                duration_ms: None,
            }]),
        }
    }

    fn item(kind: &str, text: Option<&str>) -> DesktopItem {
        DesktopItem {
            id: format!("{kind}-id"),
            kind: kind.to_string(),
            text: text.map(str::to_string),
            status: None,
            phase: None,
            tool_name: None,
            tool_call_id: None,
            payload: None,
        }
    }

    #[test]
    fn derives_resume_title_from_first_user_message() {
        let thread = test_thread(vec![item("userMessage", Some("explain this repository"))]);

        assert_eq!(
            title_from_thread(&thread).as_deref(),
            Some("explain this repository")
        );
    }

    #[test]
    fn counts_user_and_assistant_messages_only() {
        let thread = test_thread(vec![
            item("userMessage", Some("hi")),
            item("agentMessage", Some("hello")),
            item("reasoning", Some("thinking")),
        ]);

        assert_eq!(message_count_from_thread(&thread), 2);
    }

    #[test]
    fn detects_threads_with_user_messages() {
        let with_user = test_thread(vec![item("userMessage", Some("hi"))]);
        let assistant_only = test_thread(vec![item("agentMessage", Some("hello"))]);

        assert!(thread_has_user_message(&with_user));
        assert!(!thread_has_user_message(&assistant_only));
    }
}
