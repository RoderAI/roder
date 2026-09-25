//! Maps Roder's app-server methods and events to a complete agent backend.
//! Clients keep using the Roder protocol regardless of who runs the agent.

use std::collections::HashMap;
use std::sync::Arc;

use roder_api::backend::{AgentBackend, AgentBackendSession, BackendHistoryItem, BackendStart};
use roder_api::events::{ApprovalResolved, RoderEvent, TurnFailed, UserInputResolved};
use roder_api::policy_mode::PolicyMode;
use roder_core::Runtime;
use roder_protocol::{
    Item, JsonRpcError, JsonRpcRequest, Thread, ThreadItemStatus, ThreadReadParams,
    ThreadReadResult, ThreadStartParams, ThreadStartResult, ThreadStatus, Turn, TurnInputItem,
    TurnInterruptParams, TurnStartParams, TurnStartResult, TurnSteerParams,
};
use serde_json::{Value, json};
use time::OffsetDateTime;
use tokio::sync::{RwLock, mpsc, oneshot};

#[derive(Clone)]
struct ThreadHandle {
    thread: Arc<RwLock<Thread>>,
    commands: mpsc::Sender<Command>,
}

enum Command {
    Send(
        String,
        Option<PolicyMode>,
        Option<String>,
        Option<String>,
        oneshot::Sender<anyhow::Result<String>>,
    ),
    Interrupt(oneshot::Sender<anyhow::Result<()>>),
    Archive(oneshot::Sender<anyhow::Result<()>>),
    Approve(String, bool, oneshot::Sender<anyhow::Result<()>>),
    Answer(
        String,
        Vec<(String, String)>,
        oneshot::Sender<anyhow::Result<()>>,
    ),
}

pub(crate) struct AgentBackendBridge {
    backend: Arc<dyn AgentBackend>,
    runtime: Arc<Runtime>,
    threads: RwLock<HashMap<String, ThreadHandle>>,
    request_routes: Arc<RwLock<HashMap<String, String>>>,
    selected_model: RwLock<Option<(String, String)>>,
}

impl AgentBackendBridge {
    pub fn new(backend: Arc<dyn AgentBackend>, runtime: Arc<Runtime>) -> Self {
        Self {
            backend,
            runtime,
            threads: RwLock::new(HashMap::new()),
            request_routes: Arc::new(RwLock::new(HashMap::new())),
            selected_model: RwLock::new(None),
        }
    }

    pub async fn handle(&self, req: &JsonRpcRequest) -> Option<Result<Value, JsonRpcError>> {
        let params = req.params.clone().unwrap_or(Value::Null);
        let result = match req.method.as_str() {
            "initialize" => self.initialize().await,
            "model/list" => self.models().await,
            "providers/list" => self.providers().await,
            "providers/select" | "model/select" => self.select_model(&req.method, params).await,
            "thread/start" => self.start(params).await,
            "thread/read" => self.read(params).await,
            "thread/archive" => self.archive(params).await,
            "thread/list" => self.list(params).await,
            "turn/start" => self.turn_start(params).await,
            "turn/steer" => self.turn_steer(params).await,
            "turn/interrupt" => self.turn_interrupt(params).await,
            "thread/resolve_approval" => self.approve(params).await,
            "thread/resolve_user_input" => self.answer(params).await,
            _ => return None,
        };
        Some(result.map_err(|err| JsonRpcError {
            code: -32000,
            message: err.to_string(),
            data: None,
        }))
    }

    async fn start(&self, params: Value) -> anyhow::Result<Value> {
        let params: ThreadStartParams = serde_json::from_value(params)?;
        let cwd = params
            .cwd
            .as_deref()
            .map(std::path::PathBuf::from)
            .unwrap_or(std::env::current_dir()?);
        let model = params.model.clone().or_else(|| {
            self.selected_model
                .try_read()
                .ok()
                .and_then(|selection| selection.as_ref().map(|(model, _)| model.clone()))
        });
        let workspace_id = params.workspace_id.clone();
        let root_id = params.root_id.clone().unwrap_or_default();
        let (thread, reasoning) = self
            .connect(
                BackendStart {
                    cwd: cwd.clone(),
                    model: model.clone(),
                    resume_thread: None,
                    policy_mode: self.runtime.status().await.policy_mode,
                },
                workspace_id.clone(),
                root_id.clone(),
            )
            .await?;
        Ok(serde_json::to_value(ThreadStartResult {
            model: thread.model.clone(),
            model_provider: self.backend.id().to_owned(),
            reasoning,
            selection_mode: None,
            cwd: cwd.display().to_string(),
            workspace_id,
            root_id,
            thread,
        })?)
    }

