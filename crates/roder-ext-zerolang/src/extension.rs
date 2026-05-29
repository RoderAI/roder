use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;

use crate::tools::ZerolangToolContributor;
use crate::types::ZerolangConfig;

#[derive(Debug, Clone, Default)]
pub struct ZerolangExtension {
    config: ZerolangConfig,
}

impl ZerolangExtension {
    pub fn new(config: ZerolangConfig) -> Self {
        Self { config }
    }
}

impl RoderExtension for ZerolangExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-zerolang".to_string(),
            name: "Zerolang Checked Graph Edits".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some(
                "Zero language graph inspection, validation, and checked edit tools.".to_string(),
            ),
            provides: vec![ProvidedService::ToolProvider("zerolang".to_string())],
            required_capabilities: vec![
                CapabilityRequest::new("fs.read.workspace"),
                CapabilityRequest::new("fs.write.workspace"),
                CapabilityRequest::new("process.spawn.shell"),
            ],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.tool_contributor(Arc::new(ZerolangToolContributor::new(self.config.clone())));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use roder_api::extension::{ProvidedService, RoderExtension};

    use super::*;

    #[test]
    fn manifest_exposes_zerolang_tool_provider() {
        let manifest = ZerolangExtension::default().manifest();

        assert!(
            manifest
                .provides
                .contains(&ProvidedService::ToolProvider("zerolang".to_string()))
        );
    }
}
