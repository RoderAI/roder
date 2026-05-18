use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;

use crate::client::TavilySearchConfig;
use crate::tool::TavilySearchContributor;

pub struct TavilySearchExtension {
    config: TavilySearchConfig,
}

impl TavilySearchExtension {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            config: TavilySearchConfig::new(api_key),
        }
    }

    pub fn with_config(config: TavilySearchConfig) -> Self {
        Self { config }
    }
}

impl RoderExtension for TavilySearchExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-tavily-search".to_string(),
            name: "Tavily Search".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("Tavily web search tool provider".to_string()),
            provides: vec![ProvidedService::ToolProvider("tavily-search".to_string())],
            required_capabilities: vec![
                CapabilityRequest::new("network.api.tavily.com"),
                CapabilityRequest::new("secret.read.TAVILY_API_KEY"),
            ],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.tool_contributor(Arc::new(TavilySearchContributor::new(self.config.clone())?));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use roder_api::extension::{ExtensionRegistryBuilder, ProvidedService, RoderExtension};

    use super::*;

    #[test]
    fn manifest_declares_tavily_tool_provider() {
        let extension = TavilySearchExtension::new("test-key");
        let manifest = extension.manifest();

        assert_eq!(manifest.id, "roder-ext-tavily-search");
        assert_eq!(
            manifest.provides,
            vec![ProvidedService::ToolProvider("tavily-search".to_string())]
        );
    }

    #[test]
    fn installs_tool_contributor() {
        let extension = TavilySearchExtension::new("test-key");
        let mut builder = ExtensionRegistryBuilder::new();

        extension.install(&mut builder).unwrap();
        let registry = builder.build().unwrap();

        assert_eq!(registry.tools.len(), 1);
        assert_eq!(registry.tools[0].id(), "tavily-search");
    }
}
