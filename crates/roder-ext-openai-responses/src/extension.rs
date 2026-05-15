use crate::provider::OpenAiResponsesEngine;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;
use std::sync::Arc;

pub struct OpenAiResponsesExtension {
    api_key: String,
}

impl OpenAiResponsesExtension {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

impl RoderExtension for OpenAiResponsesExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-openai-responses".to_string(),
            name: "OpenAI Responses".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("OpenAI Responses Provider".to_string()),
            provides: vec![ProvidedService::InferenceEngine(
                "openai-responses".to_string(),
            )],
            required_capabilities: vec![],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.inference_engine(Arc::new(OpenAiResponsesEngine::new(self.api_key.clone())));
        Ok(())
    }
}
