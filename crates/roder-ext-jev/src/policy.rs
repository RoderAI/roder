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
    format!("Jev may navigate, click and type {place}{scope}{commits}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scope::JevOriginScope;
    use roder_api::tools::ToolCall;
    use serde_json::json;

    fn scoped(scope: JevOriginScope) -> anyhow::Result<Ceilings> {
        Ok(Ceilings {
            scope,
            ..Ceilings::default()
        })
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
            ..Ceilings::default()
        };
        let reason = approval_reason(&json!({"url": "https://shop.example.com"}), Ok(gated), None);
        assert!(
            reason.ends_with("it stops before anything that cannot be undone"),
            "{reason}"
        );
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
