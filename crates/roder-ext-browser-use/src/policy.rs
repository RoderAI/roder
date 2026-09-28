//! Roder policy for the `browser_use_*` tools.
//!
//! Reads, navigation and housekeeping of the provider's own browser run
//! without a prompt, in every mode. Clicking and typing act on a page, so
//! default mode asks, accept-all mode allows, and plan mode denies them. The
//! autonomous agent tool can do anything a person could in that browser, so
//! it is asked about in default and accept-all mode alike and denied in plan
//! mode. Bypass mode, which skips every check by the user's choice, allows it.

use roder_api::context::{PolicyContribution, PolicyContributor, PolicyReview};
use roder_api::policy_mode::PolicyMode;
use serde_json::Value;

use crate::catalog::{AGENT_TOOL, tool_def};

/// How a browser-use tool is treated by Roder policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserUseActionClass {
    /// Reads page state, HTML, screenshots, tab and session lists.
    Read,
    /// Moves around without acting on a page: navigate, back, scroll, switch tab.
    Navigate,
    /// Closes the provider's own tabs and browser sessions.
    Manage,
    /// Acts on a page: click and type.
    Act,
    /// Hands a whole task to browser-use's own autonomous agent.
    Agent,
}

/// The class of a model-facing `browser_use_*` tool, or `None` for any
/// other tool.
pub fn classify_tool(name: &str) -> Option<BrowserUseActionClass> {
    tool_def(name).map(|def| def.class)
}

pub(crate) struct BrowserUsePolicy;

#[async_trait::async_trait]
impl PolicyContributor for BrowserUsePolicy {
    fn id(&self) -> String {
        "browser-use".into()
    }

    async fn review_tool(&self, review: PolicyReview) -> anyhow::Result<PolicyContribution> {
        let Some(class) = classify_tool(&review.call.name) else {
            return Ok(PolicyContribution::Abstain);
        };
        Ok(decide(class, review.mode, &review.call.arguments))
    }
}

pub(crate) fn decide(
    class: BrowserUseActionClass,
    mode: PolicyMode,
    args: &Value,
) -> PolicyContribution {
    use BrowserUseActionClass::*;
    match (class, mode) {
        (Read | Navigate | Manage, _) => PolicyContribution::Allow { reason: None },
        (Act | Agent, PolicyMode::Plan) => PolicyContribution::Deny {
            reason: "browser-use clicks, typing and agent tasks act on pages and cannot run in \
                     plan mode"
                .into(),
        },
        (_, PolicyMode::Bypass) | (Act, PolicyMode::AcceptAll) => {
            PolicyContribution::Allow { reason: None }
        }
        (Act, PolicyMode::Default) => PolicyContribution::RequireApproval {
            reason: Some(act_reason(args)),
        },
        (Agent, PolicyMode::Default | PolicyMode::AcceptAll) => {
            PolicyContribution::RequireApproval {
                reason: Some(agent_reason(args)),
            }
        }
    }
}

fn act_reason(args: &Value) -> String {
    let target = match (
        args.get("index"),
        args.get("coordinate_x"),
        args.get("coordinate_y"),
    ) {
        (Some(index), _, _) => format!("element {index}"),
        (None, Some(x), Some(y)) => format!("the point ({x}, {y})"),
        _ => "an element".into(),
    };
    match args.get("text").and_then(Value::as_str) {
        Some(text) => format!(
            "browser-use will type {} characters into {target} of the current page",
            text.chars().count()
        ),
        None => format!("browser-use will click {target} of the current page"),
    }
}

