mod protocol;

use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;

pub use protocol::CodexBackend;

pub struct CodexBackendExtension;

impl RoderExtension for CodexBackendExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-codex-backend".into(),
            name: "Codex Agent Backend".into(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".into(),
            description: Some("Codex app-server agent runtime".into()),
            provides: vec![ProvidedService::AgentBackend("codex".into())],
            required_capabilities: vec![CapabilityRequest::new("process.codex")],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.agent_backend(Arc::new(CodexBackend::default()));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_registers_agent_backend() {
        let mut registry = ExtensionRegistryBuilder::new();
        registry.install(CodexBackendExtension).unwrap();
        registry.grant_capability(
            "roder-ext-codex-backend",
            roder_api::capabilities::CapabilityGrant::new("process.codex"),
        );
        assert!(registry.build().unwrap().agent_backend("codex").is_some());
    }
}
