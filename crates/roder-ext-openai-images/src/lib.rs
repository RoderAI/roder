use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;

mod provider;

pub use provider::{OPENAI_IMAGES_PROVIDER_ID, OpenAiImagesConfig, OpenAiImagesProvider};

#[derive(Debug, Clone)]
pub struct OpenAiImagesExtension {
    config: OpenAiImagesConfig,
}

impl OpenAiImagesExtension {
    pub fn new(api_key: Option<String>) -> Self {
        Self {
            config: OpenAiImagesConfig::new(api_key),
        }
    }

    pub fn with_config(config: OpenAiImagesConfig) -> Self {
        Self { config }
    }
}

impl RoderExtension for OpenAiImagesExtension {
    fn manifest(&self) -> ExtensionManifest {
        let mut required_capabilities = vec![CapabilityRequest::new("network.api.openai.com")];
        if self.config.api_key.is_some() {
            required_capabilities.push(CapabilityRequest::new("secret.read.OPENAI_API_KEY"));
        }
        ExtensionManifest {
            id: "roder-ext-openai-images".to_string(),
            name: "OpenAI GPT Image".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("OpenAI GPT Image generation and editing provider".to_string()),
            provides: vec![ProvidedService::MediaGenerator(
                OPENAI_IMAGES_PROVIDER_ID.to_string(),
            )],
            required_capabilities,
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.media_generator_provider(Arc::new(OpenAiImagesProvider::new(self.config.clone())));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use roder_api::extension::RoderExtension;

    use super::*;

    #[test]
    fn manifest_exposes_openai_media_generator_without_key() {
        let manifest = OpenAiImagesExtension::new(None).manifest();

        assert_eq!(
            manifest.provides,
            vec![ProvidedService::MediaGenerator(
                OPENAI_IMAGES_PROVIDER_ID.to_string()
            )]
        );
        assert!(
            !manifest
                .required_capabilities
                .iter()
                .any(|capability| capability.id == "secret.read.OPENAI_API_KEY")
        );
    }

    #[test]
    fn manifest_requests_secret_capability_when_key_is_configured() {
        let manifest = OpenAiImagesExtension::new(Some("secret".to_string())).manifest();

        assert!(
            manifest
                .required_capabilities
                .iter()
                .any(|capability| capability.id == "secret.read.OPENAI_API_KEY")
        );
    }
}
