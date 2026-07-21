pub mod client;
mod client_stream;
pub mod client_types;
pub mod error;
pub mod mcp;
pub mod options;
pub mod query;
pub mod session_store;
pub mod session_summary;
pub mod sessions;
pub mod types;

/// SDK crate version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod internal {
    pub mod cli_args;
    #[cfg(test)]
    mod cli_args_tests;
    pub mod cli_discovery;
    pub mod control;
    pub mod message_parser;
    pub mod parser;
    pub mod protocol;
    pub mod runtime;
    pub mod sdk_mcp;
    pub mod session_resume;
    #[cfg(test)]
    mod session_resume_tests;
    pub mod session_store_validation;
    pub mod sessions_fs;
    mod stdout_decoder;
    pub mod transcript_mirror;
    pub mod transport;
}

// Re-export commonly used types
pub use types::*;

pub type McpServerInfo = types::MCPServerInfo;
pub type McpServerStatus = types::MCPServerStatus;
pub type McpStatusResponse = types::MCPStatusResponse;
pub type McpToolAnnotations = types::MCPToolAnnotations;
pub type McpToolInfo = types::MCPToolInfo;

// Re-export error types
pub use error::{
    CLIConnectionError, CLIJSONDecodeError, CLINotFoundError, ClaudeSDKError, MessageParseError,
    ProcessError, Result,
};

// Re-export options types
pub use options::ClaudeAgentOptionsBuilder;

// Re-export query function and types
pub use query::{
    query, query_messages, query_messages_with_transport, query_stream_messages,
    query_stream_messages_with_transport, QueryResult, TokenUsage,
};

// Re-export client types
pub use client::{ClaudeAgentClient, SpawnedStream, SpawnedStreamCleanup};
pub use client_types::{MessageResponse, StreamEvent};

/// Python SDK-compatible alias for the interactive client.
pub type ClaudeSDKClient = ClaudeAgentClient;

// Re-export in-process MCP helpers
pub use mcp::{
    create_sdk_mcp_server, create_sdk_mcp_server_with_version, initialize_server, tool,
    tool_with_annotations, MCPContent, MCPTool, MCPToolAnnotations, SdkMcpTool, SimpleMCPServer,
};

// Re-export session store and session helpers
pub use session_store::{
    project_key_for_directory, InMemorySessionStore, SessionKey, SessionListSubkeysKey,
    SessionStore, SessionStoreEntry, SessionStoreHandle, SessionStoreListEntry,
    SessionSummaryEntry,
};
pub use sessions::{
    delete_session, delete_session_via_store, fork_session, fork_session_via_store,
    get_session_info, get_session_info_from_store, get_session_messages,
    get_session_messages_from_store, get_subagent_messages, get_subagent_messages_from_store,
    import_session_to_store, list_sessions, list_sessions_from_store, list_subagents,
    list_subagents_from_store, rename_session, rename_session_via_store, tag_session,
    tag_session_via_store, ForkSessionResult, ImportSessionOptions, ListSessionsOptions,
    LocalForkSessionResult, SDKSessionInfo, SDKSessionMessage, SessionInfo, SessionMessage,
    SessionMutationOptions, SessionQueryOptions,
};
