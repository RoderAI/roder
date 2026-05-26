use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::catalog::{PROVIDER_XIAOMI_MIMO, PROVIDER_XIAOMI_MIMO_TOKEN_PLAN};
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;

use crate::provider::{XiaomiMimoConfig, XiaomiMimoInferenceEngine, XiaomiMimoProviderSpec};
use crate::speech::XiaomiMimoSpeechSynthesizer;

pub struct XiaomiMimoExtension {
    config: XiaomiMimoConfig,
}

impl XiaomiMimoExtension {
    pub fn new(config: XiaomiMimoConfig) -> Self {
        Self { config }
    }
}

impl RoderExtension for XiaomiMimoExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-xiaomi-mimo".to_string(),
            name: "Xiaomi MiMo Providers".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some(
                "Xiaomi MiMo pay-as-you-go and Token Plan Chat Completions providers".to_string(),
            ),
            provides: vec![
                ProvidedService::InferenceEngine(PROVIDER_XIAOMI_MIMO.to_string()),
                ProvidedService::InferenceEngine(PROVIDER_XIAOMI_MIMO_TOKEN_PLAN.to_string()),
                ProvidedService::SpeechSynthesizer(PROVIDER_XIAOMI_MIMO.to_string()),
                ProvidedService::SpeechSynthesizer(PROVIDER_XIAOMI_MIMO_TOKEN_PLAN.to_string()),
            ],
            required_capabilities: vec![
                CapabilityRequest::new("network.api.xiaomimimo.com"),
                CapabilityRequest::new("network.token-plan.xiaomimimo.com"),
            ],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        for spec in [
            XiaomiMimoProviderSpec::pay_as_you_go(),
            XiaomiMimoProviderSpec::token_plan(),
        ] {
            registry.inference_engine(Arc::new(XiaomiMimoInferenceEngine::new(
                self.config.clone(),
                spec,
            )));
            registry.speech_synthesizer(Arc::new(XiaomiMimoSpeechSynthesizer::new(
                self.config.clone(),
                spec,
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
    fn manifest_exposes_text_and_speech_for_both_billing_modes() {
        let manifest = XiaomiMimoExtension::new(XiaomiMimoConfig::default()).manifest();

        assert!(
            manifest
                .provides
                .contains(&ProvidedService::InferenceEngine(
                    PROVIDER_XIAOMI_MIMO.to_string()
                ))
        );
        assert!(
            manifest
                .provides
                .contains(&ProvidedService::InferenceEngine(
                    PROVIDER_XIAOMI_MIMO_TOKEN_PLAN.to_string()
                ))
        );
        assert!(
            manifest
                .provides
                .contains(&ProvidedService::SpeechSynthesizer(
                    PROVIDER_XIAOMI_MIMO.to_string()
                ))
        );
        assert!(
            manifest
                .provides
                .contains(&ProvidedService::SpeechSynthesizer(
                    PROVIDER_XIAOMI_MIMO_TOKEN_PLAN.to_string()
                ))
        );
    }
}
