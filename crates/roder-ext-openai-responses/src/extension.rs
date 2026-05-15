use crate::provider::OpenAiResponsesEngine;
use roder_api::catalog::PROVIDER_OPENAI;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;
use std::sync::Arc;

pub struct OpenAiResponsesExtension {
    api_key: String,
    provider_id: String,
}

impl OpenAiResponsesExtension {
    pub fn new(api_key: String) -> Self {
        Self::new_with_provider_id(api_key, PROVIDER_OPENAI)
    }

    pub fn new_with_provider_id(api_key: String, provider_id: impl Into<String>) -> Self {
        Self {
            api_key,
            provider_id: provider_id.into(),
        }
    }
}

impl RoderExtension for OpenAiResponsesExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: format!("roder-ext-openai-responses-{}", self.provider_id),
            name: format!("OpenAI Responses ({})", self.provider_id),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("OpenAI Responses Provider".to_string()),
            provides: vec![ProvidedService::InferenceEngine(self.provider_id.clone())],
            required_capabilities: vec![],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.inference_engine(Arc::new(OpenAiResponsesEngine::new_with_provider_id(
            self.api_key.clone(),
            self.provider_id.clone(),
        )));
        Ok(())
    }
}
