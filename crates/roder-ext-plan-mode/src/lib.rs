mod exit_plan_tool;

use std::sync::Arc;

use roder_api::context::PolicyContributor;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use roder_api::policy_mode::PolicyMode;
use semver::Version;

pub use exit_plan_tool::{ExitPlanModeTool, ExitPlanModeToolContributor};

pub struct PlanModeExtension {
    active_mode: PolicyMode,
}

impl PlanModeExtension {
    pub fn new(active_mode: PolicyMode) -> Self {
        Self { active_mode }
    }
}

impl RoderExtension for PlanModeExtension {
    fn manifest(&self) -> ExtensionManifest {
        let mut provides = vec![ProvidedService::PolicyContributor("plan-mode".to_string())];
        if self.active_mode == PolicyMode::Plan {
            provides.push(ProvidedService::ToolProvider("plan-mode-tools".to_string()));
        }
        ExtensionManifest {
            id: "roder-ext-plan-mode".to_string(),
            name: "Plan mode policy".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("Policy contributor and exit tool for plan mode".to_string()),
            provides,
            required_capabilities: vec![],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.policy_contributor(Arc::new(PlanModePolicyContributor));
        if self.active_mode == PolicyMode::Plan {
            registry.tool_contributor(Arc::new(ExitPlanModeToolContributor));
        }
        Ok(())
    }
}

struct PlanModePolicyContributor;

impl PolicyContributor for PlanModePolicyContributor {}

pub fn extension(active_mode: PolicyMode) -> PlanModeExtension {
    PlanModeExtension::new(active_mode)
}
