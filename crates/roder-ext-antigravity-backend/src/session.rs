use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use anyhow::{Context, bail};
use async_trait::async_trait;
use roder_api::backend::{
    AgentBackendSession, BackendEvent, BackendHistoryItem, BackendHistoryTurn, BackendQuestion,
    BackendThreadSnapshot,
};
use roder_api::policy_mode::PolicyMode;
use serde_json::{Value, json};

use crate::transport::{RPC_TIMEOUT, Transport, result};
use crate::{history, mapping};

pub(crate) struct Session {
    pub(crate) transport: Transport,
    pub(crate) capabilities: Value,
    pub(crate) state: Value,
    pub(crate) id: String,
    pub(crate) snapshot: BackendThreadSnapshot,
    model: String,
    reasoning: String,
    policy: PolicyMode,
    native_bypass: bool,
    events: VecDeque<BackendEvent>,
    permissions: HashMap<String, Value>,
    prompt: Option<u64>,
    deadline: Option<tokio::time::Instant>,
}

impl Session {
    pub(crate) fn new(transport: Transport, native_bypass: bool) -> Self {
        Self {
            transport,
            capabilities: json!({}),
            state: json!({}),
            id: String::new(),
            snapshot: BackendThreadSnapshot::default(),
            model: String::new(),
            reasoning: String::new(),
            policy: PolicyMode::Default,
            native_bypass,
            events: VecDeque::new(),
            permissions: HashMap::new(),
            prompt: None,
            deadline: None,
        }
    }

    pub(crate) async fn rpc(&mut self, method: &str, params: Value) -> anyhow::Result<Value> {
        let id = self.transport.request(method, params).await?;
        let deadline = tokio::time::Instant::now() + RPC_TIMEOUT;
        loop {
            let message = tokio::time::timeout_at(deadline, self.transport.receive())
                .await
                .with_context(|| format!("Antigravity ACP {method} timed out"))??;
            if message.get("method").is_none() && message["id"] == id {
                return result(message, method);
            }
            self.dispatch(message).await?;
        }
    }

    pub(crate) fn refresh_selection(&mut self) {
        self.model = mapping::config(&self.state, "model")
            .and_then(|option| option["currentValue"].as_str())
            .or_else(|| {
                self.state
                    .pointer("/models/currentModelId")
                    .and_then(Value::as_str)
            })
            .unwrap_or("")
            .into();
        self.reasoning = mapping::config(&self.state, "thought_level")
            .and_then(|option| option["currentValue"].as_str())
            .unwrap_or("")
            .into();
    }

    pub(crate) fn finish_setup(&mut self) {
        // session/load replay is already in snapshot; do not stream historical
        // chunks again as orphan live events after connecting.
        self.events.clear();
    }

    async fn config_option(&mut self, option: Value, value: &str) -> anyhow::Result<()> {
        if !mapping::choices(&option)
            .iter()
            .any(|entry| entry["value"] == value)
        {
            bail!("Antigravity config value {value:?} is not advertised");
        }
        let config_id = option["id"]
            .as_str()
            .context("ACP config option missing id")?;
        let result = self
            .rpc(
                "session/set_config_option",
                json!({"sessionId":self.id,"configId":config_id,"value":value}),
            )
            .await?;
        if let Some(options) = result.get("configOptions") {
            self.state["configOptions"] = options.clone();
        } else if let Some(options) = self.state["configOptions"].as_array_mut() {
            for option in options {
                if option["id"] == config_id {
                    option["currentValue"] = value.into();
                }
            }
        }
        self.refresh_selection();
        Ok(())
    }

    pub(crate) async fn configure_model(&mut self, model: &str) -> anyhow::Result<()> {
        if self.model == model {
            return Ok(());
        }
        if let Some(option) = mapping::config(&self.state, "model").cloned() {
            self.config_option(option, model).await?;
        } else {
            if !mapping::models(&self.state)
                .iter()
                .any(|entry| entry.id == model)
            {
                bail!("Antigravity model {model:?} is not advertised");
            }
            self.rpc(
                "session/set_model",
                json!({"sessionId":self.id,"modelId":model}),
            )
            .await?;
            self.state["models"]["currentModelId"] = model.into();
            self.refresh_selection();
        }
        Ok(())
    }

