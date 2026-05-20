use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::catalog::PROVIDER_POOLSIDE;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;

use crate::provider::{PoolsideConfig, PoolsideInferenceEngine};

pub struct PoolsideExtension {
    config: PoolsideConfig,
}

impl PoolsideExtension {
    pub fn new(config: PoolsideConfig) -> Self {
        Self { config }
    }
}

impl RoderExtension for PoolsideExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-poolside-provider".to_string(),
            name: "Poolside Provider".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("Poolside Laguna API key provider".to_string()),
            provides: vec![ProvidedService::InferenceEngine(
                PROVIDER_POOLSIDE.to_string(),
            )],
            required_capabilities: vec![CapabilityRequest::new("network.inference.poolside.ai")],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.inference_engine(Arc::new(PoolsideInferenceEngine::new(self.config.clone())));
        Ok(())
    }
}
