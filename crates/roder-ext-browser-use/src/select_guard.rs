//! Native `<select>` dropdowns: the one element browser-use lists and cannot
//! operate.
//!
//! In the pinned release (`browser_use/mcp/server.py` and
//! `browser/watchdogs/default_action_watchdog.py` of [`crate::DEFAULT_PACKAGE`],
//! checked against the real server with a local page):
//!
//! - A click on a select is declined by the browser-side handler, which hands
//!   back a `validation_error` that the MCP server's `_click` never reads. It
//!   answers `Clicked element N` with `isError` unset, and the select is
//!   unchanged.
//! - `browser_type` first clears the field, which for a select sets its value
//!   to `""` and fires `input` and `change` events, then sends key events that
//!   Chrome can take as type-ahead. Against a local page in headless Chrome the
//!   first attempt picked the option and every following attempt left the
//!   select cleared, with the key events delivered but no selection made. The
//!   answer is `Typed '...' into element N` either way.
//! - The state lists a select as `select "Alpha Beta Cherry"`: the labels of its
//!   options and nothing about which one is selected, so the model cannot tell
//!   afterwards what it did.
//!
//! Both calls therefore fail without saying so. Roder keeps, per thread, the
//! indexes of the selects in the last page state it showed the model (an
//! explicit `browser_use_get_state`, or the state after navigation and
//! actions), and refuses a click or a typed text aimed at one of them before
//! anything is sent to the server. The refusal is an error that names the
//! routes that do work.
//!
//! The set is what the model was shown, which is what it chooses indexes from.
//! It is replaced by every state shown, cleared when a state cannot be read,
//! forgotten when a tab, a session or the browser is closed, and dropped with
//! the browser it describes. A page that was never shown to the thread, or a
//! state of another shape, refuses nothing: the call goes to the server as
//! before.
//!
//! There is deliberately no guard for stale indexes under parallel tool calls.
//! In the pinned release an element's index is its CDP backend node id: it does
//! not change when other elements are added or removed (an input inserted at the
//! top of a page left every other index as it was), and an element that was
//! removed answers `Element with index N not found`, which is already an error
//! (see `failed_action`). A call queued behind another one therefore reaches the
//! element the model chose or fails loudly. A guard comparing the page before
//! and after would only add refusals of calls that would have worked.

use std::sync::Mutex;

use serde_json::Value;

use crate::policy::BrowserUseActionClass;
use crate::state_view::Selects;

/// The selects of the last page state shown for one thread's browser.
#[derive(Default)]
pub(crate) struct ShownSelects(Mutex<Selects>);

impl ShownSelects {
    /// Records the selects of a state just shown; `None` (a state that could
    /// not be read, or no browser) forgets them.
    pub(crate) fn replace(&self, selects: Option<Selects>) {
        *self.0.lock().unwrap() = selects.unwrap_or_default();
    }

    fn contains(&self, index: u64) -> bool {
        self.0.lock().unwrap().contains(&index)
    }
}

/// Forgets what the thread was shown after a management call (closing a tab, a
/// session or the browser) that the server did not report as failed. The page
/// that was shown may be gone, and an index is only unique within a page, so
/// the same number on the page that is current now must not be refused as a
/// select it is not. Calls of another class, and a management call that
/// failed, leave the set alone.
pub(crate) fn forget_after_close(
    shown: &ShownSelects,
    class: BrowserUseActionClass,
    result: &Value,
) {
    if class == BrowserUseActionClass::Manage
        && result.get("isError").and_then(Value::as_bool) != Some(true)
    {
        shown.replace(None);
    }
}

/// The element index a `remote` call will act on, when it acts on an element
/// by index. A click that gives both coordinates is a coordinate click on the
/// server, which then ignores the index.
fn targeted_index(remote: &str, arguments: &Value) -> Option<u64> {
    match remote {
        "browser_click" => {
            let given = |name: &str| arguments.get(name).is_some_and(|value| !value.is_null());
            if given("coordinate_x") && given("coordinate_y") {
                return None;
            }
            arguments.get("index")?.as_u64()
        }
        "browser_type" => arguments.get("index")?.as_u64(),
        _ => None,
    }
}

