use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::catalog::PROVIDER_FIREWORKS;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use roder_ext_openai_responses::OpenAiResponsesEngine;
use semver::Version;

pub const DEFAULT_BASE_URL: &str = "https://api.fireworks.ai/inference/v1";

#[derive(Debug, Clone, Default)]
pub struct FireworksConfig {
    pub api_key: Option<String>,
    pub base_url: Option<String>,
}

pub struct FireworksExtension {
    config: FireworksConfig,
}

impl FireworksExtension {
    pub fn new(config: FireworksConfig) -> Self {
        Self { config }
    }
}

impl RoderExtension for FireworksExtension {
    fn manifest(&self) -> ExtensionManifest {
        let mut required_capabilities = vec![CapabilityRequest::new("network.api.fireworks.ai")];
        if self.config.api_key.is_some() {
            required_capabilities.push(CapabilityRequest::new("secret.read.FIREWORKS_API_KEY"));
        }
        ExtensionManifest {
            id: "roder-ext-fireworks-provider".to_string(),
            name: "Fireworks AI Provider".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("Fireworks AI API key provider".to_string()),
            provides: vec![ProvidedService::InferenceEngine(
                PROVIDER_FIREWORKS.to_string(),
            )],
            required_capabilities,
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.inference_engine(Arc::new(OpenAiResponsesEngine::new_fireworks_provider(
            self.config.api_key.clone(),
            self.config
                .base_url
                .clone()
                .unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
        )));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use roder_api::extension::RoderExtension;

    use super::*;

    #[test]
    fn manifest_exposes_fireworks_without_api_key() {
        let manifest = FireworksExtension::new(FireworksConfig::default()).manifest();

        assert_eq!(
            manifest.provides,
            vec![ProvidedService::InferenceEngine(
                PROVIDER_FIREWORKS.to_string()
            )]
        );
        assert!(
            !manifest
                .required_capabilities
                .iter()
                .any(|capability| capability.id == "secret.read.FIREWORKS_API_KEY")
        );
    }

    #[test]
    fn manifest_declares_secret_capability_when_api_key_is_configured() {
        let manifest = FireworksExtension::new(FireworksConfig {
            api_key: Some("secret".to_string()),
            ..FireworksConfig::default()
        })
        .manifest();

        assert!(
            manifest
                .required_capabilities
                .iter()
                .any(|capability| capability.id == "secret.read.FIREWORKS_API_KEY")
        );
    }
}
