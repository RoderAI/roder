use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;

pub mod provider;

pub use provider::OpenAiEmbeddingProvider;

pub struct OpenAiEmbeddingsExtension {
    api_key: Option<String>,
}

impl OpenAiEmbeddingsExtension {
    pub fn from_env() -> Self {
        Self {
            api_key: std::env::var("OPENAI_API_KEY").ok(),
        }
    }

    pub fn with_api_key(api_key: impl Into<String>) -> Self {
        Self {
            api_key: Some(api_key.into()),
        }
    }
}

impl RoderExtension for OpenAiEmbeddingsExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-openai-embeddings".to_string(),
            name: "OpenAI Embeddings".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("OpenAI text-embedding-3-large provider".to_string()),
            provides: vec![ProvidedService::EmbeddingProvider("openai".to_string())],
            required_capabilities: vec![
                CapabilityRequest::new("network.openai"),
                CapabilityRequest::new("secrets.openai-api-key"),
            ],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.embedding_provider(Arc::new(OpenAiEmbeddingProvider::new(self.api_key.clone())));
        Ok(())
    }
}

pub fn extension() -> OpenAiEmbeddingsExtension {
    OpenAiEmbeddingsExtension::from_env()
}
