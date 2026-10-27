use std::collections::VecDeque;

use super::*;

#[derive(Default)]
pub(super) struct PromptAdmission {
    recoveries: VecDeque<(String, PendingPrompt)>,
    rendered: HashSet<(String, u64)>,
    submission_order: HashMap<u64, usize>,
    completed: VecDeque<(String, String)>,
    accepted: HashMap<String, AcceptedStart>,
    auto_recovered: HashSet<u64>,
    probe: Option<ReceiptProbe>,
}

struct AcceptedStart {
    turn_id: String,
    next_probe: Instant,
}

struct ReceiptProbe {
    thread_id: String,
    turn_id: String,
    handle: tokio::task::JoinHandle<JsonRpcResponse>,
    sent_at: Instant,
}

pub(super) struct PendingTurnRequest {
    pub thread_id: String,
    pub method: &'static str,
    pub expected_turn_id: Option<String>,
    pub prompt: PendingPrompt,
    pub sent_at: Instant,
    pub handle: tokio::task::JoinHandle<JsonRpcResponse>,
}

impl PromptAdmission {
    pub(super) fn reset_rendered_thread(&mut self, thread_id: &str) {
        self.rendered.retain(|(thread, _)| thread != thread_id);
    }

    fn retain_recovery(&mut self, thread_id: String, prompt: PendingPrompt) {
        let order = self
            .submission_order
            .get(&prompt.client_id)
            .copied()
            .unwrap_or(usize::MAX);
        let index = self
            .recoveries
            .iter()
            .position(|(thread, other)| {
                thread == &thread_id
                    && self
                        .submission_order
                        .get(&other.client_id)
                        .copied()
                        .unwrap_or(usize::MAX)
                        > order
            })
            .unwrap_or(self.recoveries.len());
        self.recoveries.insert(index, (thread_id, prompt));
    }

    pub(super) fn take_recovery(&mut self, thread_id: &str) -> Option<PendingPrompt> {
        let index = self.recoveries.iter().position(|(id, _)| id == thread_id)?;
        self.recoveries.remove(index).map(|(_, prompt)| prompt)
    }

    fn has_recovery(&self, thread_id: &str) -> bool {
        self.recoveries.iter().any(|(id, _)| id == thread_id)
    }

    fn completed(&self, thread_id: &str, turn_id: &str) -> bool {
        self.completed
            .iter()
            .any(|(thread, turn)| thread == thread_id && turn == turn_id)
    }

    pub(super) fn note_completed(&mut self, thread_id: &str, turn_id: &str) {
        if !self.completed(thread_id, turn_id) {
            self.completed
                .push_back((thread_id.to_owned(), turn_id.to_owned()));
            if self.completed.len() > 256 {
                self.completed.pop_front();
            }
        }
        if self
            .accepted
            .get(thread_id)
            .is_some_and(|receipt| receipt.turn_id == turn_id)
        {
            self.accepted.remove(thread_id);
        }
    }
}

impl<C: AppClient> TuiApp<C> {
    pub(super) fn visible_queued_prompts(&self) -> impl Iterator<Item = (usize, &PendingPrompt)> {
        let thread = self.focused_thread_id();
        self.queued_prompts
            .iter()
            .enumerate()
            .filter(move |(_, prompt)| prompt.thread_id.as_deref().is_none_or(|id| id == thread))
    }

    pub(super) fn queued_prompt_count(&self) -> usize {
        self.visible_queued_prompts().count()
    }

    fn prompt_dispatch_held(&self) -> bool {
        let thread_id = self.focused_thread_id();
        self.active_turn_id.is_some()
            || self
                .pending_turn_requests
                .iter()
                .any(|request| request.thread_id == thread_id)
            || self.prompt_admission.accepted.contains_key(thread_id)
            || self.show_palette
            || self.show_provider_popup
            || self.confirm_dialog.is_some()
            || self.user_input_dialog.is_some()
            || self.pending_plan_exit.is_some()
            || self.source_copy_active()
    }

    pub(super) fn can_dispatch_prompt(&self) -> bool {
        !self.prompt_dispatch_held()
            && !self.prompt_admission.has_recovery(self.focused_thread_id())
    }

