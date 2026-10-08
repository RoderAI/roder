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
