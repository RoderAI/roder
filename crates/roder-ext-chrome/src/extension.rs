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
            provides: {
                let mut tools = vec![ProvidedService::ToolProvider("chrome".into())];
                if std::env::var_os("RODER_COMPUTER_USE_CDP_URL").is_some() {
                    tools.push(ProvidedService::ToolProvider("computer".into()));
                }
                tools
            },
            required_capabilities: vec![
                CapabilityRequest::new("network.web"),
                CapabilityRequest::new("fs.readwrite.roder-home"),
            ],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.tool_contributor(Arc::new(ChromeToolContributor::new()));
        if let Some(binding) = crate::computer::ComputerCdpBinding::from_env()? {
            registry.tool_contributor(Arc::new(crate::ComputerToolContributor::new(Arc::new(
                binding,
            ))));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn manifest_provides_chrome_and_native_computer_tools() {
        let _lock = ENV.lock().unwrap();
        let manifest = ChromeExtension.manifest();

        assert!(
            manifest
                .provides
                .contains(&ProvidedService::ToolProvider("chrome".to_string()))
        );
    }

    #[test]
    fn extension_installs_into_registry() {
        let _lock = ENV.lock().unwrap();
        let mut builder = ExtensionRegistryBuilder::new();
        builder.install(ChromeExtension::new()).expect("install");
        let registry = builder.build().expect("build");
        assert!(
            registry
                .provided_services()
                .contains(&ProvidedService::ToolProvider("chrome".to_string()))
        );
    }
    #[test]
    fn configured_native_computer_is_declared_and_installed() {
        let _lock = ENV.lock().unwrap();
        let names = ["RODER_COMPUTER_USE_CDP_URL", "RODER_COMPUTER_USE_URL"];
        let old = names.map(std::env::var_os);
        unsafe {
            std::env::set_var(names[0], "http://127.0.0.1:1");
            std::env::set_var(names[1], "https://example.com");
        }
        let mut builder = ExtensionRegistryBuilder::new();
        let result = builder
            .install(ChromeExtension)
            .and_then(|_| builder.build());
        unsafe {
            for (name, value) in names.into_iter().zip(old) {
                match value {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }
        let registry = result.expect("configured native computer installs");
        assert!(
            registry
                .provided_services()
                .contains(&ProvidedService::ToolProvider("computer".into()))
        );
    }
}
