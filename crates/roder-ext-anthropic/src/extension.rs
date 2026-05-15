use crate::provider::AnthropicEngine;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;
use std::sync::Arc;

pub struct AnthropicExtension {
    api_key: String,
}

impl AnthropicExtension {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

impl RoderExtension for AnthropicExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-anthropic".to_string(),
            name: "Anthropic Provider".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("Anthropic Inference Provider".to_string()),
            provides: vec![ProvidedService::InferenceEngine("anthropic".to_string())],
            required_capabilities: vec![],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.inference_engine(Arc::new(AnthropicEngine::new(self.api_key.clone())));
        Ok(())
    }
}
