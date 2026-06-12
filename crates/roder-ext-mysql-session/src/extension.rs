use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;

use crate::store::{MysqlSessionConfig, MysqlSessionStoreFactory};

pub struct MysqlSessionExtension {
    config: MysqlSessionConfig,
}

impl MysqlSessionExtension {
    pub fn new(config: MysqlSessionConfig) -> Self {
        Self { config }
    }
}

impl RoderExtension for MysqlSessionExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-mysql-session".to_string(),
            name: "MySQL Session Store".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("Tenant-scoped MySQL thread and artifact persistence".to_string()),
            provides: vec![ProvidedService::ThreadStore("mysql-session".to_string())],
            required_capabilities: vec![CapabilityRequest::new("network.mysql")],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.thread_store_factory(Arc::new(MysqlSessionStoreFactory {
            config: self.config.clone(),
        }));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use roder_api::extension::{ExtensionRegistryBuilder, ProvidedService, RoderExtension};

    use super::*;
    use crate::store::MysqlSessionConfig;

    #[test]
    fn manifest_declares_mysql_thread_store() {
        let extension = MysqlSessionExtension::new(
            MysqlSessionConfig::new("mysql://u:p@localhost/db", "tenant").unwrap(),
        );
        let manifest = extension.manifest();

        assert_eq!(manifest.id, "roder-ext-mysql-session");
        assert_eq!(
            manifest.provides,
            vec![ProvidedService::ThreadStore("mysql-session".to_string())]
        );
    }

    #[test]
    fn installs_thread_store_factory() {
        let extension = MysqlSessionExtension::new(
            MysqlSessionConfig::new("mysql://u:p@localhost/db", "tenant").unwrap(),
        );
        let mut builder = ExtensionRegistryBuilder::new();

        extension.install(&mut builder).unwrap();
        let registry = builder.build().unwrap();

        assert_eq!(registry.thread_stores.len(), 1);
        assert_eq!(registry.thread_stores[0].id(), "mysql-session");
    }
}
