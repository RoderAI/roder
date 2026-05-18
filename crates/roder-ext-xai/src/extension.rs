use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::catalog::{PROVIDER_SUPERGROK, PROVIDER_XAI};
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use roder_ext_openai_responses::OpenAiResponsesEngine;
use semver::Version;

use crate::provider::SuperGrokEngine;

const DEFAULT_XAI_BASE_URL: &str = "https://api.x.ai/v1";

pub struct XaiExtension {
    api_key: Option<String>,
    base_url: String,
}

impl XaiExtension {
    pub fn new(api_key: Option<String>, base_url: Option<String>) -> Self {
        Self {
            api_key,
            base_url: base_url.unwrap_or_else(|| DEFAULT_XAI_BASE_URL.to_string()),
        }
    }
}

impl RoderExtension for XaiExtension {
    fn manifest(&self) -> ExtensionManifest {
        let mut provides = vec![ProvidedService::InferenceEngine(
            PROVIDER_SUPERGROK.to_string(),
        )];
        let mut required_capabilities = vec![CapabilityRequest::new("network.api.x.ai")];
        if self.api_key.is_some() {
            provides.push(ProvidedService::InferenceEngine(PROVIDER_XAI.to_string()));
            required_capabilities.push(CapabilityRequest::new("secret.read.XAI_API_KEY"));
        }

        ExtensionManifest {
            id: "roder-ext-xai".to_string(),
            name: "xAI Grok Providers".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("xAI API-key and SuperGrok OAuth provider surfaces".to_string()),
            provides,
            required_capabilities,
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.inference_engine(Arc::new(SuperGrokEngine));
        if let Some(api_key) = self.api_key.as_ref() {
            registry.inference_engine(Arc::new(OpenAiResponsesEngine::new_with_config(
                api_key.clone(),
                PROVIDER_XAI,
                self.base_url.clone(),
                Vec::new(),
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use roder_api::extension::RoderExtension;

    use super::*;

    #[test]
    fn manifest_exposes_supergrok_without_api_key() {
        let manifest = XaiExtension::new(None, None).manifest();

        assert_eq!(
            manifest.provides,
            vec![ProvidedService::InferenceEngine(
                PROVIDER_SUPERGROK.to_string()
            )]
        );
    }

    #[test]
    fn manifest_exposes_xai_when_api_key_is_configured() {
        let manifest = XaiExtension::new(Some("secret".to_string()), None).manifest();

        assert!(
            manifest
                .provides
                .contains(&ProvidedService::InferenceEngine(PROVIDER_XAI.to_string()))
        );
        assert!(
            manifest
                .required_capabilities
                .iter()
                .any(|capability| capability.id.as_str() == "secret.read.XAI_API_KEY")
        );
    }
}
