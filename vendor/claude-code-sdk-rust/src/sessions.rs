//! Session management for Claude Agent SDK.

use crate::error::Result;
use crate::internal::sessions_fs;
use serde::{Deserialize, Serialize};

pub mod store;
pub use store::*;
pub mod store_fork;
pub use store_fork::*;
pub mod import;
pub use import::*;
pub mod local_fork;
pub use local_fork::*;

/// Information about a Claude session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub message_count: usize,
}

/// A message within a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    pub timestamp: String,
}

/// Options for listing sessions.
#[derive(Debug, Clone, Default)]
pub struct ListSessionsOptions {
    pub directory: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

/// Options for querying sessions.
#[derive(Debug, Clone)]
pub struct SessionQueryOptions {
    pub session_id: String,
    pub include_messages: bool,
    pub directory: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

/// Options for mutating sessions.
#[derive(Debug, Clone)]
pub struct SessionMutationOptions {
    pub session_id: String,
    pub directory: Option<String>,
}

/// List all available sessions.
pub async fn list_sessions(opts: &ListSessionsOptions) -> Result<Vec<SessionInfo>> {
    sessions_fs::list_sessions(opts).await
}

/// Get information about a specific session.
pub async fn get_session_info(session_id: &str, opts: &SessionQueryOptions) -> Result<SessionInfo> {
    sessions_fs::get_session_info(session_id, opts.directory.as_deref()).await
}

/// Get all messages for a specific session.
pub async fn get_session_messages(
    session_id: &str,
    opts: &SessionQueryOptions,
) -> Result<Vec<SessionMessage>> {
    sessions_fs::get_session_messages(
        session_id,
        opts.directory.as_deref(),
        opts.limit,
        opts.offset.unwrap_or(0),
    )
    .await
}

/// List subagent IDs for a specific session.
pub async fn list_subagents(session_id: &str, opts: &SessionQueryOptions) -> Result<Vec<String>> {
    sessions_fs::list_subagents(session_id, opts.directory.as_deref()).await
}

/// Get messages for a specific subagent transcript.
pub async fn get_subagent_messages(
    session_id: &str,
    agent_id: &str,
    opts: &SessionQueryOptions,
) -> Result<Vec<SessionMessage>> {
    sessions_fs::get_subagent_messages(
        session_id,
        agent_id,
        opts.directory.as_deref(),
        opts.limit,
        opts.offset.unwrap_or(0),
    )
    .await
}

/// Rename a session.
pub async fn rename_session(
    session_id: &str,
    title: &str,
    opts: &SessionMutationOptions,
) -> Result<()> {
    sessions_fs::rename_session(session_id, title, opts.directory.as_deref()).await
}

/// Tag a session. Pass `None` to clear the tag.
pub async fn tag_session(
    session_id: &str,
    tag: Option<&str>,
    opts: &SessionMutationOptions,
) -> Result<()> {
    sessions_fs::tag_session(session_id, tag, opts.directory.as_deref()).await
}

/// Fork a local session transcript into a new session file.
pub async fn fork_session(
    session_id: &str,
    opts: &SessionMutationOptions,
    up_to_message_id: Option<&str>,
    title: Option<&str>,
) -> Result<LocalForkSessionResult> {
    local_fork::fork_session(
        session_id,
        opts.directory.as_deref(),
        up_to_message_id,
        title,
    )
    .await
}

/// Delete a session.
pub async fn delete_session(session_id: &str, opts: &SessionMutationOptions) -> Result<()> {
    sessions_fs::delete_session(session_id, opts.directory.as_deref()).await
}
