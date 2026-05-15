use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TranscriptFoldState {
    collapsed_messages: BTreeSet<usize>,
    collapsed_tool_calls: BTreeSet<String>,
}

impl TranscriptFoldState {
    pub fn toggle_message(&mut self, message_idx: usize) -> bool {
        toggle_usize(&mut self.collapsed_messages, message_idx)
    }

    pub fn is_message_collapsed(&self, message_idx: usize) -> bool {
        self.collapsed_messages.contains(&message_idx)
    }

    pub fn toggle_tool_call(&mut self, call_id: impl Into<String>) -> bool {
        let call_id = call_id.into();
        toggle_string(&mut self.collapsed_tool_calls, call_id)
    }

    pub fn is_tool_call_expanded(&self, call_id: &str) -> bool {
        !self.collapsed_tool_calls.contains(call_id)
    }

    pub fn to_json_value(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("transcript fold state serializes")
    }

    pub fn from_json_value(value: serde_json::Value) -> serde_json::Result<Self> {
        serde_json::from_value(value)
    }

    pub fn visible_message(&self, message_idx: usize, message: &str) -> String {
        if !self.is_message_collapsed(message_idx) {
            return message.to_string();
        }
        folded_message(message)
    }
}

fn toggle_usize(set: &mut BTreeSet<usize>, value: usize) -> bool {
    if set.remove(&value) {
        true
    } else {
        set.insert(value);
        false
    }
}

fn toggle_string(set: &mut BTreeSet<String>, value: String) -> bool {
    if set.remove(&value) {
        true
    } else {
        set.insert(value);
        false
    }
}

fn folded_message(message: &str) -> String {
    let head = message.chars().take(96).collect::<String>();
    if head.len() == message.len() {
        format!("{head} [folded]")
    } else {
        format!("{head}... [folded]")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fold_state_round_trips_for_resume_storage() {
        let mut state = TranscriptFoldState::default();
        state.toggle_message(3);
        state.toggle_tool_call("call-1");

        let decoded = TranscriptFoldState::from_json_value(state.to_json_value()).unwrap();

        assert!(decoded.is_message_collapsed(3));
        assert!(!decoded.is_tool_call_expanded("call-1"));
    }

    #[test]
    fn message_fold_shortens_long_messages() {
        let mut state = TranscriptFoldState::default();
        state.toggle_message(1);

        let visible = state.visible_message(1, &"x".repeat(140));

        assert!(visible.ends_with("... [folded]"));
        assert!(visible.len() < 120);
    }
}
