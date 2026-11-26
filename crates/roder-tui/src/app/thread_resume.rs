#[cfg(test)]
mod pagination_tests;

use roder_api::lifecycle::{TurnLifecycleSnapshot, TurnLifecycleState};
use roder_protocol::{
    Thread, ThreadHistoryReadParams, ThreadHistoryReadResult, ThreadItemStatus, ThreadListParams,
    ThreadListResult, ThreadReadParams, ThreadReadResult, ThreadResumeParams, ThreadResumeResult,
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
            params: Some(
                serde_json::to_value(ThreadListParams {
                    limit: Some(100),
                    cursor: None,
                })
                .unwrap(),
            ),
        })
        .await;
    let mut threads = decode_response::<ThreadListResult>(res)?.data;
    threads.retain(|thread| {
        !thread.preview.trim().is_empty() || !thread.name.as_deref().unwrap_or("").trim().is_empty()
    });
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
        match read_thread(&self.client, &thread_id).await {
            Ok(read) => match read.thread {
                Some(thread) => self.apply_resumed_thread(thread, read.lifecycle, read.before),
                None => self.record_error(format!("thread not found: {}", short_id(&thread_id))),
            },
            Err(err) => self.record_error(format!("thread resume failed: {err}")),
        }
    }

    pub(super) fn apply_thread(&mut self, thread: Thread) {
        self.apply_thread_with_lifecycle(thread, TurnLifecycleSnapshot::default());
    }

    pub(super) fn apply_thread_with_lifecycle(
        &mut self,
        thread: Thread,
        lifecycle: TurnLifecycleSnapshot,
    ) {
        self.apply_resumed_thread(thread, lifecycle, None);
    }

    pub(super) fn apply_resumed_thread(
        &mut self,
        thread: Thread,
        lifecycle: TurnLifecycleSnapshot,
        before: Option<String>,
    ) {
        let thread_id = thread.id.clone();
        let active_turn_id = thread.status.active_turn_id.clone();
        if self.thread_id != thread_id {
            self.agent_activity = super::agent_activity::AgentActivityState::default();
        }
        self.thread_id = thread_id.clone();
        self.active_turn_id = active_turn_id;
        self.active_turn_timer = TurnTimer::default();
        if self.active_turn_id.is_some() {
            self.active_turn_timer.start(Instant::now());
            self.progress.set(super::TerminalProgress::Working);
        } else {
            self.progress.set(super::TerminalProgress::Idle);
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
        self.prompt_admission.reset_rendered_thread(&thread_id);
        self.last_user_prompt = None;
        self.image_attachments.clear();
        self.timeline = TimelineState::default();
        self.timeline
            .set_image_origin(super::media::ImageOrigin::from_thread(&thread));
        self.resume_history = ResumeHistoryState::default();

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

        let mut seen = HashSet::new();
        let items = take_unseen_items(thread.turns.as_deref().unwrap_or_default(), &mut seen);
        let item_count = items.len();
        let (older_items, visible_items) = split_resume_items(items);
        self.resume_history = ResumeHistoryState {
            older_items,
            loaded_items: visible_items.len(),
            total_items: item_count,
            before,
            seen,
            pending: None,
        };
        self.replay_resume_items(&visible_items);

        if self.thread_title.is_none() {
            self.thread_title = title_from_thread(&thread);
        }

        self.timeline.push_system(format!(
            "resumed thread {} with {item_count} {}saved item{}.",
            short_id(&thread_id),
            if self.resume_history.before.is_some() {
                "recent "
            } else {
                ""
            },
            plural_s(item_count),
        ));
        if self.resume_history.has_older_items() {
            self.timeline
                .push_system("scroll up to load older saved items.");
        }
        if let Some(warning) = lifecycle_recovery_warning(&thread_id, &lifecycle) {
            self.timeline.push_system(warning);
        }
        self.timeline.follow_latest();
        self.push_event(format!("resumed thread {}", short_id(&thread_id)));
    }

    pub(super) fn load_older_resume_history_if_needed(&mut self) {
        if self.focused_thread_id() != self.thread_id || self.timeline.scroll_offset() > 0 {
            return;
        }

        if !self.resume_history.older_items.is_empty() {
            let batch_start = self
                .resume_history
                .older_items
                .len()
                .saturating_sub(RESUME_OLDER_BATCH_ITEMS);
            let batch = self.resume_history.older_items.split_off(batch_start);
            self.prepend_resume_batch(&batch);
            return;
        }

        if self.resume_history.pending.is_none()
            && let Some(before) = self.resume_history.before.clone()
        {
            let client = self.client.clone();
            let thread_id = self.thread_id.clone();
            self.resume_history.pending = Some(tokio::spawn(async move {
                let response = tokio::time::timeout(
                    Duration::from_secs(15),
                    request_history_page(&client, &thread_id, Some(before)),
                )
                .await
                .map_err(|_| anyhow::anyhow!("history request timed out"))?;
                decode_response(response)
            }));
        }
    }

    pub(super) async fn poll_older_resume_history(&mut self) -> bool {
        if !self
            .resume_history
            .pending
            .as_ref()
            .is_some_and(|pending| pending.is_finished())
        {
            return false;
        }
        let pending = self
            .resume_history
            .pending
            .take()
            .expect("finished history request");
        let result = pending
            .await
            .map_err(anyhow::Error::new)
            .and_then(|result| result);
        if self.focused_thread_id() == self.thread_id {
            let result = result.and_then(|page| self.apply_older_resume_page(page));
            if let Err(error) = result {
                self.record_error(format!("could not load older saved items: {error}"));
            }
        }
        true
    }

    fn apply_older_resume_page(&mut self, page: ThreadHistoryReadResult) -> anyhow::Result<()> {
        let thread = page
            .thread
            .filter(|thread| thread.id == self.thread_id)
            .ok_or_else(|| anyhow::anyhow!("thread history is unavailable"))?;
        let before = page_cursor(page.has_more, page.before)?;
        anyhow::ensure!(
            before.is_none() || before != self.resume_history.before,
            "history cursor did not advance"
        );
        self.resume_history.before = before;
        let batch = take_unseen_items(
            thread.turns.as_deref().unwrap_or_default(),
            &mut self.resume_history.seen,
        );
        self.resume_history.total_items += batch.len();
        self.prepend_resume_batch(&batch);
        Ok(())
    }

    fn prepend_resume_batch(&mut self, batch: &[Item]) {
        if batch.is_empty() {
            return;
        }
        let before_height = self.timeline_rendered_height();
        let before_scroll = self.timeline.scroll_offset();
        self.replay_older_resume_items(batch);
        self.resume_history.loaded_items += batch.len();
        let after_height = self.timeline_rendered_height();
        self.timeline.preserve_scroll_after_prepend(
            before_scroll,
            after_height.saturating_sub(before_height),
        );
    }

    fn replay_resume_items(&mut self, items: &[Item]) {
        for item in items {
            self.push_item(item);
        }
    }

    fn replay_older_resume_items(&mut self, items: &[Item]) {
        for item in items.iter().rev() {
            self.prepend_item(item);
        }
    }

    fn timeline_rendered_height(&mut self) -> usize {
        let width = self.last_frame_width.max(1);
        let height = self.timeline.last_viewport_height().max(1);
        let _ = self
            .timeline
            .render_with_frame(self.theme, Rect::new(0, 0, width, height), 0);
        self.timeline.visual_height()
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
            Item::RoutingDecision { decision, .. } => {
                self.timeline
                    .push_system(routing_decision_summary(decision));
            }
            Item::Compaction { .. } => {}
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

    fn prepend_item(&mut self, item: &Item) {
        match item {
            Item::UserMessage { text, .. } => {
                self.timeline.prepend_user(text.clone());
            }
            Item::AgentMessage { text, phase, .. } => {
                self.timeline.prepend_assistant(text.clone(), phase.clone());
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
                    self.timeline.prepend_reasoning(text);
                }
            }
            Item::ToolExecution {
                id,
                tool_name,
                status,
                input,
                output,
                error,
                ..
            } => {
                if !is_stdin_tool(tool_name) {
                    let arguments = input.as_ref().map(ToString::to_string).unwrap_or_default();
                    self.timeline.prepend_saved_tool(
                        format!("history:{id}"),
                        ToolTimelineEntry::new(tool_name.clone(), arguments),
                        *status,
                        error.clone().or_else(|| output.clone()),
                    );
                }
            }
            Item::RoutingDecision { decision, .. } => {
                self.timeline
                    .prepend_system(routing_decision_summary(decision));
            }
            Item::Compaction { .. } => {}
            Item::Error { message, .. } => {
                self.timeline.prepend_error(message.clone());
            }
            Item::Raw { payload, .. } => {
                if let Some(text) = payload.as_str().filter(|text| !text.trim().is_empty()) {
                    self.timeline.prepend_system(text.to_string());
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

fn take_unseen_items(turns: &[Turn], seen: &mut HashSet<(String, String)>) -> Vec<Item> {
    turns
        .iter()
        .flat_map(|turn| {
            turn.items
                .iter()
                .map(move |item| ((turn.id.clone(), resume_item_id(item).to_string()), item))
        })
        .filter_map(|(key, item)| seen.insert(key).then(|| item.clone()))
        .collect()
}

fn resume_item_id(item: &Item) -> &str {
    match item {
        Item::UserMessage { id, .. }
        | Item::AgentMessage { id, .. }
        | Item::Reasoning { id, .. }
        | Item::ToolExecution { id, .. }
        | Item::RoutingDecision { id, .. }
        | Item::Compaction { id, .. }
        | Item::Error { id, .. }
        | Item::Raw { id, .. } => id,
    }
}

fn split_resume_items(items: Vec<Item>) -> (Vec<Item>, Vec<Item>) {
    if items.len() <= RESUME_VISIBLE_TAIL_ITEMS {
        return (Vec::new(), items);
    }
    let latest_compaction_tail_start = items
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, item)| matches!(item, Item::Compaction { .. }).then_some(index + 1))
        .filter(|start| items.len().saturating_sub(*start) <= RESUME_VISIBLE_TAIL_ITEMS);
    let start = latest_compaction_tail_start
        .unwrap_or_else(|| items.len().saturating_sub(RESUME_VISIBLE_TAIL_ITEMS));
    let mut older = items;
    let visible = older.split_off(start);
    (older, visible)
}

fn reasoning_blocks_text(blocks: &[String]) -> String {
    blocks
        .iter()
        .filter(|block| !block.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn routing_decision_summary(decision: &roder_protocol::InferenceRoutingDecisionEvent) -> String {
    let selected = format!(
        "{} / {}",
        decision.selected_selection.provider, decision.selected_selection.model
    );
    let verb = match format!("{:?}", decision.decision.outcome).as_str() {
        "Abstained" => "kept",
        "Fallback" => "fell back to",
        "Escalated" => "escalated to",
        _ => "selected",
    };
    let thinking = decision
        .decision
        .reasoning
        .as_ref()
        .map(|reasoning| {
            if !reasoning.enabled {
                "thinking off".to_string()
            } else {
                format!("thinking {}", reasoning.level.as_deref().unwrap_or("on"))
            }
        })
        .map(|label| format!(" ({label})"))
        .unwrap_or_default();
    format!("Auto {verb} {selected}{thinking}")
}

pub(super) struct ResumedThread {
    pub thread: Option<Thread>,
    pub lifecycle: TurnLifecycleSnapshot,
    pub before: Option<String>,
}

pub(super) async fn read_thread<C: AppClient>(
    client: &C,
    thread_id: &str,
) -> anyhow::Result<ResumedThread> {
    let response = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("thread/resume")),
            method: "thread/resume".to_string(),
            params: Some(
                serde_json::to_value(ThreadResumeParams {
                    thread_id: thread_id.to_string(),
                    limit: Some(RESUME_VISIBLE_TAIL_ITEMS),
                    max_bytes: None,
                })
                .unwrap(),
            ),
        })
        .await;
    if response
        .error
        .as_ref()
        .is_some_and(|error| error.code == -32012 && error.message == "forbidden: missing_scope")
    {
        // A read-only session can still browse bounded history without opening
        // a runtime session or running resume hooks.
        let (page, read) = tokio::join!(
            request_history_page(client, thread_id, None),
            read_thread_snapshot(client, thread_id, false),
        );
        let mut page = decode_response::<ThreadHistoryReadResult>(page)?;
        let read = read?;
        page.thread = page.thread.or(read.thread);
        return resumed_thread(thread_id, page, read.lifecycle);
    }
    if response.error.as_ref().is_some_and(|error| {
        error.code == -32601
            || matches!(
                error.message.as_str(),
                "thread store does not support bounded history pagination"
                    | "thread history requires a thread store"
                    | "thread/resume is unsupported by this backend"
            )
    }) {
        let read = read_thread_snapshot(client, thread_id, true).await?;
        return Ok(ResumedThread {
            thread: read.thread,
            lifecycle: read.lifecycle,
            before: None,
        });
    }
    let ThreadResumeResult {
        history: mut page,
        lifecycle,
    } = decode_response(response)?;
    if page.thread.is_none() {
        // The thread may exist only in the current app-server instance.
        let read = read_thread_snapshot(client, thread_id, false).await?;
        page.thread = read.thread;
        return resumed_thread(thread_id, page, read.lifecycle);
    }
    resumed_thread(thread_id, page, lifecycle)
}

fn resumed_thread(
    thread_id: &str,
    page: ThreadHistoryReadResult,
    lifecycle: TurnLifecycleSnapshot,
) -> anyhow::Result<ResumedThread> {
    anyhow::ensure!(
        page.thread
            .as_ref()
            .is_none_or(|thread| thread.id == thread_id),
        "history response references another thread"
    );
    Ok(ResumedThread {
        thread: page.thread,
        lifecycle,
        before: page_cursor(page.has_more, page.before)?,
    })
}

async fn request_history_page<C: AppClient>(
    client: &C,
    thread_id: &str,
    before: Option<String>,
) -> roder_protocol::JsonRpcResponse {
    let limit = if before.is_some() {
        RESUME_OLDER_BATCH_ITEMS
    } else {
        RESUME_VISIBLE_TAIL_ITEMS
    };
    client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("thread/history/read")),
            method: "thread/history/read".to_string(),
            params: Some(
                serde_json::to_value(ThreadHistoryReadParams {
                    thread_id: thread_id.to_string(),
                    before,
                    limit: Some(limit),
                    max_bytes: None,
                })
                .unwrap(),
            ),
        })
        .await
}

