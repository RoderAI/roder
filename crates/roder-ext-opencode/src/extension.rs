use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::catalog::{PROVIDER_OPENCODE, PROVIDER_OPENCODE_GO};
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;

use crate::provider::{OpenCodeConfig, OpenCodeInferenceEngine, OpenCodeProviderSpec};

pub struct OpenCodeExtension {
    zen_config: OpenCodeConfig,
    go_config: OpenCodeConfig,
}

impl OpenCodeExtension {
    pub fn new(config: OpenCodeConfig) -> Self {
        Self {
            zen_config: config,
            go_config: OpenCodeConfig::default(),
        }
    }

    pub fn new_with_go(zen_config: OpenCodeConfig, go_config: OpenCodeConfig) -> Self {
        Self {
            zen_config,
            go_config,
        }
    }
}

impl RoderExtension for OpenCodeExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-opencode-provider".to_string(),
            name: "OpenCode Providers".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("OpenCode Zen and OpenCode Go API key providers".to_string()),
            provides: vec![
                ProvidedService::InferenceEngine(PROVIDER_OPENCODE.to_string()),
                ProvidedService::InferenceEngine(PROVIDER_OPENCODE_GO.to_string()),
            ],
            required_capabilities: vec![CapabilityRequest::new("network.opencode.ai")],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.inference_engine(Arc::new(OpenCodeInferenceEngine::new(
            self.zen_config.clone(),
            OpenCodeProviderSpec::zen(),
        )));
        registry.inference_engine(Arc::new(OpenCodeInferenceEngine::new(
            self.go_config.clone(),
            OpenCodeProviderSpec::go(),
        )));
        Ok(())
    }
}
