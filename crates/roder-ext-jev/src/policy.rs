use roder_api::context::{PolicyContribution, PolicyContributor, PolicyReview};
use roder_api::policy_mode::PolicyMode;
use serde_json::Value;

use crate::runner::Ceilings;
use crate::session::{JevSessions, SessionSummary};

/// Roder's policy for `jev_browse`: plan mode denies it, default mode asks
/// for every goal, and accept-all mode lets it run, except a call that sets
/// `authorize_irreversible`, which is always asked about: no mode that skips
/// approval may authorize a purchase, payment, send or deletion unseen.
/// Bypass mode, which skips every check by the user's choice, allows it.
/// A call with `tab: "close"` only closes Jev's own tabs, so every mode
/// allows it.
pub(crate) struct JevPolicy;

#[async_trait::async_trait]
impl PolicyContributor for JevPolicy {
    fn id(&self) -> String {
        "jev".into()
    }

    async fn review_tool(&self, review: PolicyReview) -> anyhow::Result<PolicyContribution> {
        if let Some(short) = review.call.name.strip_prefix("jev_tab_") {
            return Ok(review_tab_tool(short, &review));
        }
        if review.call.name != "jev_browse" {
            return Ok(PolicyContribution::Abstain);
        }
        let args = &review.call.arguments;
        if tab(args) == "close" {
            return Ok(PolicyContribution::Allow { reason: None });
        }
        Ok(match review.mode {
            PolicyMode::Plan => PolicyContribution::Deny {
                reason: "Jev browser tasks interact with pages and cannot run in plan mode".into(),
            },
            PolicyMode::AcceptAll if !authorizes_irreversible(args) => {
                PolicyContribution::Allow { reason: None }
            }
            PolicyMode::Default | PolicyMode::AcceptAll => PolicyContribution::RequireApproval {
                reason: Some(approval_reason(
                    args,
                    Ceilings::from_env(),
                    JevSessions::global().peek(&review.context.thread_id),
                )),
            },
            PolicyMode::Bypass => PolicyContribution::Allow { reason: None },
        })
    }
}

/// The hand-over tools on the thread's Jev tab. Reading it (look,
/// screenshot, wait) is allowed in every mode. Acting on it follows
/// `jev_browse`: plan mode denies it, default mode asks for each action,
/// accept-all lets it run unless the call sets `authorize_irreversible`,
/// and bypass allows it.
fn review_tab_tool(short: &str, review: &PolicyReview) -> PolicyContribution {
    if roder_ext_chrome::direct::reads_only(short) {
        return PolicyContribution::Allow { reason: None };
    }
    let args = &review.call.arguments;
    match review.mode {
        PolicyMode::Plan => PolicyContribution::Deny {
            reason: "the jev_tab_* tools act on a page and cannot run in plan mode".into(),
        },
        PolicyMode::AcceptAll if !authorizes_irreversible(args) => {
            PolicyContribution::Allow { reason: None }
        }
        PolicyMode::Default | PolicyMode::AcceptAll => PolicyContribution::RequireApproval {
            reason: Some(tab_tool_reason(
                short,
                args,
                JevSessions::global().peek(&review.context.thread_id),
            )),
        },
        PolicyMode::Bypass => PolicyContribution::Allow { reason: None },
    }
}

/// What the person approving a hand-over tool call is asked about.
fn tab_tool_reason(short: &str, args: &Value, session: Option<SessionSummary>) -> String {
    let place = session
        .and_then(|session| session.current_url)
        .and_then(|url| host(&url))
        .map_or("this thread's Jev browser tab".to_string(), |host| {
            format!("this thread's Jev browser tab, on {host}")
        });
    let what = match short {
        "navigate" => format!(
            "load {} in",
            args["url"]
                .as_str()
                .map(|url| host(url).unwrap_or_else(|| url.to_string()))
                .unwrap_or_else(|| "a page".into())
        ),
        "type" => "type into".into(),
        "key" => format!("press {} in", args["key"].as_str().unwrap_or("a key")),
        other => format!("{other} in"),
    };
    let mut reason = format!("Roder may {what} {place}");
    if authorizes_irreversible(args) {
        reason.push_str(
            "; it is AUTHORIZED to press a control that may make a purchase, payment, send, \
             deletion or other change that cannot be undone (authorize_irreversible)",
        );
    }
    reason
}