    async fn connect(
        &self,
        start: BackendStart,
        workspace_id: String,
        root_id: String,
    ) -> anyhow::Result<(Thread, String)> {
        let session = self.backend.connect(start.clone()).await?;
        let id = session.thread_id().to_owned();
        let model = session.model().to_owned();
        let reasoning = session.reasoning().to_owned();
        let snapshot = session.snapshot();
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let history = snapshot
            .turns
            .into_iter()
            .map(|turn| Turn {
                id: turn.id,
                items: turn
                    .items
                    .into_iter()
                    .map(|item| match item {
                        BackendHistoryItem::User { id, text } => Item::UserMessage {
                            id,
                            text,
                            images: vec![],
                            status: None,
                        },
                        BackendHistoryItem::Agent { id, text } => Item::AgentMessage {
                            id,
                            text,
                            phase: None,
                            status: Some(ThreadItemStatus::Completed),
                        },
                        BackendHistoryItem::Tool {
                            id,
                            name,
                            output,
                            status,
                        } => Item::ToolExecution {
                            tool_call_id: id.clone(),
                            id,
                            tool_name: name,
                            output,
                            status: if status == "failed" {
                                ThreadItemStatus::Failed
                            } else {
                                ThreadItemStatus::Completed
                            },
                            input: None,
                            error: None,
                        },
                        BackendHistoryItem::Other { id, payload } => Item::Raw {
                            id,
                            payload,
                            status: Some(ThreadItemStatus::Completed),
                        },
                    })
                    .collect(),
                items_view: "full".into(),
                status: turn.status,
                error: None,
                started_at: turn.started_at,
                completed_at: turn.completed_at,
                duration_ms: None,
                usage: None,
                finish_reason: None,
            })
            .collect::<Vec<_>>();
        let thread = Thread {
            id: id.clone(),
            preview: snapshot.preview,
            model_provider: self.backend.id().into(),
            model,
            selection_mode: None,
            created_at: if snapshot.created_at == 0 {
                now
            } else {
                snapshot.created_at
            },
            updated_at: if snapshot.updated_at == 0 {
                now
            } else {
                snapshot.updated_at
            },
            status: ThreadStatus {
                kind: "idle".into(),
                active_turn_id: None,
                active_flags: vec![],
            },
            cwd: if snapshot.cwd.is_empty() {
                start.cwd.display().to_string()
            } else {
                snapshot.cwd
            },
            workspace_id: Some(workspace_id),
            root_id: Some(root_id),
            name: None,
            message_count: Some(history.len().try_into().unwrap_or(u32::MAX)),
            turns: Some(history),
            usage: None,
            tool_allowlist: vec![],
            developer_instructions: None,
            external_tools: vec![],
            runner: None,
            parent_thread_id: None,
            workspace_fork: None,
        };
        let state = Arc::new(RwLock::new(thread.clone()));
        let (tx, rx) = mpsc::channel(32);
        self.threads.write().await.insert(
            id.clone(),
            ThreadHandle {
                thread: state.clone(),
                commands: tx,
            },
        );
        tokio::spawn(run_session(
            session,
            rx,
            state,
            self.runtime.clone(),
            self.request_routes.clone(),
        ));
        Ok((thread, reasoning))
    }

    async fn read(&self, params: Value) -> anyhow::Result<Value> {
        let params: ThreadReadParams = serde_json::from_value(params)?;
        let handle = if let Some(handle) = self.threads.read().await.get(&params.thread_id).cloned()
        {
            handle
        } else {
            let start = BackendStart {
                cwd: std::env::current_dir()?,
                model: None,
                resume_thread: Some(params.thread_id.clone()),
                policy_mode: self.runtime.status().await.policy_mode,
            };
            self.connect(start, String::new(), String::new()).await?;
            self.threads
                .read()
                .await
                .get(&params.thread_id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("backend did not resume {}", params.thread_id))?
        };
        Ok(serde_json::to_value(ThreadReadResult {
            thread: Some(handle.thread.read().await.clone()),
            lifecycle: Default::default(),
        })?)
    }

    async fn archive(&self, params: Value) -> anyhow::Result<Value> {
        let params: roder_protocol::ThreadArchiveParams = serde_json::from_value(params)?;
        if !self.threads.read().await.contains_key(&params.thread_id) {
            self.read(json!({"threadId":params.thread_id.clone()}))
                .await?;
        }
        self.command(&params.thread_id, Command::Archive).await?;
        self.threads.write().await.remove(&params.thread_id);
        Ok(serde_json::to_value(roder_protocol::ThreadArchiveResult {
            thread_id: params.thread_id,
            archived: true,
        })?)
    }

