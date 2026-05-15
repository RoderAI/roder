use crate::provider::OpenAiChatCompletionsEngine;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;
use std::sync::Arc;

pub struct OpenAiChatCompletionsExtension {
    api_key: String,
}

impl OpenAiChatCompletionsExtension {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

impl RoderExtension for OpenAiChatCompletionsExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-openai-chat-completions".to_string(),
            name: "OpenAI Chat Completions".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("OpenAI Chat Completions Provider".to_string()),
            provides: vec![ProvidedService::InferenceEngine(
                "openai-chat-completions".to_string(),
            )],
            required_capabilities: vec![],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.inference_engine(Arc::new(OpenAiChatCompletionsEngine::new(
            self.api_key.clone(),
        )));
        Ok(())
    }
}
