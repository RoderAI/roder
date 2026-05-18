use std::path::PathBuf;
use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use roder_api::memory::MemoryStoreFactory;
use semver::Version;

pub mod context;
pub mod jobs;
pub mod schema;
pub mod scopes;
pub mod sqlite;
pub mod tools;
pub mod vector;

pub use context::MemoryContextProvider;
pub use sqlite::{SqliteMemoryStore, SqliteMemoryStoreFactory};

pub struct MemoryExtension {
    base_path: PathBuf,
}

impl MemoryExtension {
    pub fn new(base_path: PathBuf) -> Self {
        Self { base_path }
    }
}

impl RoderExtension for MemoryExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-memory".to_string(),
            name: "Local Memory".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("SQLite-backed project and global memory store".to_string()),
            provides: vec![
                ProvidedService::MemoryStore("sqlite-memory".to_string()),
                ProvidedService::ContextProvider("memory-context".to_string()),
                ProvidedService::ToolProvider("memory-tools".to_string()),
            ],
            required_capabilities: vec![CapabilityRequest::new("fs.readwrite.roder-home")],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        let factory = Arc::new(SqliteMemoryStoreFactory::new(self.base_path.clone()));
        let store = factory.create();
        registry.memory_store_factory(factory);
        registry.context_provider(Arc::new(MemoryContextProvider::new(store.clone())));
        registry.tool_contributor(Arc::new(tools::MemoryToolContributor::new(store)));
        Ok(())
    }
}

pub fn extension(base_path: PathBuf) -> MemoryExtension {
    MemoryExtension::new(base_path)
}
