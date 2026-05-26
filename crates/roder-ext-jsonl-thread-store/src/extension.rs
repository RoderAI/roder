use crate::store::JsonlThreadStoreFactory;
use roder_api::capabilities::CapabilityRequest;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;
use std::path::PathBuf;
use std::sync::Arc;

pub struct JsonlThreadStoreExtension {
    base_path: PathBuf,
}

impl JsonlThreadStoreExtension {
    pub fn new(base_path: PathBuf) -> Self {
        Self { base_path }
    }
}

impl RoderExtension for JsonlThreadStoreExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-jsonl-thread-store".to_string(),
            name: "JSONL Thread Store".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("Append-only JSONL thread persistence".to_string()),
            provides: vec![ProvidedService::ThreadStore(
                "jsonl-thread-store".to_string(),
            )],
            required_capabilities: vec![CapabilityRequest::new("fs.readwrite.roder-home")],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
            base_path: self.base_path.clone(),
        }));
        Ok(())
    }
}
