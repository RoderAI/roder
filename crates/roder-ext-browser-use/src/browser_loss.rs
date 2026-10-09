//! What the model is told when a browser-use browser is lost, and which
//! failures lose it.
//!
//! A call that fails after it reached the server (transport error, timeout,
//! the server dying, an unusable reply) stops the thread's server together
//! with its browser, and the owned profile directory goes with it. The next
//! call starts a fresh browser. The model has to know both facts, or it keeps
//! using element indexes and logins that no longer exist. Nothing is ever
//! restarted behind its back: the failing call reports the loss, and the call
//! that finds a fresh browser says so in its own result.
//!
//! One failure is not a loss: the server answering the action with a JSON-RPC
//! error. It proves the server is alive and speaking, so the call was turned
//! down (for example an argument that fails the tool's schema) and the
//! browser is as it was. Stopping it for a mistake in one call would throw
//! away the logins and tabs of a long session, so its error is shown as it
//! is, without the loss message.

use roder_ext_mcp::McpRpcError;
use serde_json::{Value, json};

use crate::observation::text_item;

/// How a call that returned no result ended.
pub(crate) enum CallFailure {
    /// The server answered the action with a JSON-RPC error; it and its
    /// browser are untouched.
    Rejected(anyhow::Error),
    /// Anything else: the server and its browser are stopped.
    Lost(anyhow::Error),
}

impl CallFailure {
    /// Classifies the failure of the action call itself.
    pub(crate) fn of_action(error: anyhow::Error) -> Self {
        if error.downcast_ref::<McpRpcError>().is_some() {
            Self::Rejected(error)
        } else {
            Self::Lost(error)
        }
    }
}

/// Added to the error of a call that took the browser down.
const BROWSER_LOST: &str = "The browser-use browser was shut down because of this failure: its \
     open tabs, logins and cookies are gone. Nothing was restarted. The next browser_use call \
     starts a fresh browser, so navigate and sign in again, and read the page before reusing \
     element indexes.";

/// Put in front of the first result from a browser that replaced a lost one.
const FRESH_BROWSER_NOTICE: &str = "Notice: the previous browser-use browser was lost (its server \
     exited, or an earlier call failed or was cancelled), so this call ran in a fresh browser: \
     earlier tabs, logins and cookies are gone and old element indexes no longer apply.";

/// `error` with the loss spelled out.
pub(crate) fn lost_browser_error(error: anyhow::Error) -> anyhow::Error {
    anyhow::anyhow!("{error:#}\n{BROWSER_LOST}")
}

/// Puts the fresh-browser notice first in a raw `CallToolResult`.
pub(crate) fn announce_fresh_browser(result: &mut Value) {
    let Some(object) = result.as_object_mut() else {
        return;
    };
    let content = object.entry("content").or_insert_with(|| json!([]));
    if let Some(items) = content.as_array_mut() {
        items.insert(0, text_item(FRESH_BROWSER_NOTICE));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_json_rpc_error_reply_to_the_action_is_a_rejection() {
        let reply: anyhow::Error = McpRpcError {
            code: -32602,
            message: "Invalid params".into(),
        }
        .into();
        let with_context = reply.context("tools/call browser_click");
        assert!(matches!(
            CallFailure::of_action(with_context),
            CallFailure::Rejected(_)
        ));
        for lost in [
            "MCP server browser-use has exited",
            "MCP server browser-use did not answer tools/call within 30s",
            "MCP server browser-use is shut down",
            // Only the typed reply counts, never the words of a message.
            "MCP error -32602: Invalid params",
        ] {
            assert!(
                matches!(
                    CallFailure::of_action(anyhow::anyhow!(lost)),
                    CallFailure::Lost(_)
                ),
                "{lost}"
            );
        }
    }

    #[test]
    fn the_error_keeps_its_cause_and_names_the_loss_and_the_next_step() {
        let error = lost_browser_error(anyhow::anyhow!("MCP server browser-use exited"));
        let text = format!("{error:#}");
        assert!(
            text.starts_with("MCP server browser-use exited\n"),
            "{text}"
        );
        assert!(text.contains("logins and cookies are gone"), "{text}");
        assert!(text.contains("Nothing was restarted"), "{text}");
        assert!(
            text.contains("The next browser_use call starts a fresh browser"),
            "{text}"
        );
    }

    #[test]
    fn the_notice_goes_first_and_leaves_the_rest_alone() {
        let mut result = json!({
            "content": [{"type": "text", "text": "{\"url\":\"about:blank\"}"}],
            "isError": false
        });
        announce_fresh_browser(&mut result);
        let items = result["content"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["text"], FRESH_BROWSER_NOTICE);
        assert!(FRESH_BROWSER_NOTICE.contains("ran in a fresh browser"));
        assert_eq!(items[1]["text"], "{\"url\":\"about:blank\"}");
        assert_eq!(result["isError"], false);
    }

    #[test]
    fn a_result_without_content_still_gets_the_notice() {
        let mut result = json!({});
        announce_fresh_browser(&mut result);
        assert_eq!(result["content"][0]["text"], FRESH_BROWSER_NOTICE);
        let mut not_an_object = json!("x");
        announce_fresh_browser(&mut not_an_object);
        assert_eq!(not_an_object, json!("x"));
    }
}