fn agent_reason(args: &Value) -> String {
    let task = args
        .get("task")
        .and_then(Value::as_str)
        .unwrap_or("(no task given)");
    let task: String = task.chars().take(300).collect();
    let domains = match args.get("allowed_domains").and_then(Value::as_array) {
        Some(domains) if !domains.is_empty() => format!(
            "it may only visit {}",
            domains
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        _ => "it may visit any site".into(),
    };
    format!(
        "{AGENT_TOOL} hands this task to browser-use's autonomous agent, which may navigate, \
         click, type and submit forms on its own; {domains}. Task: {task}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::tool_defs;
    use BrowserUseActionClass::*;
    use roder_api::tools::{ToolCall, ToolExecutionContext};
    use serde_json::json;

    #[test]
    fn every_tool_has_the_expected_class() {
        let expected = [
            ("browser_use_navigate", Navigate),
            ("browser_use_go_back", Navigate),
            ("browser_use_scroll", Navigate),
            ("browser_use_switch_tab", Navigate),
            ("browser_use_get_state", Read),
            ("browser_use_extract_content", Read),
            ("browser_use_get_html", Read),
            ("browser_use_screenshot", Read),
            ("browser_use_list_tabs", Read),
            ("browser_use_list_sessions", Read),
            ("browser_use_close_tab", Manage),
            ("browser_use_close_session", Manage),
            ("browser_use_close_all", Manage),
            ("browser_use_click", Act),
            ("browser_use_type", Act),
            ("browser_use_agent", Agent),
        ];
        assert_eq!(expected.len(), tool_defs().len());
        for (name, class) in expected {
            assert_eq!(classify_tool(name), Some(class), "{name}");
        }
        assert_eq!(classify_tool("chrome_click"), None);
    }

    #[test]
    fn plan_mode_denies_acting_tools_only() {
        for class in [Act, Agent] {
            assert!(matches!(
                decide(class, PolicyMode::Plan, &json!({})),
                PolicyContribution::Deny { .. }
            ));
        }
        for class in [Read, Navigate, Manage] {
            assert_eq!(
                decide(class, PolicyMode::Plan, &json!({})),
                PolicyContribution::Allow { reason: None }
            );
        }
    }

    #[test]
    fn default_mode_asks_before_acting() {
        assert!(matches!(
            decide(Act, PolicyMode::Default, &json!({"index": 3})),
            PolicyContribution::RequireApproval { .. }
        ));
        assert_eq!(
            decide(Navigate, PolicyMode::Default, &json!({})),
            PolicyContribution::Allow { reason: None }
        );
    }

    #[test]
    fn accept_all_still_asks_about_the_autonomous_agent() {
        assert_eq!(
            decide(Act, PolicyMode::AcceptAll, &json!({})),
            PolicyContribution::Allow { reason: None }
        );
        assert!(matches!(
            decide(Agent, PolicyMode::AcceptAll, &json!({"task": "x"})),
            PolicyContribution::RequireApproval { .. }
        ));
        assert_eq!(
            decide(Agent, PolicyMode::Bypass, &json!({"task": "x"})),
            PolicyContribution::Allow { reason: None }
        );
    }

    #[test]
    fn approval_reasons_describe_the_action_without_echoing_typed_text() {
        let PolicyContribution::RequireApproval {
            reason: Some(reason),
        } = decide(
            Act,
            PolicyMode::Default,
            &json!({"index": 7, "text": "hunter2"}),
        )
        else {
            panic!("expected approval");
        };
        assert!(reason.contains("7 characters into element 7"), "{reason}");
        assert!(!reason.contains("hunter2"), "{reason}");

        let PolicyContribution::RequireApproval {
            reason: Some(reason),
        } = decide(
            Agent,
            PolicyMode::Default,
            &json!({"task": "read the heading", "allowed_domains": ["example.com"]}),
        )
        else {
            panic!("expected approval");
        };
        assert!(reason.contains("only visit example.com"), "{reason}");
        assert!(reason.contains("read the heading"), "{reason}");
    }

    #[tokio::test]
    async fn contributor_abstains_for_other_tools() {
        let review = PolicyReview {
            call: ToolCall {
                id: "c".into(),
                name: "jev_browse".into(),
                raw_arguments: "{}".into(),
                arguments: json!({}),
                thread_id: "t".into(),
                turn_id: "u".into(),
            },
            mode: PolicyMode::Default,
            context: ToolExecutionContext::new("t", "u", PolicyMode::Default),
        };
        assert_eq!(
            BrowserUsePolicy.review_tool(review).await.unwrap(),
            PolicyContribution::Abstain
        );
    }
}