    async fn list(&self, params: Value) -> anyhow::Result<Value> {
        let params: roder_protocol::ThreadListParams = serde_json::from_value(params)?;
        let stored = self
            .backend
            .list_threads(std::env::current_dir()?, params.limit)
            .await?;
        let handles = self
            .threads
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let mut data = stored
            .into_iter()
            .map(|entry| {
                let snapshot = entry.snapshot;
                Thread {
                    id: entry.id,
                    preview: snapshot.preview,
                    model_provider: self.backend.id().into(),
                    model: entry.model,
                    selection_mode: None,
                    created_at: snapshot.created_at,
                    updated_at: snapshot.updated_at,
                    status: ThreadStatus {
                        kind: "idle".into(),
                        active_turn_id: None,
                        active_flags: vec![],
                    },
                    cwd: snapshot.cwd,
                    workspace_id: None,
                    root_id: None,
                    name: None,
                    message_count: (!snapshot.turns.is_empty())
                        .then_some(snapshot.turns.len().try_into().unwrap_or(u32::MAX)),
                    turns: None,
                    usage: None,
                    tool_allowlist: vec![],
                    developer_instructions: None,
                    external_tools: vec![],
                    runner: None,
                    parent_thread_id: None,
                    workspace_fork: None,
                }
            })
            .collect::<Vec<_>>();
        for handle in handles {
            let active = handle.thread.read().await.clone();
            if let Some(existing) = data.iter_mut().find(|thread| thread.id == active.id) {
                *existing = active;
            } else {
                data.push(active);
            }
        }
        data.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        if let Some(limit) = params.limit {
            data.truncate(limit);
        }
        Ok(json!({"data":data,"nextCursor":null,"backwardsCursor":null}))
    }

    async fn turn_start(&self, params: Value) -> anyhow::Result<Value> {
        let params: TurnStartParams = serde_json::from_value(params)?;
        let text = prompt_text(params.prompt, params.input);
        let selected = self.selected_model.read().await.clone();
        let model = params
            .model
            .or_else(|| selected.as_ref().map(|(model, _)| model.clone()));
        let reasoning = params
            .reasoning
            .or_else(|| selected.map(|(_, reasoning)| reasoning));
        let turn_id = self
            .command(&params.thread_id, |tx| {
                Command::Send(text, params.policy_mode, model, reasoning, tx)
            })
            .await?;
        Ok(serde_json::to_value(TurnStartResult { turn_id })?)
    }

    async fn turn_steer(&self, params: Value) -> anyhow::Result<Value> {
        let params: TurnSteerParams = serde_json::from_value(params)?;
        let text = prompt_text(params.prompt, params.input);
        let turn_id = self
            .command(&params.thread_id, |tx| {
                Command::Send(text, None, None, None, tx)
            })
            .await?;
        if turn_id != params.expected_turn_id {
            anyhow::bail!("active turn changed while steering");
        }
        Ok(json!({"turnId":turn_id}))
    }

    async fn turn_interrupt(&self, params: Value) -> anyhow::Result<Value> {
        let params: TurnInterruptParams = serde_json::from_value(params)?;
        self.command(&params.thread_id, Command::Interrupt).await?;
        Ok(json!({"turnId":params.turn_id}))
    }

    async fn approve(&self, params: Value) -> anyhow::Result<Value> {
        let id = params["approvalId"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing approvalId"))?;
        let thread_id = self
            .request_routes
            .write()
            .await
            .remove(id)
            .ok_or_else(|| anyhow::anyhow!("approval request expired"))?;
        let approved = params["approved"].as_bool().unwrap_or(false);
        self.command(&thread_id, |tx| {
            Command::Approve(id.to_owned(), approved, tx)
        })
        .await?;
        Ok(json!({"resolved":true}))
    }

    async fn answer(&self, params: Value) -> anyhow::Result<Value> {
        let id = params["requestId"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing requestId"))?;
        let thread_id = self
            .request_routes
            .write()
            .await
            .remove(id)
            .ok_or_else(|| anyhow::anyhow!("question request expired"))?;
        let answers = params["answers"]
            .as_object()
            .map(|answers| {
                answers
                    .iter()
                    .filter_map(|(key, value)| {
                        value
                            .as_str()
                            .map(|answer| (key.clone(), answer.to_owned()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.command(&thread_id, |tx| Command::Answer(id.to_owned(), answers, tx))
            .await?;
        Ok(json!({"resolved":true}))
    }

    async fn command<T>(
        &self,
        thread_id: &str,
        make: impl FnOnce(oneshot::Sender<anyhow::Result<T>>) -> Command,
    ) -> anyhow::Result<T> {
        let handle = self
            .threads
            .read()
            .await
            .get(thread_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("unknown backend thread {thread_id}"))?;
        let (tx, rx) = oneshot::channel();
        handle.commands.send(make(tx)).await?;
        rx.await?
    }
}

mod catalog;
mod events;
mod session;
use session::{prompt_text, run_session};
