use crate::provider::GeminiEngine;
use roder_api::capabilities::CapabilityRequest;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;
use std::sync::Arc;

pub struct GeminiExtension {
    /// Absent keys still register the engine; inference fails at call time.
    api_key: Option<String>,
}

impl GeminiExtension {
    pub fn new(api_key: Option<String>) -> Self {
        Self { api_key }
    }
}

impl RoderExtension for GeminiExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-gemini".to_string(),
            name: "Gemini Provider".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("Google Gemini Inference Provider".to_string()),
            provides: vec![ProvidedService::InferenceEngine("gemini".to_string())],
            required_capabilities: vec![
                CapabilityRequest::new("network.api.googleai"),
                CapabilityRequest::new("secret.read.GEMINI_API_KEY"),
            ],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.inference_engine(Arc::new(GeminiEngine::new(self.api_key.clone())));
        Ok(())
    }
}
