//! Goal-directed Chrome CDP browser automation.
//!
//! A Rust port of Jev Ultrafast (MIT, browser-use/jev-ultrafast), pinned to
//! revision 1231850a. The two scripts that must run inside the page are
//! vendored verbatim under `src/assets/`; the agent loop, action space,
//! decision contract and text helper are ported, and parity with upstream is
//! pinned by fixtures recorded from the Python implementation.

mod agent;
mod cdp;
mod chrome;
mod decide;
mod page;
mod policy;
mod prompts;
mod python_json;
mod runner;
mod space;
mod text_helper;
mod text_model;
mod tools;

use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;

pub use tools::{JevToolContributor, jev_tool_spec};

pub struct JevExtension;

impl RoderExtension for JevExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-jev".into(),
            name: "Jev Ultrafast Browser".into(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".into(),
            description: Some("Goal-directed browser tasks through Jev and Chrome CDP.".into()),
            provides: vec![
                ProvidedService::ToolProvider("jev".into()),
                ProvidedService::PolicyContributor("jev".into()),
            ],
            required_capabilities: vec![
                CapabilityRequest::new("network.web"),
                CapabilityRequest::new("secret.read.JEV_API_KEY"),
            ],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.tool_contributor(Arc::new(JevToolContributor));
        registry.policy_contributor(Arc::new(policy::JevPolicy));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installs_tool_and_policy_services() {
        let mut builder = ExtensionRegistryBuilder::new();
        builder.install(JevExtension).unwrap();
        let registry = builder.build().unwrap();
        assert!(
            registry
                .provided_services()
                .contains(&ProvidedService::ToolProvider("jev".into()))
        );
        assert!(
            registry
                .provided_services()
                .contains(&ProvidedService::PolicyContributor("jev".into()))
        );
    }
}
