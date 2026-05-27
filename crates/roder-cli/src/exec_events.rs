use roder_api::inference::TokenUsage;
use roder_api::thread::ThreadItemStatus;
use roder_protocol::Item;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) struct ExecUsage {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_hit_rate: Option<f64>,
}

impl Default for ExecUsage {
    fn default() -> Self {
        Self {
            input_tokens: 0,
            cached_input_tokens: 0,
            output_tokens: 0,
            reasoning_output_tokens: 0,
            cache_hit_rate: None,
        }
    }
}

impl From<Option<TokenUsage>> for ExecUsage {
    fn from(usage: Option<TokenUsage>) -> Self {
        let Some(usage) = usage else {
            return Self::default();
        };
        Self {
            input_tokens: u64::from(usage.prompt_tokens),
            cached_input_tokens: u64::from(usage.cached_prompt_tokens),
            output_tokens: u64::from(usage.completion_tokens),
            reasoning_output_tokens: 0,
            cache_hit_rate: usage.cache_hit_rate,
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
        match item {
            Item::UserMessage {
                id, text, status, ..
            } => Self {
                id,
                kind: "userMessage".to_string(),
                text: Some(text),
                status: status.map(exec_status),
                phase: None,
                tool_name: None,
                tool_call_id: None,
                payload: None,
            },
            Item::AgentMessage {
                id,
                text,
                phase,
                status,
            } => Self {
                id,
                kind: "agentMessage".to_string(),
                text: Some(text),
                status: status.map(exec_status),
                phase,
                tool_name: None,
                tool_call_id: None,
                payload: None,
            },
            Item::Reasoning {
                id,
                content,
                status,
                ..
            } => Self {
                id,
                kind: "reasoning".to_string(),
                text: Some(content.join("")),
                status: status.map(exec_status),
                phase: Some("reasoning".to_string()),
                tool_name: None,
                tool_call_id: None,
                payload: None,
            },
            Item::ToolExecution {
                id,
                tool_call_id,
                tool_name,
                status,
                input,
                output,
                error,
            } => Self {
                id,
                kind: "toolExecution".to_string(),
                text: error.or(output),
                status: Some(exec_status(status)),
                phase: None,
                tool_name: Some(tool_name),
                tool_call_id: Some(tool_call_id),
                payload: input,
            },
            Item::Compaction {
                id,
                summary,
                status,
            } => Self {
                id,
                kind: "compaction".to_string(),
                text: Some(summary),
                status: status.map(exec_status),
                phase: None,
                tool_name: None,
                tool_call_id: None,
                payload: None,
            },
            Item::Error {
                id,
                message,
                status,
            } => Self {
                id,
                kind: "error".to_string(),
                text: Some(message),
                status: status.map(exec_status),
                phase: None,
                tool_name: None,
                tool_call_id: None,
                payload: None,
            },
            Item::Raw {
                id,
                payload,
                status,
            } => Self {
                id,
                kind: "raw".to_string(),
                text: None,
                status: status.map(exec_status),
                phase: None,
                tool_name: None,
                tool_call_id: None,
                payload: Some(payload),
            },
        }
    }
}

fn exec_status(status: ThreadItemStatus) -> String {
    match status {
        ThreadItemStatus::InProgress => "inProgress",
        ThreadItemStatus::Completed => "completed",
        ThreadItemStatus::Failed => "failed",
    }
    .to_string()
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

    #[test]
    fn exec_usage_preserves_cache_metrics() {
        let usage = ExecUsage::from(Some(
            TokenUsage::new(100, 10, 110).with_cached_prompt_tokens(92),
        ));

        let value = serde_json::to_value(usage).unwrap();
        assert_eq!(value["input_tokens"], 100);
        assert_eq!(value["cached_input_tokens"], 92);
        assert!((value["cache_hit_rate"].as_f64().unwrap() - 0.92).abs() < f64::EPSILON);
    }
}
