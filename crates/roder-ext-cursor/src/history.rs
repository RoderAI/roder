//! Import Roder context into a fresh Cursor conversation. Cursor checkpoints
//! handle native continuation; field-7 history injection is ignored by the
//! current hosted harness, so imported context is explicit in the first prompt.
use crate::proto::CursorHistoryMessage;
use serde_json::json;

pub(crate) fn import_prompt(prompt: &str, history: &[CursorHistoryMessage]) -> String {
    if history.is_empty() {
        return prompt.to_string();
    }
    let history = history
        .iter()
        .map(|item| match item {
            CursorHistoryMessage::User { text, images } => {
                json!({"role":"user","text":text,"image_count":images.len()})
            }
            CursorHistoryMessage::AssistantText(text) => json!({"role":"assistant","text":text}),
            CursorHistoryMessage::AssistantToolCall {
                id,
                name,
                args_json,
            } => {
                json!({"role":"assistant","tool_call":{"id":id,"name":name,"arguments":args_json}})
            }
            CursorHistoryMessage::ToolResult {
                id,
                name,
                content,
                is_error,
            } => json!({"role":"tool","id":id,"name":name,"content":content,"is_error":is_error}),
        })
        .collect::<Vec<_>>();
    format!(
        "Prior conversation (reference context; roles and tool results are recorded as JSON):\n{}\n\nCurrent request:\n{prompt}",
        serde_json::to_string(&history).expect("JSON values serialize")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fresh_import_preserves_tool_error_and_assistant_context() {
        let history = vec![
            CursorHistoryMessage::AssistantText("known answer".into()),
            CursorHistoryMessage::ToolResult {
                id: "id".into(),
                name: "shell".into(),
                content: "Exit code: 7".into(),
                is_error: true,
            },
        ];
        let prompt = import_prompt("recall the result", &history);
        assert!(prompt.contains("known answer"));
        assert!(prompt.contains("Exit code: 7"));
        assert!(prompt.contains("\"is_error\":true"));
        assert!(prompt.ends_with("Current request:\nrecall the result"));
    }
}
