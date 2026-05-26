use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Context;
use roder_protocol::{
    AgentMessageDeltaNotification, Item, ItemCompletedNotification, ItemStartedNotification,
    JsonRpcNotification, TurnCompletedNotification, TurnStartedNotification,
};

use crate::exec_events::{ExecEvent, ExecItem, ExecUsage};

pub(crate) struct ExecOutput {
    json: bool,
    output_last_message: Option<PathBuf>,
    message_items: HashMap<String, String>,
    emitted_completed: bool,
    final_message: Option<String>,
}

impl ExecOutput {
    pub(crate) fn new(json: bool, output_last_message: Option<PathBuf>) -> Self {
        Self {
            json,
            output_last_message,
            message_items: HashMap::new(),
            emitted_completed: false,
            final_message: None,
        }
    }

    pub(crate) fn emit_event(&mut self, event: ExecEvent) -> anyhow::Result<()> {
        if self.json {
            println!("{}", serde_json::to_string(&event)?);
        }
        Ok(())
    }

    pub(crate) fn emit_error(&mut self, message: impl Into<String>) -> anyhow::Result<()> {
        let message = message.into();
        if self.json {
            self.emit_event(ExecEvent::Error { message })
        } else {
            eprintln!("{message}");
            Ok(())
        }
    }

    pub(crate) fn process_notification(
        &mut self,
        notification: &JsonRpcNotification,
        thread_id: &str,
        turn_id: &str,
    ) -> anyhow::Result<Option<TurnTerminalState>> {
        match notification.method.as_str() {
            "turn/started" => {
                let params: TurnStartedNotification =
                    serde_json::from_value(notification.params.clone())?;
                if params.thread_id == thread_id && params.turn.id == turn_id {
                    self.emit_event(ExecEvent::TurnStarted {
                        turn_id: params.turn.id,
                    })?;
                }
            }
            "item/started" => {
                let params: ItemStartedNotification =
                    serde_json::from_value(notification.params.clone())?;
                if params.thread_id == thread_id && params.turn_id == turn_id {
                    self.emit_event(ExecEvent::ItemStarted {
                        item: params.item.into(),
                    })?;
                }
            }
            "item/completed" => {
                let params: ItemCompletedNotification =
                    serde_json::from_value(notification.params.clone())?;
                if params.thread_id == thread_id && params.turn_id == turn_id {
                    if params.item.kind == "agentMessage"
                        && let Some(text) = params.item.text.as_deref()
                    {
                        self.final_message = Some(text.to_string());
                    }
                    self.emit_event(ExecEvent::ItemCompleted {
                        item: params.item.into(),
                    })?;
                }
            }
            "item/agentMessage/delta" => {
                let params: AgentMessageDeltaNotification =
                    serde_json::from_value(notification.params.clone())?;
                if params.thread_id == thread_id && params.turn_id == turn_id {
                    let text = {
                        let text = self
                            .message_items
                            .entry(params.item_id.clone())
                            .or_default();
                        text.push_str(&params.delta);
                        text.clone()
                    };
                    if params.phase.as_deref() != Some("reasoning") {
                        self.final_message = Some(text.clone());
                    }
                    self.emit_event(ExecEvent::ItemUpdated {
                        item: ExecItem {
                            id: params.item_id,
                            kind: "agentMessage".to_string(),
                            text: Some(text.clone()),
                            status: Some("inProgress".to_string()),
                            phase: params.phase,
                            tool_name: None,
                            tool_call_id: None,
                            payload: None,
                        },
                    })?;
                }
            }
            "turn/completed" => {
                let params: TurnCompletedNotification =
                    serde_json::from_value(notification.params.clone())?;
                if params.thread_id == thread_id && params.turn.id == turn_id {
                    self.emitted_completed = true;
                    let error = params.turn.error.as_ref().map(|value| value.to_string());
                    let status = params.turn.status;
                    if status == "completed" && error.is_none() {
                        self.emit_event(ExecEvent::TurnCompleted {
                            usage: params.turn.usage.into(),
                        })?;
                        return Ok(Some(TurnTerminalState::Completed));
                    }
                    let error = error.unwrap_or(status);
                    self.emit_event(ExecEvent::TurnFailed {
                        error: error.clone(),
                        usage: params.turn.usage.into(),
                    })?;
                    return Ok(Some(TurnTerminalState::Failed(error)));
                }
            }
            _ => {}
        }
        Ok(None)
    }

    pub(crate) fn backfill_final_message(&mut self, items: &[Item]) {
        if let Some(text) = items
            .iter()
            .rev()
            .find(|item| item.kind == "agentMessage" && item.phase.as_deref() != Some("reasoning"))
            .and_then(|item| item.text.clone())
        {
            self.final_message = Some(text);
        }
    }

    pub(crate) fn finish(&mut self, terminal: &TurnTerminalState) -> anyhow::Result<()> {
        let final_message = self.final_message.clone().unwrap_or_default();
        if let Some(path) = self.output_last_message.as_deref() {
            if let Some(parent) = path.parent()
                && !parent.as_os_str().is_empty()
            {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("creating {}", parent.display()))?;
            }
            std::fs::write(path, &final_message)
                .with_context(|| format!("writing {}", path.display()))?;
        }

        if !self.json && matches!(terminal, TurnTerminalState::Completed) {
            print!("{final_message}");
        }

        if self.json && !self.emitted_completed {
            match terminal {
                TurnTerminalState::Completed => self.emit_event(ExecEvent::TurnCompleted {
                    usage: ExecUsage::default(),
                })?,
                TurnTerminalState::Failed(error) => self.emit_event(ExecEvent::TurnFailed {
                    error: error.clone(),
                    usage: ExecUsage::default(),
                })?,
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TurnTerminalState {
    Completed,
    Failed(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_protocol::JsonRpcNotification;

    #[test]
    fn exec_output_tracks_agent_message_delta() {
        let mut output = ExecOutput::new(false, None);
        let notification = JsonRpcNotification {
            jsonrpc: "2.0".to_string(),
            method: "item/agentMessage/delta".to_string(),
            params: serde_json::json!({
                "threadId": "thread_1",
                "turnId": "turn_1",
                "itemId": "item_1",
                "delta": "hello",
            }),
        };

        let state = output
            .process_notification(&notification, "thread_1", "turn_1")
            .unwrap();
        assert_eq!(state, None);
        assert_eq!(output.final_message.as_deref(), Some("hello"));
    }
}
