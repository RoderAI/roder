use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::catalog::PROVIDER_SYNTHETIC;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;

use crate::provider::{SyntheticConfig, SyntheticInferenceEngine};

pub struct SyntheticExtension {
    config: SyntheticConfig,
}

impl SyntheticExtension {
    pub fn new(config: SyntheticConfig) -> Self {
        Self { config }
    }
}

impl RoderExtension for SyntheticExtension {
    fn manifest(&self) -> ExtensionManifest {
        let mut required_capabilities = vec![CapabilityRequest::new("network.api.synthetic.new")];
        if self.config.api_key.is_some() {
            required_capabilities.push(CapabilityRequest::new("secret.read.SYNTHETIC_API_KEY"));
        }
        ExtensionManifest {
            id: "roder-ext-synthetic-provider".to_string(),
            name: "Synthetic Provider".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some(
                "Synthetic API-key inference provider using OpenAI-compatible Chat Completions"
                    .to_string(),
            ),
            provides: vec![ProvidedService::InferenceEngine(
                PROVIDER_SYNTHETIC.to_string(),
            )],
            required_capabilities,
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.inference_engine(Arc::new(SyntheticInferenceEngine::new(self.config.clone())));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_exposes_synthetic_without_secret_until_configured() {
        let without_key = SyntheticExtension::new(SyntheticConfig::default()).manifest();
        assert_eq!(
            without_key.provides,
            vec![ProvidedService::InferenceEngine(
                PROVIDER_SYNTHETIC.to_string()
            )]
        );
        assert!(
            without_key
                .required_capabilities
                .iter()
                .any(|capability| capability.id == "network.api.synthetic.new")
        );
        assert!(
            !without_key
                .required_capabilities
                .iter()
                .any(|capability| capability.id == "secret.read.SYNTHETIC_API_KEY")
        );
    }

    #[test]
    fn manifest_declares_secret_capability_when_api_key_is_configured() {
        let with_key = SyntheticExtension::new(SyntheticConfig {
            api_key: Some("secret".to_string()),
            base_url: None,
        })
        .manifest();
        assert!(
            with_key
                .required_capabilities
                .iter()
                .any(|capability| capability.id == "secret.read.SYNTHETIC_API_KEY")
        );
    }
}
