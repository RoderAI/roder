//! Process-global distribution-extension path. Lives in its own integration
//! test binary because `set_distribution_extensions` is set-once per process.

use std::sync::Arc;

use roder_api::extension::{ExtensionManifest, ExtensionRegistryBuilder, RoderExtension};
use roder_extension_host::{
    DefaultRegistryConfig, build_default_registry, distribution_extensions,
    set_distribution_extensions,
};
use semver::Version;

struct FakeDistributionExtension;

impl RoderExtension for FakeDistributionExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-test-distribution".to_string(),
            name: "Test Distribution Extension".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: None,
            provides: vec![],
            required_capabilities: vec![],
        }
    }

    fn install(&self, _registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        Ok(())
    }
}

#[test]
fn distribution_extensions_flow_into_default_registry() {
    set_distribution_extensions(vec![Arc::new(FakeDistributionExtension)]).unwrap();

    // Mirrors the roder-cli plumbing: the entry point folds the process-wide
    // list into the registry config.
    let registry = build_default_registry(DefaultRegistryConfig {
        extra_extensions: distribution_extensions(),
        ..Default::default()
    })
    .unwrap();

    assert!(
        registry
            .manifests
            .iter()
            .any(|manifest| manifest.id == "roder-ext-test-distribution")
    );

    let err = set_distribution_extensions(vec![]).unwrap_err();
    assert!(err.to_string().contains("already set"));
}