/// Whether the call authorizes actions that cannot be undone.
fn authorizes_irreversible(args: &Value) -> bool {
    args["authorize_irreversible"] == Value::Bool(true)
}

/// The call's `tab`, `current` when it gives none.
fn tab(args: &Value) -> String {
    args["tab"]
        .as_str()
        .map(|tab| tab.trim().to_ascii_lowercase())
        .filter(|tab| !tab.is_empty())
        .unwrap_or_else(|| "current".into())
}

fn host(url: &str) -> Option<String> {
    reqwest::Url::parse(url.trim())
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
}

/// What the person approving is asked about: which tab Jev works in and
/// where, the operator's origin limit when there is one, and what it may
/// commit.
fn approval_reason(
    args: &Value,
    operator: anyhow::Result<Ceilings>,
    session: Option<SessionSummary>,
) -> String {
    let url = args["url"].as_str().filter(|url| !url.trim().is_empty());
    let target = url.map(|url| host(url).unwrap_or_else(|| "an unparsed URL".into()));
    let current = session
        .as_ref()
        .filter(|session| !session.targets.is_empty())
        .and_then(|session| session.current_url.as_deref())
        .and_then(host);
    let place = match (tab(args).as_str(), target) {
        ("new", Some(target)) => {
            format!("in a new tab of this thread's browser session, opening {target}")
        }
        ("reset", Some(target)) => {
            format!("in this thread's browser tab, closing its tabs and starting over on {target}")
        }
        (_, Some(target)) => format!("in this thread's browser tab, loading {target}"),
        (_, None) => match current {
            Some(current) => format!("in this thread's browser tab, continuing on {current}"),
            None => "in this thread's browser tab, continuing where it is".into(),
        },
    };
    let gate = operator
        .as_ref()
        .is_ok_and(|ceilings| ceilings.confirm_irreversible);
    let operator_fallback = operator
        .as_ref()
        .is_ok_and(|ceilings| ceilings.fallback.mode == crate::fallback::FallbackMode::Auto);
    let scope = match operator {
        Ok(ceilings) if ceilings.scope.is_restricted() => {
            format!("; it may only visit {}", ceilings.scope)
        }
        Ok(_) => String::new(),
        Err(error) => {
            format!("; the operator's limits are invalid ({error:#}), so it will not start")
        }
    };
    let commits = match (authorizes_irreversible(args), gate) {
        (true, _) => {
            "; it is AUTHORIZED to make purchases, payments, sends, deletions and other \
             changes that cannot be undone (authorize_irreversible)"
        }
        (false, true) => "; it stops before anything that cannot be undone",
        (false, false) => "",
    };
    let fallback = match operator_fallback {
        true => {
            "; if Jev cannot progress, the session's model may go on in the same tab with \
             Roder's full browser tools"
        }
        false => "",
    };
    format!("Jev may navigate, click and type {place}{scope}{commits}{fallback}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scope::JevOriginScope;
    use roder_api::tools::ToolCall;
    use serde_json::json;

    /// The operator's settings with the fallback off, which the approval
    /// then does not mention.
    fn quiet() -> Ceilings {
        Ceilings {
            fallback: crate::fallback::FallbackSettings {
                mode: crate::fallback::FallbackMode::Off,
                ..Default::default()
            },
            ..Ceilings::default()
        }
    }

    fn scoped(scope: JevOriginScope) -> anyhow::Result<Ceilings> {
        Ok(Ceilings { scope, ..quiet() })
    }

    fn summary(url: &str) -> SessionSummary {
        SessionSummary {
            current_url: Some(url.into()),
            targets: vec!["T1".into()],
            endpoint: None,
            last_used: std::time::Instant::now(),
        }
    }

    #[test]
    fn approval_names_host_without_origin_list() {
        let any = || scoped(JevOriginScope::any());
        assert_eq!(
            approval_reason(
                &json!({"url": "https://shop.example.com/cart"}),
                any(),
                None
            ),
            "Jev may navigate, click and type in this thread's browser tab, loading \
             shop.example.com"
        );
        // An old caller's list is not read: the call names no origins.
        let listed = approval_reason(
            &json!({"url": "https://a.test", "allowed_origins": ["a.test"]}),
            any(),
            None,
        );
        assert!(
            !listed.contains("origin") && !listed.contains("visit"),
            "{listed}"
        );
        let operator = scoped(
            JevOriginScope::any()
                .narrow(&["https://*.example.com"])
                .unwrap(),
        );
        assert_eq!(
            approval_reason(&json!({"url": "https://shop.example.com"}), operator, None),
            "Jev may navigate, click and type in this thread's browser tab, loading \
             shop.example.com; it may only visit https://*.example.com"
        );
        let invalid = approval_reason(
            &json!({"url": "https://a.test"}),
            Err(anyhow::anyhow!(
                "JEV_MAX_ACTIONS must be a positive whole number"
            )),
            None,
        );
        assert!(invalid.contains("will not start"), "{invalid}");
    }

    #[test]
    fn approval_names_the_session_tab_it_works_in() {
        let any = || scoped(JevOriginScope::any());
        let resy = || Some(summary("https://resy.com/cities/sf"));
        assert_eq!(
            approval_reason(&json!({"url": "", "goal": "Pick 8 PM"}), any(), resy()),
            "Jev may navigate, click and type in this thread's browser tab, continuing on \
             resy.com"
        );
        assert_eq!(
            approval_reason(
                &json!({"url": "https://tock.com/", "tab": "new"}),
                any(),
                resy()
            ),
            "Jev may navigate, click and type in a new tab of this thread's browser session, \
             opening tock.com"
        );
        let reset = approval_reason(
            &json!({"url": "https://a.test", "tab": "reset"}),
            any(),
            resy(),
        );
        assert!(reset.ends_with("starting over on a.test"), "{reset}");
        assert!(
            approval_reason(&json!({"url": null}), any(), None).ends_with("continuing where it is")
        );
    }

    #[test]
    fn an_authorized_call_is_named_in_the_approval() {
        let args = json!({"url": "https://shop.example.com", "authorize_irreversible": true});
        let reason = approval_reason(&args, scoped(JevOriginScope::any()), None);
        assert!(reason.ends_with("(authorize_irreversible)"), "{reason}");
        assert!(
            reason.contains("AUTHORIZED to make purchases, payments"),
            "{reason}"
        );
        let gated = Ceilings {
            confirm_irreversible: true,
            ..quiet()
        };
        let reason = approval_reason(&json!({"url": "https://shop.example.com"}), Ok(gated), None);
        assert!(
            reason.ends_with("it stops before anything that cannot be undone"),
            "{reason}"
        );
    }

    #[test]
    fn the_approval_says_when_the_sessions_model_may_go_on() {
        let reason = approval_reason(
            &json!({"url": "https://shop.example.com"}),
            Ok(Ceilings::default()),
            None,
        );
        assert!(
            reason.ends_with(
                "; if Jev cannot progress, the session's model may go on in the same tab with \
                 Roder's full browser tools"
            ),
            "{reason}"
        );
    }

    async fn review_tool(mode: PolicyMode, name: &str, args: Value) -> PolicyContribution {
        let call = ToolCall {
            id: "call".into(),
            name: name.into(),
            raw_arguments: args.to_string(),
            arguments: args,
            thread_id: "thread".into(),
            turn_id: "turn".into(),
        };
        let context = roder_api::tools::ToolExecutionContext::new("thread", "turn", mode);
        JevPolicy
            .review_tool(PolicyReview {
                call,
                mode,
                context,
            })
            .await
            .unwrap()
    }

    /// Reading Jev's tab is allowed in every mode; acting on it follows
    /// jev_browse, and an authorized press is always asked about.
    #[tokio::test]
    async fn the_hand_over_tools_follow_jevs_policy() {
        for mode in [
            PolicyMode::Plan,
            PolicyMode::Default,
            PolicyMode::AcceptAll,
            PolicyMode::Bypass,
        ] {
            for read in ["jev_tab_look", "jev_tab_screenshot", "jev_tab_wait"] {
                assert!(
                    matches!(
                        review_tool(mode, read, json!({})).await,
                        PolicyContribution::Allow { .. }
                    ),
                    "{read} in {mode:?}"
                );
            }
        }
        let click = json!({"ref": "e3"});
        assert!(matches!(
            review_tool(PolicyMode::Plan, "jev_tab_click", click.clone()).await,
            PolicyContribution::Deny { .. }
        ));
        match review_tool(PolicyMode::Default, "jev_tab_click", click.clone()).await {
            PolicyContribution::RequireApproval {
                reason: Some(reason),
            } => assert!(
                reason.starts_with("Roder may click in this thread's Jev"),
                "{reason}"
            ),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            review_tool(PolicyMode::AcceptAll, "jev_tab_click", click.clone()).await,
            PolicyContribution::Allow { .. }
        ));
        let authorized = json!({"ref": "e3", "authorize_irreversible": true});
        match review_tool(PolicyMode::AcceptAll, "jev_tab_click", authorized.clone()).await {
            PolicyContribution::RequireApproval {
                reason: Some(reason),
            } => assert!(reason.contains("AUTHORIZED"), "{reason}"),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            review_tool(PolicyMode::Bypass, "jev_tab_drag", authorized).await,
            PolicyContribution::Allow { .. }
        ));
        match review_tool(
            PolicyMode::Default,
            "jev_tab_navigate",
            json!({"url": "https://tock.com/x"}),
        )
        .await
        {
            PolicyContribution::RequireApproval {
                reason: Some(reason),
            } => assert!(reason.starts_with("Roder may load tock.com in"), "{reason}"),
            other => panic!("{other:?}"),
        }
    }

    async fn review(mode: PolicyMode, args: Value) -> PolicyContribution {
        let call = ToolCall {
            id: "call".into(),
            name: "jev_browse".into(),
            raw_arguments: args.to_string(),
            arguments: args,
            thread_id: "thread".into(),
            turn_id: "turn".into(),
        };
        let context = roder_api::tools::ToolExecutionContext::new("thread", "turn", mode);
        JevPolicy
            .review_tool(PolicyReview {
                call,
                mode,
                context,
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn authorizing_irreversible_actions_needs_approval_even_in_accept_all() {
        let plain = json!({"url": "https://shop.example.com", "goal": "Read"});
        let authorized = json!({"url": "https://shop.example.com", "goal": "Pay",
            "authorize_irreversible": true});
        assert!(matches!(
            review(PolicyMode::AcceptAll, plain.clone()).await,
            PolicyContribution::Allow { .. }
        ));
        match review(PolicyMode::AcceptAll, authorized.clone()).await {
            PolicyContribution::RequireApproval {
                reason: Some(reason),
            } => {
                assert!(reason.contains("authorize_irreversible"), "{reason}")
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            review(PolicyMode::Default, authorized.clone()).await,
            PolicyContribution::RequireApproval { .. }
        ));
        assert!(matches!(
            review(PolicyMode::Plan, authorized.clone()).await,
            PolicyContribution::Deny { .. }
        ));
        assert!(matches!(
            review(PolicyMode::Bypass, authorized).await,
            PolicyContribution::Allow { .. }
        ));
        // Closing only closes Jev's own tabs: allowed even in plan mode.
        for mode in [PolicyMode::Plan, PolicyMode::Default, PolicyMode::AcceptAll] {
            assert!(matches!(
                review(mode, json!({"goal": "", "url": "", "tab": "close"})).await,
                PolicyContribution::Allow { .. }
            ));
        }
        // `false` is no authorization.
        let declined = json!({"url": "https://a.test", "goal": "Pay",
            "authorize_irreversible": false});
        assert!(matches!(
            review(PolicyMode::AcceptAll, declined).await,
            PolicyContribution::Allow { .. }
        ));
    }
}
