//! Rift copy-on-write fork provider (roadmap phase 81, Task 4).
//!
//! Rift (<https://github.com/anomalyco/rift>) is an experimental
//! copy-on-write snapshot tool (APFS `clonefile`, btrfs snapshots,
//! reflinks). Because its CLI is pre-1.0 and explicitly unstable, Roder
//! depends on a narrow adapter boundary: this crate shells out to a
//! configured `rift` binary with a small documented command contract and
//! maps failures into typed errors. Offline tests run against a fake
//! `rift` script; real-binary checks are opt-in via `RODER_RIFT_LIVE=1`
//! and `RIFT_BIN`.

pub mod cli;
pub mod config;
pub mod errors;
pub mod provider;

use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;

pub use config::RiftConfig;
pub use errors::RiftError;
pub use provider::{RIFT_FORK_PROVIDER_ID, RiftForkProvider};

#[derive(Debug, Default)]
pub struct RiftForkExtension {
    config: RiftConfig,
}

impl RiftForkExtension {
    pub fn new(config: RiftConfig) -> Self {
        Self { config }
    }
}

impl RoderExtension for RiftForkExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-fork-rift".to_string(),
            name: "Rift Fork Provider".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some(
                "Workspace forks backed by Rift copy-on-write snapshots.".to_string(),
            ),
            provides: vec![ProvidedService::ForkProvider(
                RIFT_FORK_PROVIDER_ID.to_string(),
            )],
            required_capabilities: vec![CapabilityRequest::new("process.exec.rift")],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.fork_provider(Arc::new(RiftForkProvider::new(self.config.clone())));
        Ok(())
    }
}
