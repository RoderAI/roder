use roder_protocol::Item;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) struct ExecUsage {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: u64,
}

impl Default for ExecUsage {
    fn default() -> Self {
        Self {
            input_tokens: 0,
            cached_input_tokens: 0,
            output_tokens: 0,
            reasoning_output_tokens: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "type")]
pub(crate) enum ExecEvent {
    #[serde(rename = "thread.started")]
    ThreadStarted { thread_id: String },
    #[serde(rename = "turn.started")]
    TurnStarted { turn_id: String },
    #[serde(rename = "item.started")]
    ItemStarted { item: ExecItem },
    #[serde(rename = "item.updated")]
    ItemUpdated { item: ExecItem },
    #[serde(rename = "item.completed")]
    ItemCompleted { item: ExecItem },
    #[serde(rename = "turn.completed")]
    TurnCompleted { usage: ExecUsage },
    #[serde(rename = "turn.failed")]
    TurnFailed { error: String, usage: ExecUsage },
    #[serde(rename = "error")]
    Error { message: String },
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct ExecItem {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

impl From<Item> for ExecItem {
    fn from(item: Item) -> Self {
        Self {
            id: item.id,
            kind: item.kind,
            text: item.text,
            status: item.status,
            phase: item.phase,
            tool_name: item.tool_name,
            tool_call_id: item.tool_call_id,
            payload: item.payload,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_events_serialize_stable_jsonl_shape() {
        let event = ExecEvent::ItemStarted {
            item: ExecItem {
                id: "item_1".to_string(),
                kind: "command_execution".to_string(),
                text: None,
                status: Some("inProgress".to_string()),
                phase: None,
                tool_name: Some("shell".to_string()),
                tool_call_id: None,
                payload: None,
            },
        };

        let value = serde_json::to_value(event).unwrap();
        assert_eq!(value["type"], "item.started");
        assert_eq!(value["item"]["id"], "item_1");
        assert_eq!(value["item"]["type"], "command_execution");
        assert_eq!(value["item"]["tool_name"], "shell");
    }
}