    pub(super) fn observe_prompt_admission(&mut self, event: &RoderEvent) {
        match event {
            RoderEvent::TurnStarted(event) => {
                if self
                    .prompt_admission
                    .accepted
                    .get(&event.thread_id)
                    .is_some_and(|receipt| receipt.turn_id == event.turn_id)
                {
                    self.prompt_admission.accepted.remove(&event.thread_id);
                }
            }
            RoderEvent::TurnCompleted(event) => {
                self.prompt_admission
                    .note_completed(&event.thread_id, &event.turn_id);
            }
            RoderEvent::TurnInterrupted(event) => {
                self.prompt_admission
                    .note_completed(&event.thread_id, &event.turn_id);
            }
            RoderEvent::TurnFailed(event) => {
                self.prompt_admission
                    .note_completed(&event.thread_id, &event.turn_id);
            }
            RoderEvent::TurnLifecycleUpdated(event) if event.state.is_terminal() => {
                self.prompt_admission
                    .note_completed(&event.thread_id, &event.turn_id);
            }
            _ => {}
        }
    }

    fn render_submitted_prompt(&mut self, prompt: &PendingPrompt, steering: bool) {
        self.last_user_prompt = Some(prompt.clone());
        if self
            .prompt_admission
            .rendered
            .insert((self.focused_thread_id().to_owned(), prompt.client_id))
        {
            let sequence = self.prompt_admission.submission_order.len();
            self.prompt_admission
                .submission_order
                .insert(prompt.client_id, sequence);
            let display = if steering {
                format!("steer: {}", prompt.display)
            } else {
                prompt.display.clone()
            };
            self.timeline.push_user(display);
            self.timeline
                .attach_images_to_last_item(prompt.preview_paths.iter().cloned());
            self.thread_message_count = self.thread_message_count.saturating_add(1);
            if self.thread_title.is_none() {
                self.thread_title = Some(truncate(&prompt.display, 72));
            }
        }
    }

    pub(super) async fn start_prepared_prompt(&mut self, mut pending: PendingPrompt) {
        let thread_id = self.focused_thread_id().to_owned();
        pending.thread_id.get_or_insert(thread_id.clone());
        if !self.can_dispatch_prompt()
            || pending.thread_id.as_deref() != Some(&thread_id)
            || self
                .queued_prompts
                .iter()
                .any(|prompt| prompt.thread_id.as_deref().is_none_or(|id| id == thread_id))
        {
            self.queue_prepared_prompt(pending);
            return;
        }
        self.dispatch_prepared_start(pending).await;
    }

    pub(super) async fn retry_prepared_prompt(&mut self, pending: PendingPrompt) {
        if self.prompt_dispatch_held() {
            self.prompt_admission
                .recoveries
                .push_front((self.focused_thread_id().to_owned(), pending));
            self.record_error("retry is held until pending requests and settings settle".into());
            return;
        }
        self.dispatch_prepared_start(pending).await;
    }

    async fn dispatch_prepared_start(&mut self, mut pending: PendingPrompt) {
        let thread_id = self.focused_thread_id().to_owned();
        pending.thread_id.get_or_insert(thread_id.clone());
        self.render_submitted_prompt(&pending, false);
        let params = TurnStartParams {
            thread_id: thread_id.clone(),
            input: pending_turn_input(pending.message.clone(), pending.images.clone()),
            prompt: None,
            model_provider: None,
            model: None,
            reasoning: None,
            service_tier: None,
            developer_context: None,
            mcp_auth_token: None,
            policy_mode: Some(self.policy_mode),
            task_ledger_required: false,
        };
        let client = self.client.clone();
        let client_id = pending.client_id;
        let handle = tokio::spawn(async move {
            client
                .send_request(JsonRpcRequest {
                    jsonrpc: "2.0".into(),
                    id: Some(serde_json::json!(format!("prompt-{client_id}"))),
                    method: "turn/start".into(),
                    params: Some(serde_json::to_value(params).unwrap()),
                })
                .await
        });
        self.pending_turn_requests.push(PendingTurnRequest {
            thread_id,
            method: "turn/start",
            expected_turn_id: None,
            prompt: pending,
            sent_at: Instant::now(),
            handle,
        });
    }

