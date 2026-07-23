use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;

use crate::client::ParallelSearchConfig;
use crate::tool::ParallelSearchContributor;

pub struct ParallelSearchExtension {
    config: ParallelSearchConfig,
}

impl ParallelSearchExtension {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            config: ParallelSearchConfig::new(api_key),
        }
    }

    pub fn with_config(config: ParallelSearchConfig) -> Self {
        Self { config }
    }
}

impl RoderExtension for ParallelSearchExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-parallel-search".to_string(),
            name: "Parallel Search".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some(
                "Parallel.ai search and URL extract tool provider".to_string(),
            ),
            provides: vec![ProvidedService::ToolProvider("parallel-search".to_string())],
            required_capabilities: vec![CapabilityRequest::new("network.web")],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.tool_contributor(Arc::new(ParallelSearchContributor::new(
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
    fn manifest_declares_parallel_tool_provider() {
        let extension = ParallelSearchExtension::new("test-key");
        let manifest = extension.manifest();

        assert_eq!(manifest.id, "roder-ext-parallel-search");
        assert_eq!(
            manifest.provides,
            vec![ProvidedService::ToolProvider("parallel-search".to_string())]
        );
    }

    #[test]
    fn installs_search_and_extract_tools() {
        let extension = ParallelSearchExtension::new("test-key");
        let mut builder = ExtensionRegistryBuilder::new();

        extension.install(&mut builder).unwrap();
        let registry = builder.build().unwrap();

        assert_eq!(registry.tools.len(), 1);
        assert_eq!(registry.tools[0].id(), "parallel-search");

        let mut tool_registry = roder_api::tools::ToolRegistry::default();
        registry.tools[0].contribute(&mut tool_registry).unwrap();
        let names = tool_registry
            .specs()
            .into_iter()
            .map(|spec| spec.name)
            .collect::<Vec<_>>();
        assert!(names.iter().any(|name| name == "parallel_search"));
        assert!(names.iter().any(|name| name == "parallel_extract"));
    }
}
