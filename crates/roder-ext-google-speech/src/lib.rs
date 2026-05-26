use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;

mod provider;

pub use provider::{GOOGLE_SPEECH_PROVIDER_ID, GoogleSpeechConfig, GoogleSpeechTranscriber};

#[derive(Debug, Clone)]
pub struct GoogleSpeechExtension {
    config: GoogleSpeechConfig,
}

impl GoogleSpeechExtension {
    pub fn new(config: GoogleSpeechConfig) -> Self {
        Self { config }
    }
}

impl RoderExtension for GoogleSpeechExtension {
    fn manifest(&self) -> ExtensionManifest {
        let mut required_capabilities =
            vec![CapabilityRequest::new("network.speech.googleapis.com")];
        if self.config.access_token.is_some() {
            required_capabilities.push(CapabilityRequest::new(
                "secret.read.RODER_GOOGLE_SPEECH_ACCESS_TOKEN",
            ));
        }
        ExtensionManifest {
            id: "roder-ext-google-speech".to_string(),
            name: "Google Speech".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("Google Cloud Speech-to-Text transcription provider".to_string()),
            provides: vec![ProvidedService::SpeechTranscriber(
                GOOGLE_SPEECH_PROVIDER_ID.to_string(),
            )],
            required_capabilities,
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.speech_transcriber(Arc::new(GoogleSpeechTranscriber::new(self.config.clone())));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use roder_api::extension::RoderExtension;

    use super::*;

    #[test]
    fn manifest_exposes_google_speech_transcriber_without_credentials() {
        let manifest = GoogleSpeechExtension::new(GoogleSpeechConfig::default()).manifest();

        assert_eq!(
            manifest.provides,
            vec![ProvidedService::SpeechTranscriber(
                GOOGLE_SPEECH_PROVIDER_ID.to_string()
            )]
        );
        assert!(
            !manifest
                .required_capabilities
                .iter()
                .any(|capability| capability.id == "secret.read.RODER_GOOGLE_SPEECH_ACCESS_TOKEN")
        );
    }
}
