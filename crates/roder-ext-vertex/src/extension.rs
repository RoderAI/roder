use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;

use crate::provider::{VERTEX_PROVIDER_ID, VertexConfig, VertexEngine};

pub struct VertexExtension {
    /// Empty credential fields still register the engine; inference fails at
    /// call time naming the credential env vars.
    config: VertexConfig,
}

impl VertexExtension {
    pub fn new(config: VertexConfig) -> Self {
        Self { config }
    }
}

impl RoderExtension for VertexExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-vertex".to_string(),
            name: "Vertex AI Provider".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("Google Vertex AI Inference Provider".to_string()),
            provides: vec![ProvidedService::InferenceEngine(
                VERTEX_PROVIDER_ID.to_string(),
            )],
            required_capabilities: vec![
                CapabilityRequest::new("network.api.vertexai"),
                CapabilityRequest::new("secret.read.GOOGLE_APPLICATION_CREDENTIALS"),
                CapabilityRequest::new("secret.read.VERTEX_CREDENTIALS_JSON"),
            ],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.inference_engine(Arc::new(VertexEngine::new(self.config.clone())));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_provides_vertex_engine_and_credential_capabilities() {
        let manifest = VertexExtension::new(VertexConfig::default()).manifest();

        assert_eq!(
            manifest.provides,
            vec![ProvidedService::InferenceEngine("vertex".to_string())]
        );
        for capability in [
            "network.api.vertexai",
            "secret.read.GOOGLE_APPLICATION_CREDENTIALS",
            "secret.read.VERTEX_CREDENTIALS_JSON",
        ] {
            assert!(
                manifest
                    .required_capabilities
                    .iter()
                    .any(|request| request.id.as_str() == capability),
                "missing {capability}"
            );
        }
    }
}
