use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;

use anyhow::{Context, anyhow, bail};
use async_trait::async_trait;
use roder_api::backend::{
    AgentBackend, AgentBackendSession, BackendEvent, BackendModel, BackendQuestion, BackendStart,
    BackendThreadSnapshot, BackendThreadSummary,
};
use roder_api::policy_mode::PolicyMode;
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use tokio::process::{Child, ChildStdin};
use tokio::sync::mpsc;

/// Runs a private Codex app-server over its stable stdio JSONL transport.
#[derive(Clone)]
pub struct CodexBackend {
    program: PathBuf,
}

impl Default for CodexBackend {
    fn default() -> Self {
        Self {
            program: PathBuf::from("codex"),
        }
    }
}

impl CodexBackend {
    pub fn with_program(program: PathBuf) -> Self {
        Self { program }
    }
}

#[async_trait]
impl AgentBackend for CodexBackend {
    fn id(&self) -> &'static str {
        "codex"
    }

    async fn connect(&self, start: BackendStart) -> anyhow::Result<Box<dyn AgentBackendSession>> {
        let mut session = self.spawn_session(&start.cwd).await?;
        let result = if let Some(thread_id) = start.resume_thread {
            session
                .request("thread/resume", json!({"threadId":thread_id}))
                .await?
        } else {
            let mut params = json!({"cwd":start.cwd,"serviceName":"roder"});
            let (approval, sandbox) = mapping::codex_policy(start.policy_mode);
            params["approvalPolicy"] = approval.into();
            params["sandbox"] = mapping::thread_sandbox(sandbox).into();
            if let Some(model) = start.model {
                params["model"] = model.into();
            }
            session.request("thread/start", params).await?
        };
        session.thread_id = result
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .context("Codex thread response had no thread.id")?
            .to_owned();
        session.model = result
            .pointer("/thread/model")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        session.reasoning = result
            .pointer("/thread/reasoningEffort")
            .and_then(Value::as_str)
            .unwrap_or("medium")
            .to_owned();
        session.snapshot = mapping::snapshot_from_thread(&result["thread"]);
        Ok(Box::new(session))
    }

    async fn list_threads(
        &self,
        cwd: PathBuf,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<BackendThreadSummary>> {
        let mut session = self.spawn_session(&cwd).await?;
        let result = session
            .request(
                "thread/list",
                json!({"cwd":cwd,"limit":limit.unwrap_or(100)}),
            )
            .await?;
        Ok(result["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|thread| {
                Some(BackendThreadSummary {
                    id: thread["id"].as_str()?.to_owned(),
                    model: thread["model"].as_str().unwrap_or("").to_owned(),
                    snapshot: mapping::snapshot_from_thread(thread),
                })
            })
            .collect())
    }

    async fn list_models(&self, cwd: PathBuf) -> anyhow::Result<Vec<BackendModel>> {
        let mut session = self.spawn_session(&cwd).await?;
        let config = session.request("config/read", json!({})).await?;
        let configured_model = config.pointer("/config/model").and_then(Value::as_str);
        let configured_reasoning = config
            .pointer("/config/model_reasoning_effort")
            .and_then(Value::as_str);
        let result = session.request("model/list", json!({})).await?;
        Ok(result["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|model| {
                Some(BackendModel {
                    id: model["model"].as_str()?.to_owned(),
                    name: model["displayName"]
                        .as_str()
                        .unwrap_or_else(|| model["model"].as_str().unwrap_or(""))
                        .to_owned(),
                    is_default: configured_model
                        .map(|configured| configured == model["model"].as_str().unwrap_or(""))
                        .unwrap_or_else(|| model["isDefault"].as_bool().unwrap_or(false)),
                    default_reasoning: if configured_model == model["model"].as_str() {
                        configured_reasoning
                    } else {
                        None
                    }
                    .or_else(|| model["defaultReasoningEffort"].as_str())
                    .unwrap_or("medium")
                    .to_owned(),
                    reasoning_efforts: model["supportedReasoningEfforts"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|effort| effort["reasoningEffort"].as_str().map(str::to_owned))
                        .collect(),
                })
            })
            .collect())
    }
}

struct CodexSession {
    _child: Child,
    stdin: ChildStdin,
    rx: mpsc::Receiver<Value>,
    queued: VecDeque<Value>,
    derived: VecDeque<BackendEvent>,
    next_id: u64,
    thread_id: String,
    model: String,
    reasoning: String,
    turn_id: Option<String>,
    streamed: HashSet<String>,
    streamed_tool_output: HashSet<String>,
    usage_baseline: Option<usage::UsageCounts>,
    approvals: HashMap<String, PendingApproval>,
    snapshot: BackendThreadSnapshot,
}

struct PendingApproval {
    id: Value,
    result_on_approval: Value,
    result_on_denial: Value,
}

