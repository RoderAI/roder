//! Per-thread MCP bearer-token registry.
//!
//! The remote app-server is a single process shared by many client sessions and
//! configures its MCP servers once at startup (a process-wide token). To let a
//! remote client scope a thread's MCP tool calls to a specific identity (for
//! Vex: a per-user-and-organization capability token), the client forwards the
//! token at `thread/start`; the app-server records it here keyed by thread id,
//! and the MCP tool extension reads it during execution via
//! [`crate::tools::ToolExecutionContext::thread_id`].
//!
//! Lives in `roder-api` so both the app-server (writer) and MCP tool extension
//! (reader) can reach it without depending on one another. The map is in-memory
//! only: tokens are re-supplied by the client on the next `thread/start`, and
//! they are expected to be short-lived capability tokens.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

fn registry() -> &'static RwLock<HashMap<String, String>> {
    static REGISTRY: OnceLock<RwLock<HashMap<String, String>>> = OnceLock::new();
    REGISTRY.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Records the MCP bearer token a remote client supplied for `thread_id`. An
/// empty token is ignored (treated as "use the process default").
pub fn set_thread_token(thread_id: impl Into<String>, token: impl Into<String>) {
    let token = token.into();
    if token.trim().is_empty() {
        return;
    }
    if let Ok(mut map) = registry().write() {
        map.insert(thread_id.into(), token);
    }
}

/// Forgets any token recorded for `thread_id` (e.g. on archive).
pub fn clear_thread_token(thread_id: &str) {
    if let Ok(mut map) = registry().write() {
        map.remove(thread_id);
    }
}

/// The MCP bearer token to use for `thread_id`, if one was registered.
pub fn thread_token(thread_id: &str) -> Option<String> {
    registry()
        .read()
        .ok()
        .and_then(|map| map.get(thread_id).cloned())
}
