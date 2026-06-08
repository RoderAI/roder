use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;

pub mod provider;

pub use provider::{
    DEFAULT_DIMENSIONS, DEFAULT_ENDPOINT, DEFAULT_MODEL, GOOGLE_EMBEDDING_PROVIDER_ID,
    GoogleEmbeddingProvider, GoogleEmbeddingsConfig, document_input, query_input,
};

pub struct GoogleEmbeddingsExtension {
    config: GoogleEmbeddingsConfig,
}

impl GoogleEmbeddingsExtension {
    pub fn from_env() -> Self {
        Self {
            config: GoogleEmbeddingsConfig::from_env(),
        }
    }

    pub fn new(config: GoogleEmbeddingsConfig) -> Self {
        Self { config }
    }

    pub fn with_api_key(api_key: impl Into<String>) -> Self {
        Self {
            config: GoogleEmbeddingsConfig {
                api_key: Some(api_key.into()),
                ..GoogleEmbeddingsConfig::default()
            },
        }
    }
}

impl RoderExtension for GoogleEmbeddingsExtension {
    fn manifest(&self) -> ExtensionManifest {
        let mut required_capabilities = vec![CapabilityRequest::new(
            "network.generativelanguage.googleapis.com",
        )];
        if self.config.api_key.is_some() {
            required_capabilities.push(CapabilityRequest::new("secret.read.google_api_key"));
        }
        ExtensionManifest {
            id: "roder-ext-google-embeddings".to_string(),
            name: "Google Gemini Embeddings".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("Google Gemini Embedding 2 provider".to_string()),
            provides: vec![ProvidedService::EmbeddingProvider(
                GOOGLE_EMBEDDING_PROVIDER_ID.to_string(),
            )],
            required_capabilities,
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.embedding_provider(Arc::new(GoogleEmbeddingProvider::new(self.config.clone())));
        Ok(())
    }
}

pub fn extension() -> GoogleEmbeddingsExtension {
    GoogleEmbeddingsExtension::from_env()
}

#[cfg(test)]
mod tests {
    use roder_api::extension::RoderExtension;

    use super::*;

    #[test]
    fn manifest_exposes_google_embedding_provider_without_credentials() {
        let manifest = GoogleEmbeddingsExtension::new(GoogleEmbeddingsConfig::default()).manifest();

        assert_eq!(
            manifest.provides,
            vec![ProvidedService::EmbeddingProvider(
                GOOGLE_EMBEDDING_PROVIDER_ID.to_string()
            )]
        );
        assert!(
            !manifest
                .required_capabilities
                .iter()
                .any(|capability| capability.id == "secret.read.google_api_key")
        );
    }

    #[test]
    fn manifest_tracks_google_embedding_secret_when_configured() {
        let manifest = GoogleEmbeddingsExtension::with_api_key("key").manifest();

        assert!(
            manifest
                .required_capabilities
                .iter()
                .any(|capability| capability.id == "secret.read.google_api_key")
        );
    }
}
