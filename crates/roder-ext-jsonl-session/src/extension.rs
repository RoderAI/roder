use semver::Version;
use roder_api::extension::{ExtensionManifest, ExtensionRegistryBuilder, RoderExtension};
use std::path::PathBuf;
use std::sync::Arc;
use crate::store::JsonlSessionStoreFactory;

pub struct JsonlSessionExtension {
    base_path: PathBuf,
}

impl JsonlSessionExtension {
    pub fn new(base_path: PathBuf) -> Self {
        Self { base_path }
    }
}

impl RoderExtension for JsonlSessionExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-jsonl-session".to_string(),
            name: "JSONL Session Store".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("Append-only JSONL session persistence".to_string()),
            provides: vec![],
            required_capabilities: vec![],
        }
    }

    fn install(
        &self,
        registry: &mut ExtensionRegistryBuilder,
    ) -> anyhow::Result<()> {
        registry.session_store_factory(Arc::new(JsonlSessionStoreFactory {
            base_path: self.base_path.clone(),
        }));
        Ok(())
    }
}
