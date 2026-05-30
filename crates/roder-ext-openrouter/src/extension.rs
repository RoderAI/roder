use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::catalog::PROVIDER_OPENROUTER;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use roder_ext_openai_responses::OpenAiResponsesEngine;
use semver::Version;

const DEFAULT_BASE_URL: &str = "https://openrouter.ai/api/v1";

#[derive(Debug, Clone, Default)]
pub struct OpenRouterConfig {
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub http_referer: Option<String>,
    pub app_title: Option<String>,
}

pub struct OpenRouterExtension {
    config: OpenRouterConfig,
}

impl OpenRouterExtension {
    pub fn new(config: OpenRouterConfig) -> Self {
        Self { config }
    }
}

impl RoderExtension for OpenRouterExtension {
    fn manifest(&self) -> ExtensionManifest {
        let mut required_capabilities = vec![CapabilityRequest::new("network.openrouter.ai")];
        if self.config.api_key.is_some() {
            required_capabilities.push(CapabilityRequest::new("secret.read.OPENROUTER_API_KEY"));
        }
        ExtensionManifest {
            id: "roder-ext-openrouter-provider".to_string(),
            name: "OpenRouter Provider".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("OpenRouter API key provider".to_string()),
            provides: vec![ProvidedService::InferenceEngine(
                PROVIDER_OPENROUTER.to_string(),
            )],
            required_capabilities,
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.inference_engine(Arc::new(OpenAiResponsesEngine::new_openrouter_provider(
            self.config.api_key.clone(),
            self.config
                .base_url
                .clone()
                .unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
            openrouter_headers(&self.config),
        )));
        Ok(())
    }
}

fn openrouter_headers(config: &OpenRouterConfig) -> Vec<(String, String)> {
    let mut headers = Vec::new();
    if let Some(value) = nonempty(config.http_referer.clone()) {
        headers.push(("HTTP-Referer".to_string(), value));
    }
    if let Some(value) = nonempty(config.app_title.clone()) {
        headers.push(("X-Title".to_string(), value));
    }
    headers
}

fn nonempty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use roder_api::extension::RoderExtension;

    use super::*;

    #[test]
    fn manifest_exposes_openrouter_without_api_key() {
        let manifest = OpenRouterExtension::new(OpenRouterConfig::default()).manifest();

        assert_eq!(
            manifest.provides,
            vec![ProvidedService::InferenceEngine(
                PROVIDER_OPENROUTER.to_string()
            )]
        );
        assert!(
            !manifest
                .required_capabilities
                .iter()
                .any(|capability| capability.id == "secret.read.OPENROUTER_API_KEY")
        );
    }

    #[test]
    fn manifest_declares_secret_capability_when_api_key_is_configured() {
        let manifest = OpenRouterExtension::new(OpenRouterConfig {
            api_key: Some("secret".to_string()),
            ..OpenRouterConfig::default()
        })
        .manifest();

        assert!(
            manifest
                .required_capabilities
                .iter()
                .any(|capability| capability.id == "secret.read.OPENROUTER_API_KEY")
        );
    }
}
