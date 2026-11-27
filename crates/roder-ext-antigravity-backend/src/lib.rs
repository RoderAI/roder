//! Antigravity subscriptions through the official `agy acp` agent runtime.
mod backend;
mod history;
mod mapping;
pub mod runtime;
mod transport;

use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;

pub use backend::AntigravityBackend;
pub use transport::clear_api_key_environment;

pub struct AntigravityBackendExtension;

impl RoderExtension for AntigravityBackendExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-antigravity-backend".into(),
            name: "Antigravity Agent Backend".into(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".into(),
            description: Some("Google Antigravity subscription via official ACP".into()),
            provides: vec![ProvidedService::AgentBackend("antigravity".into())],
            required_capabilities: vec![CapabilityRequest::new("process.agy")],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.agent_backend(Arc::new(AntigravityBackend::default()));
        Ok(())
    }
}
