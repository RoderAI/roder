use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::catalog::PROVIDER_CLAUDE_CODE;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;

use crate::provider::{ClaudeCodeConfig, ClaudeCodeEngine};

pub struct ClaudeCodeExtension {
    config: ClaudeCodeConfig,
}

impl ClaudeCodeExtension {
    pub fn new(config: ClaudeCodeConfig) -> Self {
        Self { config }
    }
}

impl RoderExtension for ClaudeCodeExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-claude-code-provider".to_string(),
            name: "Claude Code Provider".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some(
                "Claude Code CLI harness provider backed by claude-agent-sdk".to_string(),
            ),
            provides: vec![ProvidedService::InferenceEngine(
                PROVIDER_CLAUDE_CODE.to_string(),
            )],
            required_capabilities: vec![CapabilityRequest::new("process.claude")],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.inference_engine(Arc::new(ClaudeCodeEngine::new(self.config.clone())));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use roder_api::extension::RoderExtension;

    use super::*;

    #[test]
    fn manifest_provides_claude_code_engine() {
        let manifest = ClaudeCodeExtension::new(ClaudeCodeConfig::default()).manifest();
        assert_eq!(
            manifest.provides,
            vec![ProvidedService::InferenceEngine(
                PROVIDER_CLAUDE_CODE.to_string()
            )]
        );
    }
}
