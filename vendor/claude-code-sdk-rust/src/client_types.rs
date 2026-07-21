use std::collections::HashMap;

use crate::types::{ContentBlock, RateLimitInfo};

/// Response from sending a message to Claude.
#[derive(Debug, Clone)]
pub struct MessageResponse {
    pub content: String,
    pub blocks: Vec<ContentBlock>,
    pub model: String,
    pub stop_reason: Option<String>,
    pub session_id: String,
    pub usage: Option<HashMap<String, serde_json::Value>>,
}

/// Events that can occur during streaming.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    ContentChunk(String),
    ThinkingChunk {
        thinking: String,
        signature: Option<String>,
    },
    ToolUseStart {
        id: String,
        name: String,
        input: serde_json::Map<String, serde_json::Value>,
    },
    ToolUseDelta {
        id: String,
        partial_input: String,
    },
    ToolResult {
        tool_use_id: String,
        content: Option<serde_json::Value>,
        is_error: Option<bool>,
    },
    RateLimit(RateLimitInfo),
    /// A single assistant message (or usage-bearing message delta) finished.
    /// Under partial messages a turn with tool calls emits several of these —
    /// one per assistant message. Do NOT treat this as end-of-turn.
    Complete(MessageResponse),
    /// The CLI's final result message: the whole turn (including any tool
    /// calls and follow-up assistant messages) is done.
    TurnComplete(MessageResponse),
    Error(String),
}

impl MessageResponse {
    pub fn has_tool_uses(&self) -> bool {
        self.blocks
            .iter()
            .any(|b| matches!(b, ContentBlock::ToolUse { .. }))
    }

    pub fn get_tool_uses(&self) -> Vec<&ContentBlock> {
        self.blocks
            .iter()
            .filter(|b| matches!(b, ContentBlock::ToolUse { .. }))
            .collect()
    }
}
