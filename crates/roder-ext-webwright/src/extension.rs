use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;

use crate::run::{WEBWRIGHT_TASK_EXECUTOR_ID, WebwrightTaskExecutor};
use crate::tools::WebwrightToolContributor;

pub struct WebwrightExtension;

impl RoderExtension for WebwrightExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-webwright".to_string(),
            name: "Webwright Browser Agent".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some(
                "Webwright-style browser automation task executor and helpers".to_string(),
            ),
            provides: vec![
                ProvidedService::TaskExecutor(WEBWRIGHT_TASK_EXECUTOR_ID.to_string()),
                ProvidedService::ToolProvider("webwright".to_string()),
            ],
            required_capabilities: vec![
                CapabilityRequest::new("fs.read.workspace"),
                CapabilityRequest::new("fs.write.workspace"),
                CapabilityRequest::new("fs.readwrite.roder-home"),
                CapabilityRequest::new("process.spawn.shell"),
                CapabilityRequest::new("network.web"),
            ],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.task_executor(Arc::new(WebwrightTaskExecutor::new()));
        registry.tool_contributor(Arc::new(WebwrightToolContributor));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use roder_api::extension::RoderExtension;

    use super::*;

    #[test]
    fn manifest_exposes_task_executor_and_tools() {
        let manifest = WebwrightExtension.manifest();
        assert!(manifest.provides.contains(&ProvidedService::TaskExecutor(
            WEBWRIGHT_TASK_EXECUTOR_ID.to_string()
        )));
        assert!(
            manifest
                .provides
                .contains(&ProvidedService::ToolProvider("webwright".to_string()))
        );
    }
}
