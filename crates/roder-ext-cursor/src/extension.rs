use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::catalog::PROVIDER_CURSOR;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;

use crate::provider::{CursorConfig, CursorInferenceEngine};

pub struct CursorExtension {
    config: CursorConfig,
}

impl CursorExtension {
    pub fn new(config: CursorConfig) -> Self {
        Self { config }
    }
}

impl RoderExtension for CursorExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-cursor-provider".to_string(),
            name: "Cursor Provider".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("Cursor Composer direct AgentService provider".to_string()),
            provides: vec![ProvidedService::InferenceEngine(
                PROVIDER_CURSOR.to_string(),
            )],
            required_capabilities: vec![
                CapabilityRequest::new("network.agentn.global.api5.cursor.sh"),
                CapabilityRequest::new("network.api2.cursor.sh"),
            ],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.inference_engine(Arc::new(CursorInferenceEngine::new(self.config.clone())));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use roder_api::extension::RoderExtension;

    use super::*;

    #[test]
    fn manifest_provides_cursor_engine() {
        let manifest = CursorExtension::new(CursorConfig::default()).manifest();
        assert_eq!(
            manifest.provides,
            vec![ProvidedService::InferenceEngine(
                PROVIDER_CURSOR.to_string()
            )]
        );
    }
}
