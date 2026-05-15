use std::sync::Arc;

use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;

use crate::client::FirecrawlSearchConfig;
use crate::tool::FirecrawlSearchContributor;

pub struct FirecrawlSearchExtension {
    config: FirecrawlSearchConfig,
}

impl FirecrawlSearchExtension {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            config: FirecrawlSearchConfig::new(api_key),
        }
    }

    pub fn with_config(config: FirecrawlSearchConfig) -> Self {
        Self { config }
    }
}

impl RoderExtension for FirecrawlSearchExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-firecrawl-search".to_string(),
            name: "Firecrawl Search".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("Firecrawl web search tool provider".to_string()),
            provides: vec![ProvidedService::ToolProvider(
                "firecrawl-search".to_string(),
            )],
            required_capabilities: vec![],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.tool_contributor(Arc::new(FirecrawlSearchContributor::new(
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
    fn manifest_declares_firecrawl_tool_provider() {
        let extension = FirecrawlSearchExtension::new("test-key");
        let manifest = extension.manifest();

        assert_eq!(manifest.id, "roder-ext-firecrawl-search");
        assert_eq!(
            manifest.provides,
            vec![ProvidedService::ToolProvider(
                "firecrawl-search".to_string()
            )]
        );
    }

    #[test]
    fn installs_tool_contributor() {
        let extension = FirecrawlSearchExtension::new("test-key");
        let mut builder = ExtensionRegistryBuilder::new();

        extension.install(&mut builder).unwrap();
        let registry = builder.build().unwrap();

        assert_eq!(registry.tools.len(), 1);
        assert_eq!(registry.tools[0].id(), "firecrawl-search");
    }
}
