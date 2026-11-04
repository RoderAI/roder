use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::catalog::PROVIDER_HOPPER;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;

use crate::provider::{HopperConfig, HopperInferenceEngine};

pub struct HopperExtension {
    config: HopperConfig,
}

impl HopperExtension {
    pub fn new(config: HopperConfig) -> Self {
        Self { config }
    }
}

impl RoderExtension for HopperExtension {
    fn manifest(&self) -> ExtensionManifest {
        let base_url = HopperInferenceEngine::new(self.config.clone()).base_url();
        let mut required_capabilities = reqwest::Url::parse(&base_url)
            .ok()
            .and_then(|url| url.host_str().map(|host| format!("network.{host}")))
            .map(CapabilityRequest::new)
            .into_iter()
            .collect::<Vec<_>>();
        if self.config.api_key.is_some() {
            required_capabilities.push(CapabilityRequest::new("secret.read.HOPPER_API_KEY"));
        }
        ExtensionManifest {
            id: "roder-ext-hopper-provider".to_string(),
            name: "Hopper Provider".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some(
                "Hopper API-key inference provider using OpenAI-compatible Chat Completions"
                    .to_string(),
            ),
            provides: vec![ProvidedService::InferenceEngine(
                PROVIDER_HOPPER.to_string(),
            )],
            required_capabilities,
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.inference_engine(Arc::new(HopperInferenceEngine::new(self.config.clone())));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestEnv;

    #[test]
    fn manifest_exposes_hopper_without_secret_until_configured() {
        let _env = TestEnv::new();
        let without_key = HopperExtension::new(HopperConfig::default()).manifest();
        assert_eq!(
            without_key.provides,
            vec![ProvidedService::InferenceEngine(
                PROVIDER_HOPPER.to_string()
            )]
        );
        assert!(
            without_key
                .required_capabilities
                .iter()
                .any(|capability| capability.id == "network.api.withhopper.com")
        );
        assert!(
            !without_key
                .required_capabilities
                .iter()
                .any(|capability| capability.id == "secret.read.HOPPER_API_KEY")
        );
    }

    #[test]
    fn manifest_declares_secret_capability_when_api_key_is_configured() {
        let _env = TestEnv::new();
        let with_key = HopperExtension::new(HopperConfig {
            api_key: Some("secret".to_string()),
            base_url: None,
        })
        .manifest();
        assert!(
            with_key
                .required_capabilities
                .iter()
                .any(|capability| capability.id == "secret.read.HOPPER_API_KEY")
        );
    }

    #[test]
    fn manifest_network_capability_tracks_resolved_base_url() {
        let env = TestEnv::new();
        let network_capabilities = |config| {
            HopperExtension::new(config)
                .manifest()
                .required_capabilities
                .into_iter()
                .filter(|capability| capability.id.starts_with("network."))
                .map(|capability| capability.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            network_capabilities(HopperConfig::default()),
            ["network.api.withhopper.com"]
        );
        // SAFETY: TestEnv holds this test binary's environment lock.
        unsafe {
            std::env::set_var("HOPPER_BASE_URL", "https://env.example/v1");
            std::env::set_var("RODER_HOPPER_BASE_URL", "https://alias.example/v1");
        }
        assert_eq!(
            network_capabilities(HopperConfig::default()),
            ["network.env.example"]
        );
        std::fs::write(
            env.config_dir.join("config.toml"),
            "[providers.hopper]\nbase_url = \"https://config.example/v1\"\n",
        )
        .unwrap();
        assert_eq!(
            network_capabilities(HopperConfig::default()),
            ["network.config.example"]
        );
        assert_eq!(
            network_capabilities(HopperConfig {
                api_key: None,
                base_url: Some("http://127.0.0.1:8888/proxy/v1".into()),
            }),
            ["network.127.0.0.1"]
        );
    }
}