    pub(super) fn steer_prepared_prompt(&mut self, mut pending: PendingPrompt) {
        let thread_id = self.focused_thread_id().to_owned();
        pending.thread_id.get_or_insert(thread_id.clone());
        let Some(turn_id) = self.active_turn_id.clone() else {
            self.queue_prepared_prompt(pending);
            return;
        };
        if pending.thread_id.as_deref() != Some(&thread_id)
            || self.show_palette
            || self.show_provider_popup
            || self.confirm_dialog.is_some()
            || self.user_input_dialog.is_some()
            || self.source_copy_active()
        {
            self.queue_prepared_prompt(pending);
            return;
        }
        self.render_submitted_prompt(&pending, true);
        let params = TurnSteerParams {
            thread_id: thread_id.clone(),
            expected_turn_id: turn_id.clone(),
            input: pending_turn_input(pending.message.clone(), pending.images.clone()),
            prompt: None,
        };
        let client = self.client.clone();
        let client_id = pending.client_id;
        let handle = tokio::spawn(async move {
            client
                .send_request(JsonRpcRequest {
                    jsonrpc: "2.0".into(),
                    id: Some(serde_json::json!(format!("prompt-{client_id}"))),
                    method: "turn/steer".into(),
                    params: Some(serde_json::to_value(params).unwrap()),
                })
                .await
        });
        self.pending_turn_requests.push(PendingTurnRequest {
            thread_id,
            method: "turn/steer",
            expected_turn_id: Some(turn_id),
            prompt: pending,
            sent_at: Instant::now(),
            handle,
        });
    }

    pub(super) async fn finish_turn_requests_if_ready(&mut self) -> bool {
        let mut changed = false;
        let mut index = 0;
        while index < self.pending_turn_requests.len() {
            if !self.pending_turn_requests[index].handle.is_finished()
                && self.pending_turn_requests[index].sent_at.elapsed() < Duration::from_secs(30)
            {
                index += 1;
                continue;
            }
            let request = self.pending_turn_requests.remove(index);
            if !request.handle.is_finished() {
                request.handle.abort();
            }
            let response = request.handle.await;
            // Only the explicit server rejection proves an absent turn. A lost
            // response, malformed receipt, or task cancellation never authorizes resend.
            let no_active = request.method == "turn/steer"
                && request.expected_turn_id.is_some()
                && response
                    .as_ref()
                    .ok()
                    .and_then(|response| response.error.as_ref())
                    .is_some_and(|error| error.message.contains("no active turn to steer"));
            let result = match response {
                Ok(response) => decode_response::<roder_protocol::TurnStartResult>(response),
                Err(error) => Err(anyhow::anyhow!("request task failed: {error}")),
            };
            match result {
                Ok(receipt) => {
                    if request.expected_turn_id.is_none()
                        && !self
                            .prompt_admission
                            .completed(&request.thread_id, &receipt.turn_id)
                        && !(request.thread_id == self.focused_thread_id()
                            && self.active_turn_id.as_deref() == Some(&receipt.turn_id))
                    {
                        self.prompt_admission.accepted.insert(
                            request.thread_id,
                            AcceptedStart {
                                turn_id: receipt.turn_id,
                                next_probe: Instant::now() + Duration::from_secs(2),
                            },
                        );
                    }
                }
                Err(error) => {
                    let recover_automatically = no_active
                        && self
                            .prompt_admission
                            .auto_recovered
                            .insert(request.prompt.client_id);
                    if request.thread_id == self.focused_thread_id()
                        && no_active
                        && request.expected_turn_id.is_some()
                        && self.active_turn_id == request.expected_turn_id
                    {
                        self.active_turn_id = None;
                        self.active_turn_timer.reset();
                        self.working_status_override = None;
                        self.clear_compaction_progress();
                        self.progress.set(TerminalProgress::Error);
                    }
                    if recover_automatically {
                        let order =
                            self.prompt_admission.submission_order[&request.prompt.client_id];
                        let index = self
                            .queued_prompts
                            .iter()
                            .position(|prompt| {
                                prompt.thread_id == request.prompt.thread_id
                                    && self
                                        .prompt_admission
                                        .submission_order
                                        .get(&prompt.client_id)
                                        .copied()
                                        .unwrap_or(usize::MAX)
                                        > order
                            })
                            .unwrap_or(self.queued_prompts.len());
                        self.queued_prompts.insert(index, request.prompt);
                    } else {
                        self.prompt_admission
                            .retain_recovery(request.thread_id.clone(), request.prompt);
                    }
                    let recovery = if recover_automatically {
                        "Follow-up retained for the next turn."
                    } else {
                        "Input retained; inspect history before /retry if acceptance is uncertain, or /retry discard to drop it."
                    };
                    let message = format!("{} failed: {error}. {recovery}", request.method);
                    if request.thread_id == self.focused_thread_id() {
                        self.record_error(message);
                    } else if let Some(timeline) =
                        self.team_timeline_for_thread_mut(&request.thread_id)
                    {
                        timeline.push_error(message);
                    } else {
                        self.record_error(format!("thread {}: {message}", request.thread_id));
                    }
                }
            }
            changed = true;
        }
        changed
    }

