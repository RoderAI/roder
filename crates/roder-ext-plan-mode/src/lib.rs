mod exit_plan_tool;

use std::sync::Arc;

use roder_api::context::{PolicyContribution, PolicyContributor, PolicyReview};
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use roder_api::policy_mode::PolicyMode;
use semver::Version;

pub use exit_plan_tool::{ExitPlanModeTool, ExitPlanModeToolContributor};

pub struct PlanModeExtension {
    _active_mode: PolicyMode,
}

impl PlanModeExtension {
    pub fn new(active_mode: PolicyMode) -> Self {
        Self {
            _active_mode: active_mode,
        }
    }
}

impl RoderExtension for PlanModeExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-plan-mode".to_string(),
            name: "Plan mode policy".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("Policy contributor for plan mode".to_string()),
            provides: vec![ProvidedService::PolicyContributor("plan-mode".to_string())],
            required_capabilities: vec![],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.policy_contributor(Arc::new(PlanModePolicyContributor));
        Ok(())
    }
}

struct PlanModePolicyContributor;

#[async_trait::async_trait]
impl PolicyContributor for PlanModePolicyContributor {
    fn id(&self) -> roder_api::extension::PolicyContributorId {
        "plan-mode".to_string()
    }

    async fn review_tool(&self, _review: PolicyReview) -> anyhow::Result<PolicyContribution> {
        Ok(PolicyContribution::Abstain)
    }
}

pub fn extension(active_mode: PolicyMode) -> PlanModeExtension {
    PlanModeExtension::new(active_mode)
}
