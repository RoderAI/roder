use async_trait::async_trait;
use roder_api::context::{PolicyContribution, PolicyContributor, PolicyReview};
use roder_api::policy_mode::PolicyMode;

pub(crate) struct CuaPolicy;
#[async_trait]
impl PolicyContributor for CuaPolicy {
    fn id(&self) -> String {
        "cua".into()
    }
    async fn review_tool(&self, review: PolicyReview) -> anyhow::Result<PolicyContribution> {
        let Some(name) = review.call.name.strip_prefix("cua_") else {
            return Ok(PolicyContribution::Abstain);
        };
        if !crate::specs::is_input(name) {
            return Ok(PolicyContribution::Abstain);
        }
        Ok(match review.mode {
            PolicyMode::Plan => PolicyContribution::Deny {
                reason: "desktop input is disabled in Plan mode".into(),
            },
            PolicyMode::Default => PolicyContribution::RequireApproval {
                reason: Some("operate the thread's remote desktop".into()),
            },
            PolicyMode::AcceptAll | PolicyMode::Bypass => PolicyContribution::Abstain,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::tools::{ToolCall, ToolExecutionContext};
    use serde_json::json;
    #[tokio::test]
    async fn browser_mutations_require_approval_and_plan_denies_them() {
        for tool in crate::browser_specs::TOOLS {
            for mode in [PolicyMode::Default, PolicyMode::Plan, PolicyMode::Bypass] {
                let result = CuaPolicy
                    .review_tool(PolicyReview {
                        mode,
                        context: ToolExecutionContext::new("t", "u", mode),
                        call: ToolCall {
                            id: "c".into(),
                            name: format!("cua_{tool}"),
                            arguments: json!({}),
                            raw_arguments: String::new(),
                            thread_id: "t".into(),
                            turn_id: "u".into(),
                        },
                    })
                    .await
                    .unwrap();
                if *tool == "get_browser_state" || mode == PolicyMode::Bypass {
                    assert_eq!(result, PolicyContribution::Abstain);
                } else if mode == PolicyMode::Plan {
                    assert!(matches!(result, PolicyContribution::Deny { .. }));
                } else {
                    assert!(matches!(result, PolicyContribution::RequireApproval { .. }));
                }
            }
        }
    }
}
