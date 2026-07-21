//! Protocol constants for Claude CLI communication.

/// Message types used in CLI communication.
pub mod message_types {
    pub const USER: &str = "user";
    pub const ASSISTANT: &str = "assistant";
    pub const SYSTEM: &str = "system";
    pub const TASK_START: &str = "task_start";
    pub const TASK_PROGRESS: &str = "task_progress";
    pub const TASK_COMPLETE: &str = "task_complete";
    pub const TASK_USAGE: &str = "task_usage";
    pub const RESULT: &str = "result";
    pub const RATE_LIMIT: &str = "rate_limit";
    pub const STREAM_EVENT: &str = "stream_event";
    pub const SERVER_ERROR: &str = "server_error";
}

/// Content block types.
pub mod content_types {
    pub const TEXT: &str = "text";
    pub const THINKING: &str = "thinking";
    pub const TOOL_USE: &str = "tool_use";
    pub const TOOL_RESULT: &str = "tool_result";
    pub const UNKNOWN: &str = "unknown";
}

/// Stream event types.
pub mod stream_event_types {
    pub const CONTENT_BLOCK_START: &str = "content_block_start";
    pub const CONTENT_BLOCK_DELTA: &str = "content_block_delta";
    pub const CONTENT_BLOCK_STOP: &str = "content_block_stop";
    pub const MESSAGE_START: &str = "message_start";
    pub const MESSAGE_DELTA: &str = "message_delta";
    pub const MESSAGE_STOP: &str = "message_stop";
}

/// Field names for protocol maps.
pub mod fields {
    pub const TYPE: &str = "type";
    pub const SESSION_ID: &str = "session_id";
    pub const MESSAGE: &str = "message";
    pub const CONTENT: &str = "content";
    pub const ROLE: &str = "role";
    pub const MODEL: &str = "model";
    pub const STOP_REASON: &str = "stop_reason";
    pub const USAGE: &str = "usage";
    pub const INPUT_TOKENS: &str = "input_tokens";
    pub const OUTPUT_TOKENS: &str = "output_tokens";
    pub const TOTAL_TOKENS: &str = "total_tokens";
    pub const ERROR: &str = "error";
    pub const EVENT: &str = "event";
    pub const ID: &str = "id";
    pub const NAME: &str = "name";
    pub const INPUT: &str = "input";
    pub const OUTPUT: &str = "output";
    pub const IS_ERROR: &str = "is_error";
    pub const THINKING: &str = "thinking";
    pub const SIGNATURE: &str = "signature";
    pub const DELTA: &str = "delta";
    pub const TEXT: &str = "text";
    pub const PARTIAL_JSON: &str = "partial_json";
    pub const INDEX: &str = "index";
}

/// CLI argument constants.
pub mod cli_args {
    pub const VERSION: &str = "--version";
    pub const JSON: &str = "--json";
    pub const SESSION: &str = "--session";
    pub const RESUME: &str = "--resume";
    pub const PRINT_CONFIG: &str = "--print-config";
    pub const MODEL: &str = "--model";
    pub const MAX_TURNS: &str = "--max-turns";
    pub const CWD: &str = "--cwd";
    pub const NO_AUTO: &str = "--no-auto";
    pub const ONLY: &str = "--only";
}
