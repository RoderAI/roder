//! Upstream results that mean the action did not happen although the server
//! reports them as ordinary text.
//!
//! The pinned browser-use release answers a stale element index with the
//! plain text `Element with index N not found`, and a few dead ends with fixed
//! `Error: ...` strings, all with `isError` unset (`browser_use/mcp/server.py`
//! of [`crate::DEFAULT_PACKAGE`]). Core counts consecutive failures from
//! `is_error`, so a model re-clicking a dead index never trips it. Roder marks
//! exactly these texts as errors.
//!
//! Matching is on the whole text of one content item: no substring, no case
//! folding, no guessing at an `Error:` prefix. Page-derived text (extracted
//! content, HTML, an observation) must not be able to flip a result, and a
//! false positive feeds core's failure stop. Exceptions inside the server
//! already arrive with `isError` set and are left alone.

use serde_json::{Value, json};

/// The autonomous agent's remote tool, the only one whose results can carry
/// [`AGENT_FAILED_PREFIX`].
const AGENT_REMOTE: &str = "retry_with_browser_use_agent";

/// Fixed `Error: ...` results of the pinned server. Each is the whole text.
const PINNED_ERRORS: &[&str] = &[
    "Error: No browser session active",
    "Error: Provide either index or both coordinate_x and coordinate_y",
    "Error: No active CDP session",
    "Error: Could not get page HTML",
    "Error: LLM not initialized (set OPENAI_API_KEY)",
    "Error: FileSystem not initialized",
    "Error: Tools not initialized",
    "Error: OPENAI_API_KEY not set in config or environment",
];

/// How the pinned agent runner words a run that raised; the exception text
/// follows. Only the server writes this at the start of its result, because
/// a finished run starts with `Task completed in`.
const AGENT_FAILED_PREFIX: &str = "Agent task failed: ";

/// Whether `text`, one content item of a `remote` tool's result, is a pinned
/// failure.
pub(crate) fn is_failed_text(remote: &str, text: &str) -> bool {
    PINNED_ERRORS.contains(&text)
        || is_missing_element(text)
        || (remote == AGENT_REMOTE && text.starts_with(AGENT_FAILED_PREFIX))
}

/// `Element with index {N} not found`, N a Python integer.
fn is_missing_element(text: &str) -> bool {
    text.strip_prefix("Element with index ")
        .and_then(|rest| rest.strip_suffix(" not found"))
        .is_some_and(|index| {
            let digits = index.strip_prefix('-').unwrap_or(index);
            !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
        })
}

/// Sets `isError` on a raw `CallToolResult` when one of its text items is a
/// pinned failure.
pub(crate) fn mark_failed(remote: &str, result: &mut Value) {
    let Some(object) = result.as_object_mut() else {
        return;
    };
    if object.get("isError").and_then(Value::as_bool) == Some(true) {
        return;
    }
    let failed = object
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|item| item.get("text").and_then(Value::as_str))
        .any(|text| is_failed_text(remote, text));
    if failed {
        object.insert("isError".into(), json!(true));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DEFAULT_PACKAGE;

    /// The strings above were read from this release. A bump must re-read
    /// `browser_use/mcp/server.py` and update both, then re-run the ignored
    /// live test, which clicks a stale index against the real server.
    #[test]
    fn the_strings_belong_to_the_release_they_were_read_from() {
        assert_eq!(
            DEFAULT_PACKAGE, "browser-use[cli]==0.13.10",
            "browser-use pin changed: re-verify failed_action.rs against the new \
             browser_use/mcp/server.py (Element with index N not found, the Error: strings, \
             Agent task failed:)"
        );
    }

    #[test]
    fn stale_indexes_are_failures() {
        for index in ["0", "7", "424242", "-1"] {
            let text = format!("Element with index {index} not found");
            assert!(is_failed_text("browser_click", &text), "{text}");
            assert!(is_failed_text("browser_type", &text), "{text}");
        }
    }

    #[test]
    fn every_pinned_error_string_is_a_failure() {
        assert_eq!(PINNED_ERRORS.len(), 8);
        for text in PINNED_ERRORS {
            assert!(text.starts_with("Error: "), "{text}");
            assert!(is_failed_text("browser_get_state", text), "{text}");
        }
    }

    #[test]
    fn near_misses_are_not_failures() {
        for text in [
            "element with index 7 not found",
            "Element with index  not found",
            "Element with index seven not found",
            "Element with index 7.5 not found",
            "Element with index -  not found",
            "Element with index 7 not found.",
            "Element with index 7 not found\nand more",
            " Element with index 7 not found",
            "Element with index 7 not found in the page",
            "Element with index 7",
            "Clicked element 7",
            "Navigated to: https://example.com/",
            "Error:",
            "Error: No browser session active.",
            "error: No browser session active",
            "Error: something the pinned server never says",
            "Navigated to: Error: No browser session active",
            "Task completed in 3 steps\nAgent task failed: x",
        ] {
            assert!(!is_failed_text("browser_click", text), "{text:?}");
        }
    }

    #[test]
    fn the_agent_failure_prefix_counts_only_for_the_agent_tool() {
        let text = "Agent task failed: rate limited";
        assert!(is_failed_text("retry_with_browser_use_agent", text));
        assert!(!is_failed_text("browser_extract_content", text));
        assert!(!is_failed_text(
            "retry_with_browser_use_agent",
            "Task completed in 4 steps\nSuccess: False"
        ));
    }

    #[test]
    fn mark_failed_flags_a_pinned_text_and_nothing_else() {
        let mut dead = json!({
            "content": [{"type": "text", "text": "Element with index 9 not found"}],
            "isError": false
        });
        mark_failed("browser_click", &mut dead);
        assert_eq!(dead["isError"], true);

        let mut missing_flag = json!({
            "content": [{"type": "text", "text": "Error: No browser session active"}]
        });
        mark_failed("browser_get_state", &mut missing_flag);
        assert_eq!(missing_flag["isError"], true);

        let mut fine = json!({
            "content": [{"type": "text", "text": "Clicked element 9"}],
            "isError": false
        });
        mark_failed("browser_click", &mut fine);
        assert_eq!(fine["isError"], false);

        let mut untouched = json!({"content": [{"type": "text", "text": "Clicked element 9"}]});
        mark_failed("browser_click", &mut untouched);
        assert!(untouched.get("isError").is_none());
    }

    #[test]
    fn mark_failed_ignores_non_text_items_and_odd_shapes() {
        let mut image = json!({
            "content": [{"type": "image", "text": "Element with index 9 not found"}]
        });
        mark_failed("browser_click", &mut image);
        assert!(image.get("isError").is_none());

        for mut odd in [
            json!("Element with index 9 not found"),
            json!(null),
            json!([]),
        ] {
            let before = odd.clone();
            mark_failed("browser_click", &mut odd);
            assert_eq!(odd, before);
        }
    }

    #[test]
    fn mark_failed_does_not_clear_a_server_reported_error() {
        let mut result = json!({
            "content": [{"type": "text", "text": "Error: boom"}],
            "isError": true
        });
        mark_failed("browser_click", &mut result);
        assert_eq!(result["isError"], true);
    }
}
