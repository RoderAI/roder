use super::events::emit_event;
use super::*;

pub(super) fn prompt_text(prompt: Option<String>, input: Vec<TurnInputItem>) -> String {
    prompt.unwrap_or_else(|| {
        input
            .into_iter()
            .filter_map(|item| item.text)
            .collect::<Vec<_>>()
            .join("\n")
    })
}

pub(super) async fn run_session(
    mut session: Box<dyn AgentBackendSession>,
    mut commands: mpsc::Receiver<Command>,
    thread: Arc<RwLock<Thread>>,
    runtime: Arc<Runtime>,
    routes: Arc<RwLock<HashMap<String, String>>>,
) {
    let thread_id = session.thread_id().to_owned();
    let mut turn_id = String::new();
    let mut output = HashMap::<String, String>::new();
    let mut usage = None;
    loop {
        tokio::select! {
            Some(command) = commands.recv() => match command {
                Command::Send(text, mode, model, reasoning, reply) => {
                    let result = session.send(text.clone(), mode, model, reasoning).await;
                    if let Ok(id) = &result {
                        turn_id = id.clone();
                        let mut thread = thread.write().await;
                        thread.message_count = Some(thread.message_count.unwrap_or(0) + 1);
                        if thread.preview.is_empty() { thread.preview = text.chars().take(80).collect(); }
                        thread.turns.get_or_insert_with(Vec::new).push(Turn {
                            id: id.clone(), items: vec![Item::UserMessage { id: format!("user-{id}"), text, images: vec![], status: None }],
                            items_view: "full".into(), status: "inProgress".into(), error: None,
                            started_at: Some(OffsetDateTime::now_utc().unix_timestamp()), completed_at: None,
                            duration_ms: None, usage: None, finish_reason: None,
                        });
                    }
                    let _ = reply.send(result);
                }
                Command::Interrupt(reply) => { let _ = reply.send(session.interrupt().await); }
                Command::Archive(reply) => {
                    let result = session.archive().await;
                    let success = result.is_ok();
                    let _ = reply.send(result);
                    if success { break; }
                }
                Command::Approve(id, approved, reply) => {
                    let result = session.approve(&id, approved).await;
                    let _ = runtime.emit(RoderEvent::ApprovalResolved(ApprovalResolved {
                        thread_id: thread_id.clone(), turn_id: turn_id.clone(), approval_id: id.clone(),
                        tool_id: id, tool_name: "Codex tool".into(), approved,
                        timestamp: OffsetDateTime::now_utc(),
                    })).await;
                    let _ = reply.send(result);
                }
                Command::Answer(id, answers, reply) => {
                    let result = session.answer(&id, answers.clone()).await;
                    let _ = runtime.emit(RoderEvent::UserInputResolved(UserInputResolved {
                        thread_id: thread_id.clone(), turn_id: turn_id.clone(), request_id: id,
                        answers: json!(answers), timestamp: OffsetDateTime::now_utc(),
                    })).await;
                    let _ = reply.send(result);
                }
            },
            event = session.next_event() => match event {
                Ok(event) => emit_event(event, &thread_id, &mut turn_id, &mut output, &mut usage, &thread, &runtime, &routes).await,
                Err(err) => {
                    if !turn_id.is_empty() {
                        let _ = runtime.emit(RoderEvent::TurnFailed(TurnFailed {
                            thread_id, turn_id, error: err.to_string(), error_kind: Some("backend_disconnected".into()),
                            usage: None, timestamp: OffsetDateTime::now_utc(),
                        })).await;
                    }
                    break;
                }
            },
            else => break,
        }
    }
}
