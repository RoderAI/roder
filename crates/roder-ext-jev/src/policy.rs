use roder_api::context::{PolicyContribution, PolicyContributor, PolicyReview};
use roder_api::policy_mode::PolicyMode;

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
        Ok(match review.mode {
            PolicyMode::Plan => PolicyContribution::Deny {
                reason: "Jev browser tasks interact with pages and cannot run in plan mode".into(),
            },
            PolicyMode::Default => PolicyContribution::RequireApproval {
                reason: Some("Jev may navigate, click, and type in the browser".into()),
            },
            PolicyMode::AcceptAll | PolicyMode::Bypass => {
                PolicyContribution::Allow { reason: None }
            }
        })
    }
}