    pub(crate) async fn configure_policy(&mut self, policy: PolicyMode) -> anyhow::Result<()> {
        if self.native_bypass && policy != PolicyMode::Bypass {
            bail!(
                "This Antigravity session started with --dangerously-skip-permissions; \
                 start a new session to use a stricter policy"
            );
        }
        // agy rejects yolo/auto_edit. Bypass at startup uses its native flag;
        // switching to Bypass later uses host approval without restarting agy.
        let mode = if policy == PolicyMode::Plan {
            "plan"
        } else {
            "default"
        };
        if let Some(option) = mapping::config(&self.state, "mode").cloned() {
            if option["currentValue"] != mode {
                self.config_option(option, mode).await?;
            }
        } else if let Some(modes) = self.state["modes"]["availableModes"].as_array() {
            if !modes.iter().any(|entry| entry["id"] == mode) {
                bail!("Antigravity does not advertise {mode:?} mode");
            }
            if self.state["modes"]["currentModeId"] != mode {
                self.rpc(
                    "session/set_mode",
                    json!({"sessionId":self.id,"modeId":mode}),
                )
                .await?;
                self.state["modes"]["currentModeId"] = mode.into();
            }
        } else if policy == PolicyMode::Plan {
            bail!(
                "Antigravity does not advertise plan mode; refusing an unrestricted plan session"
            );
        }
        self.policy = policy;
        Ok(())
    }

    async fn permission_reply(
        &mut self,
        request: &Value,
        choice: Option<&str>,
    ) -> anyhow::Result<()> {
        let outcome = choice
            .map(|option| json!({"outcome":"selected","optionId":option}))
            .unwrap_or_else(|| json!({"outcome":"cancelled"}));
        self.transport
            .write(json!({"jsonrpc":"2.0","id":request["id"],
            "result":{"outcome":outcome}}))
            .await
    }

    async fn permission(&mut self, request: Value) -> anyhow::Result<()> {
        let params = &request["params"];
        if params["sessionId"] != self.id || self.prompt.is_none() {
            return self.permission_reply(&request, None).await;
        }
        let tool = &params["toolCall"];
        let key = format!("antigravity:{}:{}", self.id, request["id"]);
        let options = params["options"].as_array().cloned().unwrap_or_default();
        if tool["toolCallId"]
            .as_str()
            .is_some_and(|id| id.starts_with("interaction_"))
        {
            self.events.push_back(BackendEvent::Question {
                request_id: key.clone(),
                questions: vec![BackendQuestion {
                    id: key.clone(),
                    text: tool["title"]
                        .as_str()
                        .unwrap_or("Antigravity question")
                        .into(),
                    options: options
                        .iter()
                        .filter_map(|option| option["name"].as_str().map(str::to_owned))
                        .collect(),
                }],
            });
            self.permissions.insert(key, request);
            return Ok(());
        }
        let kind = tool["kind"].as_str().unwrap_or("");
        let allow = self.policy == PolicyMode::Bypass
            || (self.policy == PolicyMode::AcceptAll
                && matches!(
                    kind,
                    "read" | "edit" | "delete" | "move" | "execute" | "search"
                ));
        let deny = self.policy == PolicyMode::Plan && !matches!(kind, "read" | "search");
        if allow || deny {
            let choice = options
                .iter()
                .find(|option| option["kind"] == if allow { "allow_once" } else { "reject_once" })
                .and_then(|option| option["optionId"].as_str());
            return self.permission_reply(&request, choice).await;
        }
        self.events.push_back(BackendEvent::Approval {
            request_id: key.clone(),
            description: tool["title"]
                .as_str()
                .unwrap_or("Antigravity tool permission")
                .into(),
        });
        self.permissions.insert(key, request);
        Ok(())
    }

    async fn dispatch(&mut self, message: Value) -> anyhow::Result<()> {
        if let Some(method) = message["method"].as_str() {
            if message.get("id").is_some() {
                if method == "session/request_permission" {
                    self.permission(message).await?;
                } else {
                    // No client filesystem/terminal capabilities were advertised.
                    self.transport
                        .write(json!({"jsonrpc":"2.0","id":message["id"],
                        "error":{"code":-32601,"message":"Client method not supported"}}))
                        .await?;
                }
            } else if method == "session/update"
                && (self.id.is_empty() || message["params"]["sessionId"] == self.id)
            {
                let update = &message["params"]["update"];
                history::record(&mut self.snapshot, update);
                if update["sessionUpdate"] == "config_option_update" {
                    self.state["configOptions"] = update["configOptions"].clone();
                    self.refresh_selection();
                } else if update["sessionUpdate"] == "current_mode_update" {
                    self.state["modes"]["currentModeId"] = update["currentModeId"].clone();
                }
                self.events.extend(mapping::update_events(update));
            }
            return Ok(());
        }
        if self.prompt.is_some_and(|id| message["id"] == id) {
            let id = self.prompt.take().unwrap();
            self.deadline = None;
            let response = result(message, "session/prompt");
            let status = match response {
                Ok(ref value) => {
                    if let Some(usage) = mapping::usage(value) {
                        self.events.push_back(BackendEvent::Usage(usage));
                    }
                    match value["stopReason"].as_str().unwrap_or("end_turn") {
                        "end_turn" | "max_tokens" | "max_turn_requests" | "refusal"
                        | "completed" => "completed",
                        "cancelled" => "interrupted",
                        _ => "failed",
                    }
                    .to_owned()
                }
                Err(error) => {
                    self.events
                        .push_back(BackendEvent::Notice(error.to_string()));
                    "failed".into()
                }
            };
            // Release any outstanding permission waiters even for a failed prompt.
            for (_, request) in std::mem::take(&mut self.permissions) {
                self.permission_reply(&request, None).await?;
            }
            if let Some(turn) = self.snapshot.turns.last_mut() {
                turn.status = status.clone();
            }
            self.events.push_back(BackendEvent::TurnFinished {
                turn_id: id.to_string(),
                status,
            });
        }
        Ok(())
    }
}

