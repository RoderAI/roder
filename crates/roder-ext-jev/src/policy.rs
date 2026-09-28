use roder_api::context::{PolicyContribution, PolicyContributor, PolicyReview};
use roder_api::policy_mode::PolicyMode;
use serde_json::Value;

use crate::runner::Ceilings;

/// Roder's policy for `jev_browse`: plan mode denies it, default mode asks
/// for every goal, and accept-all mode lets it run, except a call that sets
/// `authorize_irreversible`, which is always asked about: no mode that skips
/// approval may authorize a purchase, payment, send or deletion unseen.
/// Bypass mode, which skips every check by the user's choice, allows it.
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
        Ok(match review.mode {
            PolicyMode::Plan => PolicyContribution::Deny {
                reason: "Jev browser tasks interact with pages and cannot run in plan mode".into(),
            },
            PolicyMode::AcceptAll if !authorizes_irreversible(args) => {
                PolicyContribution::Allow { reason: None }
            }
            PolicyMode::Default | PolicyMode::AcceptAll => PolicyContribution::RequireApproval {
                reason: Some(approval_reason(args, Ceilings::from_env())),
            },
            PolicyMode::Bypass => PolicyContribution::Allow { reason: None },
        })
    }
}

/// Whether the call authorizes actions that cannot be undone.
fn authorizes_irreversible(args: &Value) -> bool {
    args["authorize_irreversible"] == Value::Bool(true)
}

/// What the person approving is asked about: the site the task starts on,
/// the origins it may reach from there, and what it may commit.
fn approval_reason(args: &Value, operator: anyhow::Result<Ceilings>) -> String {
    let gate = operator
        .as_ref()
        .is_ok_and(|ceilings| ceilings.confirm_irreversible);
    let operator = operator.map(|ceilings| ceilings.scope);
    let host = args["url"]
        .as_str()
        .and_then(|url| reqwest::Url::parse(url.trim()).ok())
        .and_then(|url| url.host_str().map(str::to_string))
        .unwrap_or_else(|| "an unparsed URL".into());
    let scope = operator.and_then(|scope| match args["allowed_origins"].as_array() {
        Some(origins) => {
            let origins = origins.iter().filter_map(Value::as_str).collect::<Vec<_>>();
            scope.narrow(&origins)
        }
        None => Ok(scope),
    });
    let scope = match scope {
        Ok(scope) if scope.is_restricted() => format!("it may only visit {scope}"),
        Ok(_) => "it may follow links to any site".into(),
        Err(error) => format!("its allowed origins are invalid ({error:#}), so it will not start"),
    };
    let commits = match (authorizes_irreversible(args), gate) {
        (true, _) => {
            "; it is AUTHORIZED to make purchases, payments, sends, deletions and other \
             changes that cannot be undone (authorize_irreversible)"
        }
        (false, true) => "; it stops before anything that cannot be undone",
        (false, false) => "",
    };
    format!(
        "Jev may navigate, click, and type in the browser, starting on {host}; {scope}{commits}"
    )
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

    #[test]
    fn the_approval_names_the_host_and_the_scope() {
        let any = || scoped(JevOriginScope::any());
        assert_eq!(
            approval_reason(&json!({"url": "https://shop.example.com/cart"}), any()),
            "Jev may navigate, click, and type in the browser, starting on shop.example.com; \
             it may follow links to any site"
        );
        let operator = || {
            scoped(
                JevOriginScope::any()
                    .narrow(&["https://*.example.com"])
                    .unwrap(),
            )
        };
        assert_eq!(
            approval_reason(
                &json!({"url": "https://shop.example.com",
                    "allowed_origins": ["https://shop.example.com"]}),
                operator()
            ),
            "Jev may navigate, click, and type in the browser, starting on shop.example.com; \
             it may only visit https://*.example.com and within https://shop.example.com"
        );
        let invalid = approval_reason(
            &json!({"url": "https://a.test", "allowed_origins": ["a.test"]}),
            any(),
        );
        assert!(invalid.contains("will not start"), "{invalid}");
    }

    #[test]
    fn an_authorized_call_is_named_in_the_approval() {
        let args = json!({"url": "https://shop.example.com", "authorize_irreversible": true});
        let reason = approval_reason(&args, scoped(JevOriginScope::any()));
        assert!(reason.ends_with("(authorize_irreversible)"), "{reason}");
        assert!(
            reason.contains("AUTHORIZED to make purchases, payments"),
            "{reason}"
        );
        let gated = Ceilings {
            confirm_irreversible: true,
            ..Ceilings::default()
        };
        let reason = approval_reason(&json!({"url": "https://shop.example.com"}), Ok(gated));
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
        // `false` is no authorization.
        let declined = json!({"url": "https://a.test", "goal": "Pay",
            "authorize_irreversible": false});
        assert!(matches!(
            review(PolicyMode::AcceptAll, declined).await,
            PolicyContribution::Allow { .. }
        ));
    }
}
