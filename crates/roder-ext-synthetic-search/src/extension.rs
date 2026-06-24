use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;

use crate::client::SyntheticSearchConfig;
use crate::tool::SyntheticSearchContributor;

pub struct SyntheticSearchExtension {
    config: SyntheticSearchConfig,
}

impl SyntheticSearchExtension {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            config: SyntheticSearchConfig::new(api_key),
        }
    }

    pub fn with_config(config: SyntheticSearchConfig) -> Self {
        Self { config }
    }
}

impl RoderExtension for SyntheticSearchExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-synthetic-search".to_string(),
            name: "Synthetic Search".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("Synthetic web search tool provider".to_string()),
            provides: vec![ProvidedService::ToolProvider(
                "synthetic-search".to_string(),
            )],
            required_capabilities: vec![
                CapabilityRequest::new("network.api.synthetic.new"),
                CapabilityRequest::new("secret.read.SYNTHETIC_API_KEY"),
            ],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.tool_contributor(Arc::new(SyntheticSearchContributor::new(
            self.config.clone(),
        )?));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use roder_api::extension::{ExtensionRegistryBuilder, ProvidedService, RoderExtension};

    use super::*;

    #[test]
    fn manifest_declares_synthetic_tool_provider() {
        let extension = SyntheticSearchExtension::new("test-key");
        let manifest = extension.manifest();

        assert_eq!(manifest.id, "roder-ext-synthetic-search");
        assert_eq!(
            manifest.provides,
            vec![ProvidedService::ToolProvider(
                "synthetic-search".to_string()
            )]
        );
        assert!(
            manifest
                .required_capabilities
                .iter()
                .any(|capability| capability.id == "network.api.synthetic.new")
        );
        assert!(
            manifest
                .required_capabilities
                .iter()
                .any(|capability| capability.id == "secret.read.SYNTHETIC_API_KEY")
        );
    }

    #[test]
    fn installs_tool_contributor() {
        let extension = SyntheticSearchExtension::new("test-key");
        let mut builder = ExtensionRegistryBuilder::new();

        extension.install(&mut builder).unwrap();
        let registry = builder.build().unwrap();

        assert_eq!(registry.tools.len(), 1);
        assert_eq!(registry.tools[0].id(), "synthetic-search");
    }
}
