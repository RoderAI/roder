//! Markdown-file-based knowledge engine (roadmap phase 93, first slice).
//!
//! Stores project knowledge documents as plain markdown files with YAML front
//! matter under the Roder home, registers `knowledge_*` tools, and injects
//! relevant knowledge into turns. Future engines (e.g. an embedding-backed
//! "brain" with automatic reconciliation) plug in through the same
//! `KnowledgeStore` contract in `roder-api`.

use std::path::PathBuf;
use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use roder_api::knowledge::KnowledgeStoreFactory;
use semver::Version;

pub mod context;
pub mod document;
pub mod store;
pub mod tools;

pub use context::KnowledgeContextProvider;
pub use store::{MarkdownKnowledgeStore, MarkdownKnowledgeStoreFactory, STORE_ID};

pub struct KnowledgeMdExtension {
    base_path: PathBuf,
    recall_limit: usize,
    recall_enabled: bool,
}

impl KnowledgeMdExtension {
    pub fn new(base_path: PathBuf) -> Self {
        Self {
            base_path,
            recall_limit: 4,
            recall_enabled: true,
        }
    }

    pub fn with_recall(mut self, enabled: bool, limit: usize) -> Self {
        self.recall_enabled = enabled;
        self.recall_limit = limit.max(1);
        self
    }
}

impl RoderExtension for KnowledgeMdExtension {
    fn manifest(&self) -> ExtensionManifest {
        let mut provides = vec![
            ProvidedService::KnowledgeStore(STORE_ID.to_string()),
            ProvidedService::ToolProvider("knowledge-tools".to_string()),
        ];
        if self.recall_enabled {
            provides.push(ProvidedService::ContextProvider(
                "knowledge-context".to_string(),
            ));
        }
        ExtensionManifest {
            id: "roder-ext-knowledge-md".to_string(),
            name: "Markdown Knowledge Base".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some(
                "Markdown-file-based project knowledge base engine".to_string(),
            ),
            provides,
            required_capabilities: vec![CapabilityRequest::new("fs.readwrite.roder-home")],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        let factory = Arc::new(MarkdownKnowledgeStoreFactory::new(self.base_path.clone()));
        let store = factory.create();
        registry.knowledge_store_factory(factory);
        if self.recall_enabled {
            registry.context_provider(Arc::new(
                KnowledgeContextProvider::new(store.clone()).with_recall_limit(self.recall_limit),
            ));
        }
        registry.tool_contributor(Arc::new(tools::KnowledgeToolContributor::new(store)));
        Ok(())
    }
}

pub fn extension(base_path: PathBuf) -> KnowledgeMdExtension {
    KnowledgeMdExtension::new(base_path)
}
