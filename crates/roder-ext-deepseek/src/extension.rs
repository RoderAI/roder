use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::catalog::PROVIDER_DEEPSEEK;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;

use crate::provider::{DeepSeekConfig, DeepSeekInferenceEngine};

pub struct DeepSeekExtension {
    config: DeepSeekConfig,
}

impl DeepSeekExtension {
    pub fn new(config: DeepSeekConfig) -> Self {
        Self { config }
    }
}

impl RoderExtension for DeepSeekExtension {
    fn manifest(&self) -> ExtensionManifest {
        let mut required_capabilities = vec![CapabilityRequest::new("network.api.deepseek.com")];
        if self.config.api_key.is_some() {
            required_capabilities.push(CapabilityRequest::new("secret.read.DEEPSEEK_API_KEY"));
        }
        ExtensionManifest {
            id: "roder-ext-deepseek-provider".to_string(),
            name: "DeepSeek Platform Provider".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some(
                "DeepSeek Platform API-key inference provider using OpenAI-compatible Chat Completions"
                    .to_string(),
            ),
            provides: vec![ProvidedService::InferenceEngine(
                PROVIDER_DEEPSEEK.to_string(),
            )],
            required_capabilities,
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.inference_engine(Arc::new(DeepSeekInferenceEngine::new(self.config.clone())));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_exposes_deepseek_without_secret_until_configured() {
        let without_key = DeepSeekExtension::new(DeepSeekConfig::default()).manifest();
        assert_eq!(
            without_key.provides,
            vec![ProvidedService::InferenceEngine(
                PROVIDER_DEEPSEEK.to_string()
            )]
        );
        assert!(
            without_key
                .required_capabilities
                .iter()
                .any(|capability| capability.id == "network.api.deepseek.com")
        );
        assert!(
            !without_key
                .required_capabilities
                .iter()
                .any(|capability| capability.id == "secret.read.DEEPSEEK_API_KEY")
        );
    }

    #[test]
    fn manifest_declares_secret_capability_when_api_key_is_configured() {
        let with_key = DeepSeekExtension::new(DeepSeekConfig {
            api_key: Some("secret".to_string()),
            base_url: None,
        })
        .manifest();
        assert!(
            with_key
                .required_capabilities
                .iter()
                .any(|capability| capability.id == "secret.read.DEEPSEEK_API_KEY")
        );
    }
}
