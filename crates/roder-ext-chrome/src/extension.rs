use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;

use crate::tools::ChromeToolContributor;

/// The Roder Chrome browser-control extension. Registers the model-facing
/// `chrome_*` tools, bound to the live process browser bridge.
pub struct ChromeExtension;

impl ChromeExtension {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ChromeExtension {
    fn default() -> Self {
        Self::new()
    }
}

impl RoderExtension for ChromeExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-chrome".to_string(),
            name: "Chrome Browser Control".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some(
                "Inspect, control, debug, and record the user's live Chrome session through the \
                 Roder browser extension bridge."
                    .to_string(),
            ),
            provides: vec![ProvidedService::ToolProvider("chrome".to_string())],
            required_capabilities: vec![
                CapabilityRequest::new("network.web"),
                CapabilityRequest::new("fs.readwrite.roder-home"),
            ],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.tool_contributor(Arc::new(ChromeToolContributor::new()));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_provides_chrome_tools() {
        let manifest = ChromeExtension.manifest();
        assert!(
            manifest
                .provides
                .contains(&ProvidedService::ToolProvider("chrome".to_string()))
        );
    }

    #[test]
    fn extension_installs_into_registry() {
        let mut builder = ExtensionRegistryBuilder::new();
        builder.install(ChromeExtension::new()).expect("install");
        let registry = builder.build().expect("build");
        assert!(
            registry
                .provided_services()
                .contains(&ProvidedService::ToolProvider("chrome".to_string()))
        );
    }
}
