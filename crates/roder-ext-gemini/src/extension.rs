use semver::Version;
use roder_api::extension::{ExtensionManifest, ExtensionRegistryBuilder, RoderExtension};
use std::sync::Arc;
use crate::provider::GeminiEngine;

pub struct GeminiExtension {
    api_key: String,
}

impl GeminiExtension {
    pub fn new(api_key: String) -> Self {
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
            provides: vec![],
            required_capabilities: vec![],
        }
    }

    fn install(
        &self,
        registry: &mut ExtensionRegistryBuilder,
    ) -> anyhow::Result<()> {
        registry.inference_engine(Arc::new(GeminiEngine::new(self.api_key.clone())));
        Ok(())
    }
}