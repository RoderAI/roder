pub mod provider;

use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;

pub use provider::{
    DEFAULT_DIMENSIONS, DEFAULT_ENDPOINT, DEFAULT_MODEL, ZEROENTROPY_EMBEDDING_PROVIDER_ID,
    ZeroEntropyEmbeddingProvider, ZeroEntropyEmbeddingsConfig, ZeroEntropyEncodingFormat,
    ZeroEntropyLatency,
};

pub struct ZeroEntropyEmbeddingsExtension {
    config: ZeroEntropyEmbeddingsConfig,
}

impl ZeroEntropyEmbeddingsExtension {
    pub fn from_env() -> Self {
        Self {
            config: ZeroEntropyEmbeddingsConfig::from_env(),
        }
    }

    pub fn new(config: ZeroEntropyEmbeddingsConfig) -> Self {
        Self { config }
    }

    pub fn with_api_key(api_key: impl Into<String>) -> Self {
        Self {
            config: ZeroEntropyEmbeddingsConfig {
                api_key: Some(api_key.into()),
                ..ZeroEntropyEmbeddingsConfig::default()
            },
        }
    }
}

impl RoderExtension for ZeroEntropyEmbeddingsExtension {
    fn manifest(&self) -> ExtensionManifest {
        let mut required_capabilities = vec![CapabilityRequest::new("network.api.zeroentropy.dev")];
        if self.config.api_key.is_some() {
            required_capabilities.push(CapabilityRequest::new("secret.read.zeroentropy_api_key"));
        }
        ExtensionManifest {
            id: "roder-ext-zeroentropy-embeddings".to_string(),
            name: "ZeroEntropy Embeddings".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("ZeroEntropy zembed-1 embedding provider".to_string()),
            provides: vec![ProvidedService::EmbeddingProvider(
                ZEROENTROPY_EMBEDDING_PROVIDER_ID.to_string(),
            )],
            required_capabilities,
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.embedding_provider(Arc::new(ZeroEntropyEmbeddingProvider::new(
            self.config.clone(),
        )));
        Ok(())
    }
}

pub fn extension() -> ZeroEntropyEmbeddingsExtension {
    ZeroEntropyEmbeddingsExtension::from_env()
}

#[cfg(test)]
mod tests {
    use roder_api::extension::RoderExtension;

    use super::*;

    #[test]
    fn manifest_exposes_zeroentropy_embedding_provider_without_credentials() {
        let manifest =
            ZeroEntropyEmbeddingsExtension::new(ZeroEntropyEmbeddingsConfig::default()).manifest();

        assert_eq!(
            manifest.provides,
            vec![ProvidedService::EmbeddingProvider(
                ZEROENTROPY_EMBEDDING_PROVIDER_ID.to_string()
            )]
        );
        assert!(
            !manifest
                .required_capabilities
                .iter()
                .any(|capability| capability.id == "secret.read.zeroentropy_api_key")
        );
    }

    #[test]
    fn manifest_tracks_zeroentropy_secret_when_configured() {
        let manifest = ZeroEntropyEmbeddingsExtension::with_api_key("key").manifest();

        assert!(
            manifest
                .required_capabilities
                .iter()
                .any(|capability| capability.id == "secret.read.zeroentropy_api_key")
        );
    }
}
