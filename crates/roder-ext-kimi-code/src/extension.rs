use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::catalog::PROVIDER_KIMI_CODE;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;

use crate::provider::{KimiCodeConfig, KimiCodeInferenceEngine, KimiCodeProviderSpec};

pub struct KimiCodeExtension {
    config: KimiCodeConfig,
}

impl KimiCodeExtension {
    pub fn new(config: KimiCodeConfig) -> Self {
        Self { config }
    }
}

impl RoderExtension for KimiCodeExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-kimi-code-provider".to_string(),
            name: "Kimi Code Provider".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some(
                "Kimi Code (Moonshot AI) direct subscription inference provider".to_string(),
            ),
            provides: vec![ProvidedService::InferenceEngine(
                PROVIDER_KIMI_CODE.to_string(),
            )],
            required_capabilities: vec![CapabilityRequest::new("network.kimi.com")],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.inference_engine(Arc::new(KimiCodeInferenceEngine::new(
            self.config.clone(),
            KimiCodeProviderSpec::default(),
        )));
        Ok(())
    }
}
