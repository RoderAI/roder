use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;

mod provider;

pub use provider::{OPENAI_SPEECH_PROVIDER_ID, OpenAiSpeechConfig, OpenAiSpeechTranscriber};

#[derive(Debug, Clone)]
pub struct OpenAiSpeechExtension {
    config: OpenAiSpeechConfig,
}

impl OpenAiSpeechExtension {
    pub fn new(api_key: Option<String>) -> Self {
        Self {
            config: OpenAiSpeechConfig::new(api_key),
        }
    }

    pub fn with_config(config: OpenAiSpeechConfig) -> Self {
        Self { config }
    }
}

impl RoderExtension for OpenAiSpeechExtension {
    fn manifest(&self) -> ExtensionManifest {
        let mut required_capabilities = vec![CapabilityRequest::new("network.api.openai.com")];
        if self.config.api_key.is_some() {
            required_capabilities.push(CapabilityRequest::new("secret.read.OPENAI_API_KEY"));
        }
        ExtensionManifest {
            id: "roder-ext-openai-speech".to_string(),
            name: "OpenAI Speech".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("OpenAI speech-to-text transcription provider".to_string()),
            provides: vec![ProvidedService::SpeechTranscriber(
                OPENAI_SPEECH_PROVIDER_ID.to_string(),
            )],
            required_capabilities,
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.speech_transcriber(Arc::new(OpenAiSpeechTranscriber::new(self.config.clone())));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use roder_api::extension::RoderExtension;

    use super::*;

    #[test]
    fn manifest_exposes_openai_speech_transcriber_without_key() {
        let manifest = OpenAiSpeechExtension::new(None).manifest();

        assert_eq!(
            manifest.provides,
            vec![ProvidedService::SpeechTranscriber(
                OPENAI_SPEECH_PROVIDER_ID.to_string()
            )]
        );
        assert!(
            !manifest
                .required_capabilities
                .iter()
                .any(|capability| capability.id == "secret.read.OPENAI_API_KEY")
        );
    }

    #[test]
    fn manifest_requests_secret_capability_when_key_is_configured() {
        let manifest = OpenAiSpeechExtension::new(Some("secret".to_string())).manifest();

        assert!(
            manifest
                .required_capabilities
                .iter()
                .any(|capability| capability.id == "secret.read.OPENAI_API_KEY")
        );
    }
}