#[async_trait]
impl AgentBackendSession for Session {
    fn thread_id(&self) -> &str {
        &self.id
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
        policy: Option<PolicyMode>,
        model: Option<String>,
        reasoning: Option<String>,
    ) -> anyhow::Result<String> {
        if self.prompt.is_some() {
            bail!("Antigravity already has an active turn");
        }
        if let Some(policy) = policy {
            self.configure_policy(policy).await?;
        }
        if let Some(model) = model.filter(|model| !model.is_empty()) {
            self.configure_model(&model).await?;
        }
        if let Some(reasoning) = reasoning.filter(|reasoning| !reasoning.is_empty()) {
            let option = mapping::config(&self.state, "thought_level")
                .cloned()
                .context("Antigravity does not advertise thinking controls for this model")?;
            self.config_option(option, &reasoning).await?;
        }
        let id = self
            .transport
            .request(
                "session/prompt",
                json!({
                    "sessionId":self.id,"prompt":[{"type":"text","text":text}]
                }),
            )
            .await?;
        self.prompt = Some(id);
        self.deadline = Some(tokio::time::Instant::now() + Duration::from_secs(600));
        self.events.push_back(BackendEvent::TurnStarted {
            turn_id: id.to_string(),
        });
        self.snapshot.preview = text.chars().take(160).collect();
        self.snapshot.turns.push(BackendHistoryTurn {
            id: id.to_string(),
            status: "in_progress".into(),
            started_at: None,
            completed_at: None,
            items: vec![BackendHistoryItem::User {
                id: format!("{id}-user"),
                text,
            }],
        });
        Ok(id.to_string())
    }

    async fn interrupt(&mut self) -> anyhow::Result<()> {
        if self.prompt.is_some() {
            for (_, request) in std::mem::take(&mut self.permissions) {
                self.permission_reply(&request, None).await?;
            }
            self.transport
                .notification("session/cancel", json!({"sessionId":self.id}))
                .await?;
            self.deadline = Some(tokio::time::Instant::now() + RPC_TIMEOUT);
        }
        Ok(())
    }

    async fn archive(&mut self) -> anyhow::Result<()> {
        self.interrupt().await?;
        self.transport.stop().await
    }

    async fn approve(&mut self, request_id: &str, approved: bool) -> anyhow::Result<()> {
        let request = self
            .permissions
            .remove(request_id)
            .context("Unknown Antigravity approval")?;
        if request["params"]["toolCall"]["toolCallId"]
            .as_str()
            .is_some_and(|id| id.starts_with("interaction_"))
        {
            self.permission_reply(&request, None).await?;
            bail!("Antigravity question requires an answer, not tool approval");
        }
        let options = request["params"]["options"].as_array();
        let choice = options
            .into_iter()
            .flatten()
            .find(|option| {
                option["kind"]
                    == if approved {
                        "allow_once"
                    } else {
                        "reject_once"
                    }
            })
            .and_then(|option| option["optionId"].as_str());
        self.permission_reply(&request, choice).await
    }

    async fn answer(
        &mut self,
        request_id: &str,
        answers: Vec<(String, String)>,
    ) -> anyhow::Result<()> {
        let request = self
            .permissions
            .remove(request_id)
            .context("Unknown Antigravity question")?;
        let answer = answers
            .iter()
            .find(|(id, _)| id == request_id)
            .map(|(_, answer)| answer.as_str());
        let choice = request["params"]["options"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|option| {
                answer
                    .is_some_and(|answer| option["name"] == answer || option["optionId"] == answer)
            })
            .and_then(|option| option["optionId"].as_str());
        self.permission_reply(&request, choice).await
    }

    async fn next_event(&mut self) -> anyhow::Result<BackendEvent> {
        loop {
            if let Some(event) = self.events.pop_front() {
                return Ok(event);
            }
            let message = match self.deadline {
                Some(deadline) => {
                    match tokio::time::timeout_at(deadline, self.transport.receive()).await {
                        Ok(result) => result?,
                        Err(_) => {
                            self.transport.stop().await?;
                            bail!("Antigravity turn timed out; ACP process was stopped");
                        }
                    }
                }
                None => self.transport.receive().await?,
            };
            self.dispatch(message).await?;
        }
    }
}