    pub(super) async fn submit_next_queued_prompt(&mut self) -> bool {
        if !self.can_dispatch_prompt() {
            return false;
        }
        let thread_id = self.focused_thread_id().to_owned();
        let Some(prompt) = self.queued_prompts.pop_front_for(&thread_id) else {
            return false;
        };
        self.dispatch_prepared_start(prompt).await;
        true
    }

    pub(super) async fn poll_prompt_receipt(&mut self) -> bool {
        let mut changed = false;
        if self.prompt_admission.probe.as_ref().is_some_and(|probe| {
            probe.handle.is_finished() || probe.sent_at.elapsed() >= Duration::from_secs(5)
        }) {
            let probe = self.prompt_admission.probe.take().unwrap();
            if !probe.handle.is_finished() {
                probe.handle.abort();
            }
            if let Ok(response) = probe.handle.await
                && let Ok(read) = decode_response::<roder_protocol::ThreadReadResult>(response)
            {
                if read.lifecycle.records.iter().any(|record| {
                    record.thread_id == probe.thread_id
                        && record.turn_id == probe.turn_id
                        && record.state.is_terminal()
                }) {
                    self.prompt_admission
                        .note_completed(&probe.thread_id, &probe.turn_id);
                    changed = true;
                } else if let Some(thread) = read.thread
                    && thread.status.active_turn_id.as_deref() == Some(&probe.turn_id)
                {
                    self.prompt_admission.accepted.remove(&probe.thread_id);
                    if self.focused_thread_id() == probe.thread_id && self.active_turn_id.is_none()
                    {
                        self.active_turn_id = Some(probe.turn_id);
                        self.active_turn_timer.start(Instant::now());
                        self.progress.set(TerminalProgress::Working);
                    }
                    changed = true;
                }
            }
        }
        if self.prompt_admission.probe.is_none()
            && let Some((thread_id, receipt)) = self
                .prompt_admission
                .accepted
                .iter_mut()
                .find(|(_, receipt)| receipt.next_probe <= Instant::now())
        {
            receipt.next_probe = Instant::now() + Duration::from_secs(2);
            let thread_id = thread_id.clone();
            let turn_id = receipt.turn_id.clone();
            let client = self.client.clone();
            let params = serde_json::json!({"threadId": thread_id, "includeTurns": false});
            let handle = tokio::spawn(async move {
                client
                    .send_request(JsonRpcRequest {
                        jsonrpc: "2.0".into(),
                        id: Some(serde_json::json!("prompt-receipt")),
                        method: "thread/read".into(),
                        params: Some(params),
                    })
                    .await
            });
            self.prompt_admission.probe = Some(ReceiptProbe {
                thread_id,
                turn_id,
                handle,
                sent_at: Instant::now(),
            });
        }
        changed
    }
}

#[cfg(test)]
#[path = "prompt_admission_tests.rs"]
mod tests;
