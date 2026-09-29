//! Goal-directed Chrome CDP browser automation.
//!
//! Started as a Rust port of Jev Ultrafast (MIT, browser-use/jev-ultrafast)
//! at revision 1231850a. Jev owns its in-page scripts (`src/assets/`), prompts
//! and fixtures, and diverges from upstream on purpose where it settles,
//! reaches and names controls, hit-tests, fingerprints and retries; the README
//! lists every divergence. The agent loop, action space, decision contract
//! and text helper keep upstream's shape, pinned by fixtures first recorded
//! from the Python implementation and re-recorded deliberately where Jev
//! diverges.

#![doc = include_str!("../README.md")]

mod agent;
mod block;
mod cdp;
mod chrome;
mod decide;
mod effects;
mod engine;
#[cfg(test)]
mod fixture_harness;
mod http;
mod irreversible;
mod page;
mod policy;
mod prompts;
mod python_json;
mod report;
mod runner;
mod scope;
mod secret;
mod session;
mod space;
mod text_helper;
mod text_model;
mod tools;
mod usage;

use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;

pub use decide::JevTypeSafeDecisionClient;
pub use engine::{
    Covered, JevActOutcome, JevActionRecord, JevBrowser, JevControl, JevDecision,
    JevDecisionClient, JevDecisionRecord, JevDecisionTransport, JevDialog, JevEngine,
    JevEngineConfig, JevFrameText, JevPageFacts, JevRunResult, JevStatus, JevStop, JevTextValue,
    JevTextValueResolver, StaleObservation,
};
pub use scope::JevOriginScope;
pub use tools::{JevToolContributor, jev_tool_spec};
pub use usage::{JevBilled, JevCallUsage, JevTokenCount, JevUsage};

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
