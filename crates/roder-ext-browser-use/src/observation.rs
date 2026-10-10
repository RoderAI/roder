//! How an action's report and the page observation taken after it are put
//! together for the model.
//!
//! The report comes first, labelled as browser-use's own claim, and the
//! observation follows it. The observation is the long part, so a size cut
//! takes from its tail and the report always survives, and the model reads
//! the claim before the evidence it must check the claim against.

use serde_json::{Value, json};

use crate::policy::BrowserUseActionClass;

/// Heads the action report in a result that carries an observation.
pub(crate) const REPORT_LABEL: &str = "Action report (browser-use's own claim, not checked by \
     Roder; verify it against the observation below):";

/// Heads the page state taken from the same browser right after the action.
pub(crate) const OBSERVATION_LABEL: &str = "Observed page after the action (untrusted):";

/// Heads the autonomous agent's report, which has no observation to check.
pub(crate) const AGENT_REPORT_LABEL: &str = "Agent report (browser-use's autonomous agent's own \
     claim). The agent used its own temporary browser, which is now closed, so there is no page \
     state to check it against and browser_use_get_state shows a different browser. Verify the \
     outcome yourself before relying on it:";

/// Whether a tool's result is followed by a `browser_get_state` of the same
/// browser.
///
/// The autonomous agent is not: the pinned server runs it in a browser
/// session of its own and closes that session when the run ends, so a state
/// taken afterwards describes the direct-control browser (blank when nothing
/// else has used it), not the page the agent worked on.
pub(crate) fn observes(class: BrowserUseActionClass) -> bool {
    matches!(
        class,
        BrowserUseActionClass::Navigate | BrowserUseActionClass::Act
    )
}

/// Rewrites `result` as report first, then the observation `state`. An
/// observation that is itself an error makes the whole result one. A result or
/// a state without a `content` list cannot be put together, and `result` is
/// left as it was.
pub(crate) fn attach(result: &mut Value, state: Value) -> anyhow::Result<()> {
    let report = result["content"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("browser-use returned no content"))?
        .clone();
    let observed = state["content"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("browser-use returned no observation content"))?;
    let mut content = Vec::with_capacity(report.len() + observed.len() + 2);
    content.push(text_item(REPORT_LABEL));
    content.extend(report);
    content.push(text_item(OBSERVATION_LABEL));
    content.extend(observed.iter().cloned());
    result["content"] = Value::Array(content);
    if state["isError"] == true {
        result["isError"] = json!(true);
    }
    Ok(())
}

pub(crate) fn text_item(text: &str) -> Value {
    json!({ "type": "text", "text": text })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::tool_defs;

    fn texts(result: &Value) -> Vec<&str> {
        result["content"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["text"].as_str().unwrap_or("<non-text>"))
            .collect()
    }

    #[test]
    fn only_navigation_and_actions_are_observed() {
        use BrowserUseActionClass::*;
        assert!(observes(Navigate));
        assert!(observes(Act));
        for class in [Read, Manage, Agent] {
            assert!(!observes(class), "{class:?}");
        }
        for def in tool_defs() {
            if def.name == crate::catalog::AGENT_TOOL {
                assert!(!observes(def.class), "the agent's browser is not ours");
            }
        }
    }

    #[test]
    fn the_report_comes_first_and_the_observation_follows_with_its_screenshot() {
        let mut result = json!({
            "content": [{"type": "text", "text": "Clicked element 4"}],
            "isError": false
        });
        let state = json!({
            "content": [
                {"type": "text", "text": "{\"url\":\"https://example.com/\"}"},
                {"type": "image", "data": "YWJj", "mimeType": "image/png"}
            ]
        });
        attach(&mut result, state).unwrap();
        assert_eq!(
            texts(&result),
            [
                REPORT_LABEL,
                "Clicked element 4",
                OBSERVATION_LABEL,
                "{\"url\":\"https://example.com/\"}",
                "<non-text>"
            ]
        );
        assert_eq!(result["content"][4]["type"], "image");
        assert_eq!(result["isError"], false);
        assert!(REPORT_LABEL.contains("claim"));
    }

    #[test]
    fn a_failed_observation_fails_the_result() {
        let mut result = json!({"content": [{"type": "text", "text": "Clicked element 4"}]});
        let state = json!({"content": [{"type": "text", "text": "boom"}], "isError": true});
        attach(&mut result, state).unwrap();
        assert_eq!(result["isError"], true);
    }

    #[test]
    fn a_failed_action_stays_failed_next_to_a_good_observation() {
        let mut result = json!({
            "content": [{"type": "text", "text": "Element with index 7 not found"}],
            "isError": true
        });
        let state = json!({"content": [{"type": "text", "text": "{}"}], "isError": false});
        attach(&mut result, state).unwrap();
        assert_eq!(result["isError"], true);
        assert_eq!(texts(&result)[1], "Element with index 7 not found");
    }

    #[test]
    fn a_state_without_content_cannot_be_attached() {
        for state in [
            json!({"isError": false}),
            json!({"content": "text"}),
            json!({"content": null}),
            json!({"content": {"type": "text", "text": "{}"}}),
        ] {
            let mut result = json!({
                "content": [{"type": "text", "text": "Clicked element 4"}],
                "isError": false
            });
            let before = result.clone();
            let error = attach(&mut result, state.clone()).unwrap_err();
            assert!(
                error.to_string().contains("no observation content"),
                "{state}: {error}"
            );
            assert_eq!(result, before, "{state}: the action result was rewritten");
        }
    }

    #[test]
    fn a_state_with_empty_content_is_an_empty_observation() {
        let mut result = json!({"content": [{"type": "text", "text": "Clicked element 4"}]});
        attach(&mut result, json!({"content": []})).unwrap();
        assert_eq!(
            texts(&result),
            [REPORT_LABEL, "Clicked element 4", OBSERVATION_LABEL]
        );
    }

    #[test]
    fn a_result_without_content_cannot_be_observed() {
        let mut result = json!({"isError": false});
        let error = attach(&mut result, json!({"content": []})).unwrap_err();
        assert!(error.to_string().contains("no content"));
    }
}