impl CodexSession {
    async fn write(&mut self, value: &Value) -> anyhow::Result<()> {
        self.stdin
            .write_all(serde_json::to_string(value)?.as_bytes())
            .await?;
        self.stdin.write_all(b"\n").await?;
        self.stdin.flush().await?;
        Ok(())
    }

    async fn request(&mut self, method: &str, params: Value) -> anyhow::Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        self.write(&json!({"id":id,"method":method,"params":params}))
            .await?;
        loop {
            let message = self.rx.recv().await.context("Codex app-server closed")?;
            if message.get("id") == Some(&json!(id)) {
                if let Some(error) = message.get("error") {
                    bail!(
                        "Codex {method}: {}",
                        error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown error")
                    );
                }
                return message
                    .get("result")
                    .cloned()
                    .context("Codex response missing result");
            }
            self.queued.push_back(message);
        }
    }
}

mod events;
mod mapping;
mod transport;
mod usage;
use mapping::{approval_description, string_at, tool_event};

#[async_trait]
impl AgentBackendSession for CodexSession {
    fn thread_id(&self) -> &str {
        &self.thread_id
    }

    fn model(&self) -> &str {
        &self.model
    }
    fn reasoning(&self) -> &str {
        &self.reasoning
    }
    fn snapshot(&self) -> BackendThreadSnapshot {
        self.snapshot.clone()
    }

    async fn send(
        &mut self,
        text: String,
        policy_mode: Option<PolicyMode>,
        model: Option<String>,
        reasoning: Option<String>,
    ) -> anyhow::Result<String> {
        let input = json!([{"type":"text","text":text}]);
        let result = if let Some(turn_id) = &self.turn_id {
            self.request(
                "turn/steer",
                json!({"threadId":self.thread_id,"expectedTurnId":turn_id,"input":input}),
            )
            .await?
        } else {
            let mut params = json!({"threadId":self.thread_id,"input":input});
            if let Some(mode) = policy_mode {
                let (approval, sandbox) = mapping::codex_policy(mode);
                params["approvalPolicy"] = approval.into();
                params["sandboxPolicy"] = json!({"type":sandbox});
            }
            if let Some(model) = model {
                params["model"] = model.into();
            }
            if let Some(reasoning) = reasoning {
                params["effort"] = reasoning.into();
            }
            self.request("turn/start", params).await?
        };
        if self.turn_id.is_none() {
            self.turn_id = result
                .pointer("/turn/id")
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
        self.turn_id
            .clone()
            .context("Codex turn response had no turn id")
    }

    async fn interrupt(&mut self) -> anyhow::Result<()> {
        if let Some(turn_id) = self.turn_id.clone() {
            self.request(
                "turn/interrupt",
                json!({"threadId":self.thread_id,"turnId":turn_id}),
            )
            .await?;
        }
        Ok(())
    }

    async fn archive(&mut self) -> anyhow::Result<()> {
        self.request("thread/archive", json!({"threadId":self.thread_id}))
            .await?;
        Ok(())
    }

    async fn approve(&mut self, request_id: &str, approved: bool) -> anyhow::Result<()> {
        let request = self
            .approvals
            .remove(request_id)
            .ok_or_else(|| anyhow!("approval request expired"))?;
        self.write(&json!({"id":request.id,"result":if approved { request.result_on_approval } else { request.result_on_denial }})).await
    }

    async fn answer(
        &mut self,
        request_id: &str,
        answers: Vec<(String, String)>,
    ) -> anyhow::Result<()> {
        let request = self
            .approvals
            .remove(request_id)
            .ok_or_else(|| anyhow!("question request expired"))?;
        let answers = answers
            .into_iter()
            .map(|(key, answer)| (key, json!({"answers":[answer]})))
            .collect::<serde_json::Map<String, Value>>();
        self.write(&json!({"id":request.id,"result":{"answers":answers}}))
            .await
    }

    async fn next_event(&mut self) -> anyhow::Result<BackendEvent> {
        loop {
            if let Some(event) = self.derived.pop_front() {
                return Ok(event);
            }
            let message = if let Some(message) = self.queued.pop_front() {
                message
            } else {
                self.rx.recv().await.context("Codex app-server closed")?
            };
            if message.get("id").is_some()
                && message.get("method").is_some()
                && !matches!(
                    message.get("method").and_then(Value::as_str),
                    Some(
                        "item/commandExecution/requestApproval"
                            | "item/fileChange/requestApproval"
                            | "item/permissions/requestApproval"
                            | "item/tool/requestUserInput"
                    )
                )
            {
                let id = message["id"].clone();
                self.write(&json!({"id":id,"error":{"code":-32601,"message":"Roder backend does not support this client request"}})).await?;
            }
            if let Some(event) = self.event(message) {
                return Ok(event);
            }
        }
    }
}

#[cfg(test)]
mod tests;
