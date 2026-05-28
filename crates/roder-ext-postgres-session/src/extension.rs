use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;

use crate::store::{PostgresSessionConfig, PostgresSessionStoreFactory};

pub struct PostgresSessionExtension {
    config: PostgresSessionConfig,
}

impl PostgresSessionExtension {
    pub fn new(config: PostgresSessionConfig) -> Self {
        Self { config }
    }
}

impl RoderExtension for PostgresSessionExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-postgres-session".to_string(),
            name: "PostgreSQL Session Store".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some(
                "Tenant-scoped PostgreSQL thread and artifact persistence".to_string(),
            ),
            provides: vec![ProvidedService::ThreadStore("postgres-session".to_string())],
            required_capabilities: vec![CapabilityRequest::new("network.postgres")],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.thread_store_factory(Arc::new(PostgresSessionStoreFactory {
            config: self.config.clone(),
        }));
        Ok(())
    }
}