fn page_cursor(has_more: bool, before: Option<String>) -> anyhow::Result<Option<String>> {
    if !has_more {
        return Ok(None);
    }
    let before = before
        .filter(|cursor| !cursor.is_empty())
        .ok_or_else(|| anyhow::anyhow!("history page is missing its next cursor"))?;
    Ok(Some(before))
}

async fn read_thread_snapshot<C: AppClient>(
    client: &C,
    thread_id: &str,
    include_turns: bool,
) -> anyhow::Result<ThreadReadResult> {
    let res = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("thread/read")),
            method: "thread/read".to_string(),
            params: Some(
                serde_json::to_value(ThreadReadParams {
                    thread_id: thread_id.to_string(),
                    include_turns,
                })
                .unwrap(),
            ),
        })
        .await;
    decode_response(res)
}

fn lifecycle_recovery_warning(
    thread_id: &str,
    lifecycle: &TurnLifecycleSnapshot,
) -> Option<String> {
    let recovery_needed = lifecycle
        .records
        .iter()
        .filter(|record| record.state == TurnLifecycleState::RecoveryNeeded)
        .count();
    let corrupt_records = lifecycle.corrupt_record_count;

    let mut warnings = Vec::new();
    if recovery_needed > 0 {
        let turns = if recovery_needed == 1 {
            "turn was"
        } else {
            "turns were"
        };
        warnings.push(format!(
            "Recovery warning: {recovery_needed} {turns} reconciled after an interrupted runtime. Verify results before continuing."
        ));
    }
    if corrupt_records > 0 {
        let records = if corrupt_records == 1 {
            "record was"
        } else {
            "records were"
        };
        warnings.push(format!(
            "Lifecycle warning: {corrupt_records} corrupt lifecycle {records} skipped. Inspect with `roder thread lifecycle {thread_id} --json`."
        ));
    }

    (!warnings.is_empty()).then(|| warnings.join("\n"))
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
    if let Some(message_count) = thread.message_count {
        return message_count as usize;
    }
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
    use roder_api::lifecycle::{TurnCleanupState, TurnLifecycleRecord};
    use roder_protocol::{ThreadStatus, Turn};
    use time::OffsetDateTime;

    use super::*;

    fn test_thread(items: Vec<Item>) -> Thread {
        Thread {
            id: "thread-a".to_string(),
            preview: String::new(),
            tool_allowlist: Vec::new(),
            service_tier: None,
            developer_instructions: None,
            external_tools: Vec::new(),
            runner: None,
            parent_thread_id: None,
            workspace_fork: None,
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
            message_count: None,
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
                finish_reason: None,
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

    #[test]
    fn recovery_warning_reports_reconciled_turns_and_corrupt_records() {
        let lifecycle = TurnLifecycleSnapshot {
            records: vec![TurnLifecycleRecord::new(
                "thread-a".to_string(),
                "turn-a".to_string(),
                TurnLifecycleState::RecoveryNeeded,
                TurnCleanupState::Unknown,
                None,
                OffsetDateTime::UNIX_EPOCH,
            )],
            corrupt_record_count: 2,
        };

        let warning = lifecycle_recovery_warning("thread-a", &lifecycle)
            .expect("recovery state should be visible");

        assert!(warning.contains("1 turn was reconciled"));
        assert!(warning.contains("2 corrupt lifecycle records were skipped"));
        assert!(warning.contains("roder thread lifecycle thread-a --json"));
    }

    #[test]
    fn recovery_warning_is_absent_without_recovery_diagnostics() {
        assert!(
            lifecycle_recovery_warning("thread-a", &TurnLifecycleSnapshot::default()).is_none()
        );
    }
}
