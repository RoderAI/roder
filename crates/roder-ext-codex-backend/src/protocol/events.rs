use super::*;

impl CodexSession {
    pub(super) fn event(&mut self, message: Value) -> Option<BackendEvent> {
        let method = message.get("method")?.as_str()?;
        let params = message.get("params").unwrap_or(&Value::Null);
        match method {
            "turn/started" => {
                self.usage_baseline = None;
                self.turn_id = params
                    .pointer("/turn/id")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                Some(BackendEvent::TurnStarted {
                    turn_id: self.turn_id.clone()?,
                })
            }
            "turn/completed" => {
                let turn_id = self.turn_id.take().or_else(|| {
                    params
                        .pointer("/turn/id")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })?;
                Some(BackendEvent::TurnFinished {
                    turn_id,
                    status: {
                        let status = params
                            .pointer("/turn/status")
                            .and_then(Value::as_str)
                            .unwrap_or("completed");
                        match params
                            .pointer("/turn/error/message")
                            .and_then(Value::as_str)
                        {
                            Some(error) => format!("{status}: {error}"),
                            None => status.to_owned(),
                        }
                    },
                })
            }
            "item/agentMessage/delta" => {
                if let Some(id) = params.get("itemId").and_then(Value::as_str) {
                    self.streamed.insert(id.to_owned());
                }
                params
                    .get("delta")
                    .and_then(Value::as_str)
                    .map(|text| BackendEvent::Text(text.to_owned()))
            }
            "thread/tokenUsage/updated" => {
                let (usage, metadata) = usage::usage_event(&mut self.usage_baseline, params)?;
                self.derived.push_back(metadata);
                Some(usage)
            }
            "item/started" => tool_event(params.get("item")?, false),
            "item/completed" => {
                let item = params.get("item")?;
                let id = item.get("id").and_then(Value::as_str).unwrap_or("");
                if item.get("type").and_then(Value::as_str) == Some("agentMessage")
                    && !self.streamed.remove(id)
                {
                    return item
                        .get("text")
                        .and_then(Value::as_str)
                        .map(|text| BackendEvent::Text(text.to_owned()));
                }
                if item.get("type").and_then(Value::as_str) == Some("fileChange") {
                    if let Some(changes) = item.get("changes").and_then(Value::as_array) {
                        for change in changes {
                            if let Some(path) = change.get("path").and_then(Value::as_str) {
                                self.derived.push_back(BackendEvent::FileChanged {
                                    path: path.to_owned(),
                                    change_type: change
                                        .pointer("/kind/type")
                                        .and_then(Value::as_str)
                                        .unwrap_or("update")
                                        .to_owned(),
                                });
                            }
                        }
                    }
                }
                if item.get("type").and_then(Value::as_str) == Some("commandExecution")
                    && !self.streamed_tool_output.remove(id)
                {
                    if let Some(text) = item
                        .get("aggregatedOutput")
                        .and_then(Value::as_str)
                        .filter(|text| !text.is_empty())
                    {
                        self.derived.push_back(BackendEvent::ToolOutput {
                            id: id.to_owned(),
                            text: text.to_owned(),
                        });
                    }
                }
                let completed = tool_event(item, true);
                if let Some(first) = self.derived.pop_front() {
                    if let Some(completed) = completed {
                        self.derived.push_back(completed);
                    }
                    Some(first)
                } else {
                    completed
                }
            }
            "item/commandExecution/outputDelta" => {
                let id = string_at(params, "itemId");
                self.streamed_tool_output.insert(id.clone());
                Some(BackendEvent::ToolOutput {
                    id,
                    text: string_at(params, "delta"),
                })
            }
            "item/commandExecution/requestApproval"
            | "item/fileChange/requestApproval"
            | "item/permissions/requestApproval" => {
                let id = message.get("id")?.clone();
                let key = id.to_string();
                let (result_on_approval, result_on_denial) = if method
                    == "item/permissions/requestApproval"
                {
                    (
                        json!({"permissions":params.get("permissions").cloned().unwrap_or_else(|| json!({}))}),
                        json!({"permissions":{}}),
                    )
                } else {
                    (json!({"decision":"accept"}), json!({"decision":"decline"}))
                };
                self.approvals.insert(
                    key.clone(),
                    PendingApproval {
                        id,
                        result_on_approval,
                        result_on_denial,
                    },
                );
                Some(BackendEvent::Approval {
                    request_id: key,
                    description: approval_description(method, params),
                })
            }
            "item/tool/requestUserInput" => {
                let id = message.get("id")?.clone();
                let key = id.to_string();
                self.approvals.insert(
                    key.clone(),
                    PendingApproval {
                        id,
                        result_on_approval: Value::Null,
                        result_on_denial: Value::Null,
                    },
                );
                let questions = params
                    .get("questions")?
                    .as_array()?
                    .iter()
                    .filter_map(|question| {
                        Some(BackendQuestion {
                            id: question.get("id")?.as_str()?.to_owned(),
                            text: question.get("question")?.as_str()?.to_owned(),
                            options: question
                                .get("options")
                                .and_then(Value::as_array)
                                .map(|options| {
                                    options
                                        .iter()
                                        .filter_map(|option| {
                                            option.get("label")?.as_str().map(str::to_owned)
                                        })
                                        .collect()
                                })
                                .unwrap_or_default(),
                        })
                    })
                    .collect();
                Some(BackendEvent::Question {
                    request_id: key,
                    questions,
                })
            }
            "error" => Some(BackendEvent::Notice(
                params
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("Codex error")
                    .to_owned(),
            )),
            "warning" | "configWarning" | "roder/error" => Some(BackendEvent::Notice(
                params
                    .get("message")
                    .or_else(|| params.get("summary"))
                    .and_then(Value::as_str)
                    .unwrap_or(method)
                    .to_owned(),
            )),
            _ if message.get("id").is_some() => Some(BackendEvent::Notice(format!(
                "Codex requires an unsupported client action: {method}"
            ))),
            _ => None,
        }
    }
}
