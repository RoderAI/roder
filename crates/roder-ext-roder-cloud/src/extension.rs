use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::catalog::PROVIDER_RODER_CLOUD;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;

use crate::engine::RoderCloudEngine;

#[derive(Debug, Clone, Default)]
pub struct RoderCloudConfig {
    /// Long-lived `roder_` team API key from the roder.cloud dashboard.
    pub api_key: Option<String>,
    /// Inference edge base URL including `/v1` (deploy-specific; local dev
    /// is `http://127.0.0.1:8080/v1`).
    pub base_url: Option<String>,
    /// Dashboard / token-exchange host; defaults to `https://roder.cloud`.
    pub web_url: Option<String>,
}

pub struct RoderCloudExtension {
    config: RoderCloudConfig,
}

impl RoderCloudExtension {
    pub fn new(config: RoderCloudConfig) -> Self {
        Self { config }
    }
}

impl RoderExtension for RoderCloudExtension {
    fn manifest(&self) -> ExtensionManifest {
        let mut required_capabilities = vec![CapabilityRequest::new("network.roder.cloud")];
        if self.config.api_key.is_some() {
            required_capabilities.push(CapabilityRequest::new("secret.read.RODER_CLOUD_API_KEY"));
        }
        ExtensionManifest {
            id: "roder-ext-roder-cloud-provider".to_string(),
            name: "Roder Cloud Provider".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("roder.cloud hosted inference provider".to_string()),
            provides: vec![ProvidedService::InferenceEngine(
                PROVIDER_RODER_CLOUD.to_string(),
            )],
            required_capabilities,
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.inference_engine(Arc::new(RoderCloudEngine::new(
            self.config.api_key.clone(),
            self.config.base_url.clone(),
            self.config.web_url.clone(),
        )));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use roder_api::extension::RoderExtension;

    use super::*;

    #[test]
    fn manifest_exposes_roder_cloud_without_api_key() {
        let manifest = RoderCloudExtension::new(RoderCloudConfig::default()).manifest();

        assert_eq!(
            manifest.provides,
            vec![ProvidedService::InferenceEngine(
                PROVIDER_RODER_CLOUD.to_string()
            )]
        );
        assert!(
            !manifest
                .required_capabilities
                .iter()
                .any(|capability| capability.id == "secret.read.RODER_CLOUD_API_KEY")
        );
    }

    #[test]
    fn manifest_declares_secret_capability_when_api_key_is_configured() {
        let manifest = RoderCloudExtension::new(RoderCloudConfig {
            api_key: Some("roder_secret".to_string()),
            ..RoderCloudConfig::default()
        })
        .manifest();

        assert!(
            manifest
                .required_capabilities
                .iter()
                .any(|capability| capability.id == "secret.read.RODER_CLOUD_API_KEY")
        );
    }
}