/// The error text for a `remote` call aimed at a select the model was shown,
/// or `None` when the call is not one.
pub(crate) fn refusal(
    shown: &ShownSelects,
    remote: &str,
    arguments: &Value,
    has_llm_key: bool,
) -> Option<String> {
    let index = targeted_index(remote, arguments).filter(|index| shown.contains(*index))?;
    let agent = if has_llm_key {
        "browser_use_agent: give it the whole task, including the URL and the option to \
         choose; it works in its own temporary browser, so nothing it does carries over to \
         this page. "
    } else {
        ""
    };
    Some(format!(
        "Refused: element {index} is a native <select> dropdown, and the pinned browser-use \
         cannot operate those. Its click is ignored while it still reports success, and \
         typing first clears the select's choice and only sometimes picks an option; the page \
         state never shows which option is selected. Nothing was sent to the browser. Routes \
         that work: {agent}jev_browse or chrome_select, if those tools are available. Both \
         drive a different browser than browser_use's own, so open the page there."
    ))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::state_view::compact_result;

    /// A raw `browser_get_state` result of the pinned shape.
    fn state_result(elements: Vec<Value>) -> Value {
        let state = serde_json::to_string_pretty(&json!({
            "url": "https://example.com/",
            "title": "Form",
            "tabs": [],
            "interactive_elements": elements,
        }))
        .unwrap();
        json!({"content": [{"type": "text", "text": state}], "isError": false})
    }

    fn element(index: u64, tag: &str) -> Value {
        json!({"index": index, "tag": tag, "text": "x"})
    }

    #[test]
    fn a_state_yields_every_select_it_lists_not_only_the_ones_in_view() {
        let mut elements: Vec<Value> = (0..400)
            .map(|n| element(2 * n + 1, if n % 100 == 7 { "select" } else { "div" }))
            .collect();
        elements.push(element(2000, "SELECT"));
        elements.push(element(2002, "option"));
        elements.push(element(2004, "selector"));
        let mut result = state_result(elements);
        let selects = compact_result(&mut result, 0, &[]).expect("a state of the pinned shape");
        let mut found: Vec<u64> = selects.into_iter().collect();
        found.sort();
        assert_eq!(found, [15, 215, 415, 615, 2000]);
        // The view cuts the list long before index 615.
        let view = result["content"][0]["text"].as_str().unwrap();
        assert!(!view.contains("[615] "), "{view}");
    }

    #[test]
    fn what_is_not_a_readable_state_has_no_selects() {
        let mut plain = json!({"content": [{"type": "text", "text": "loading"}]});
        assert_eq!(compact_result(&mut plain, 0, &[]), None);
        let mut failed = state_result(vec![element(7, "select")]);
        failed["isError"] = json!(true);
        assert_eq!(compact_result(&mut failed, 0, &[]), None);
        let mut other = json!({"content": [{"type": "text", "text": "{\"url\": \"x\"}"}]});
        assert_eq!(compact_result(&mut other, 0, &[]), None);
        assert_eq!(compact_result(&mut json!({}), 0, &[]), None);
        let mut none = state_result(vec![element(3, "a")]);
        assert_eq!(compact_result(&mut none, 0, &[]), Some(Selects::new()));
    }

    fn shown(indexes: &[u64]) -> ShownSelects {
        let shown = ShownSelects::default();
        shown.replace(Some(indexes.iter().copied().collect()));
        shown
    }

    #[test]
    fn a_click_or_type_on_a_shown_select_is_refused() {
        let shown = shown(&[7, 40]);
        for (remote, arguments) in [
            ("browser_click", json!({"index": 7})),
            ("browser_click", json!({"index": 40, "new_tab": true})),
            ("browser_type", json!({"index": 7, "text": "Banana"})),
            ("browser_type", json!({"index": 40, "text": ""})),
            (
                "browser_click",
                json!({"index": 7, "coordinate_x": null, "coordinate_y": 5}),
            ),
        ] {
            let text = refusal(&shown, remote, &arguments, false)
                .unwrap_or_else(|| panic!("{remote} {arguments} was not refused"));
            assert!(text.starts_with("Refused: element "), "{text}");
        }
    }

    #[test]
    fn nothing_else_is_refused() {
        let shown = shown(&[7]);
        for (remote, arguments) in [
            // Another element, or no index at all.
            ("browser_click", json!({"index": 8})),
            ("browser_click", json!({})),
            ("browser_type", json!({"index": 8, "text": "x"})),
            // The server clicks the coordinates and ignores the index.
            (
                "browser_click",
                json!({"index": 7, "coordinate_x": 1, "coordinate_y": 2}),
            ),
            // Not an integer index.
            ("browser_click", json!({"index": "7"})),
            ("browser_click", json!({"index": 7.5})),
            ("browser_click", json!({"index": -7})),
            ("browser_click", json!({"index": null})),
            // Tools that do not act on an element by index.
            ("browser_get_state", json!({"index": 7})),
            ("browser_extract_content", json!({"index": 7})),
            ("browser_navigate", json!({"url": "https://example.com"})),
            ("retry_with_browser_use_agent", json!({"task": "x"})),
        ] {
            assert_eq!(
                refusal(&shown, remote, &arguments, true),
                None,
                "{remote} {arguments}"
            );
        }
        assert_eq!(
            refusal(&shown, "browser_click", &json!("7"), true),
            None,
            "arguments that are not an object"
        );
    }

    #[test]
    fn nothing_is_refused_before_a_state_was_shown() {
        let shown = ShownSelects::default();
        assert_eq!(
            refusal(&shown, "browser_click", &json!({"index": 7}), true),
            None
        );
    }

    #[test]
    fn each_state_replaces_the_last_and_an_unreadable_one_forgets_it() {
        let shown = shown(&[7]);
        shown.replace(Some([9].into_iter().collect()));
        assert!(refusal(&shown, "browser_click", &json!({"index": 7}), false).is_none());
        assert!(refusal(&shown, "browser_click", &json!({"index": 9}), false).is_some());
        shown.replace(None);
        assert!(refusal(&shown, "browser_click", &json!({"index": 9}), false).is_none());
    }

    #[test]
    fn only_a_management_call_that_did_not_fail_forgets_the_selects() {
        use BrowserUseActionClass::*;

        let mut cases = Vec::new();
        for class in [Read, Navigate, Act, Agent] {
            cases.push((class, json!({"isError": false}), true));
        }
        cases.push((Manage, json!({"isError": true}), true));
        cases.push((Manage, json!({"isError": false}), false));
        cases.push((Manage, json!({"content": []}), false));
        for (class, result, kept) in cases {
            let shown = shown(&[7]);
            forget_after_close(&shown, class, &result);
            assert_eq!(
                refusal(&shown, "browser_click", &json!({"index": 7}), false).is_some(),
                kept,
                "{class:?} {result}"
            );
        }
    }

    #[test]
    fn the_text_names_the_routes_that_work_and_what_each_one_is() {
        let without_key = refusal(&shown(&[7]), "browser_click", &json!({"index": 7}), false);
        let without_key = without_key.unwrap();
        for expected in [
            "element 7 is a native <select>",
            "reports success",
            "clears the select's choice",
            "never shows which option is selected",
            "Nothing was sent to the browser",
            "jev_browse",
            "chrome_select",
            "different browser than browser_use's own",
        ] {
            assert!(without_key.contains(expected), "{expected}: {without_key}");
        }
        assert!(
            !without_key.contains("browser_use_agent"),
            "no key, no agent: {without_key}"
        );

        let with_key = refusal(&shown(&[7]), "browser_click", &json!({"index": 7}), true);
        let with_key = with_key.unwrap();
        assert!(with_key.contains("browser_use_agent"), "{with_key}");
        assert!(with_key.contains("its own temporary browser"), "{with_key}");
        assert!(with_key.contains("jev_browse"));
        assert!(with_key.contains("chrome_select"));
    }
}
