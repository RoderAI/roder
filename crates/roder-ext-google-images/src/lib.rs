use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;

mod provider;

pub use provider::{GOOGLE_IMAGES_PROVIDER_ID, GoogleImagesConfig, GoogleImagesProvider};

/// Google Gemini Nano Banana image generation extension. The media provider
/// id is `google` (scoped to media generation only); the chat inference
/// provider keeps its separate `gemini` id.
#[derive(Debug, Clone)]
pub struct GoogleImagesExtension {
    config: GoogleImagesConfig,
}

impl GoogleImagesExtension {
    pub fn new(api_key: Option<String>) -> Self {
        Self {
            config: GoogleImagesConfig::new(api_key),
        }
    }

    pub fn with_config(config: GoogleImagesConfig) -> Self {
        Self { config }
    }
}

impl RoderExtension for GoogleImagesExtension {
    fn manifest(&self) -> ExtensionManifest {
        let mut required_capabilities = vec![CapabilityRequest::new(
            "network.generativelanguage.googleapis.com",
        )];
        if self.config.api_key.is_some() {
            required_capabilities.push(CapabilityRequest::new("secret.read.GEMINI_API_KEY"));
        }
        ExtensionManifest {
            id: "roder-ext-google-images".to_string(),
            name: "Google Gemini Images".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some(
                "Google Gemini Nano Banana image generation provider".to_string(),
            ),
            provides: vec![ProvidedService::MediaGenerator(
                GOOGLE_IMAGES_PROVIDER_ID.to_string(),
            )],
            required_capabilities,
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.media_generator_provider(Arc::new(GoogleImagesProvider::new(self.config.clone())));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use roder_api::extension::RoderExtension;

    use super::*;

    #[test]
    fn manifest_exposes_google_media_generator_without_key() {
        let manifest = GoogleImagesExtension::new(None).manifest();

        assert_eq!(
            manifest.provides,
            vec![ProvidedService::MediaGenerator(
                GOOGLE_IMAGES_PROVIDER_ID.to_string()
            )]
        );
        assert!(
            !manifest
                .required_capabilities
                .iter()
                .any(|capability| capability.id == "secret.read.GEMINI_API_KEY")
        );
    }

    #[test]
    fn manifest_requests_secret_capability_when_key_is_configured() {
        let manifest = GoogleImagesExtension::new(Some("secret".to_string())).manifest();

        assert!(
            manifest
                .required_capabilities
                .iter()
                .any(|capability| capability.id == "secret.read.GEMINI_API_KEY")
        );
    }
}
