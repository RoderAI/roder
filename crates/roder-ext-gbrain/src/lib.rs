//! Bi-temporal **gbrain** memory extension for roder.
//!
//! Implements the organizational-memory model the OrgMemBench paper specifies —
//! and that the gbrain system it benchmarks lacks: valid-time / transaction-time
//! facts (`valid_at`/`invalid_at`/`ingested_at`/`expired_at`), invalidate-never-
//! delete supersession with explicit reasons, `as_of` date-travel, and hybrid
//! (vector + lexical + graph) recall. Registers **additively** as a second
//! memory store with its own `gbrain_*` tools and context provider; the default
//! `sqlite-memory` store stays primary.

use std::path::PathBuf;
use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::embeddings::EmbeddingProvider;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;

pub mod agent;
pub mod context;
pub mod embed;
pub mod ground;
pub mod model;
pub mod reason;
pub mod render;
pub mod response_format;
pub mod retrieval;
pub mod schema;
pub mod store;
pub mod tools;

pub use agent::{AgentAnswer, AgentBudget, DecisionAgent, ProgressSink, WorkingContext};
pub use context::GbrainContextProvider;
pub use embed::Embedder;
pub use reason::{AnthropicReasoner, Reasoner};
pub use model::{AsOf, FactStatus, TemporalFact};
pub use store::{
    CaptureInput, ConsolidateStats, GbrainStore, GbrainStoreFactory, RecallParams, RecallResult,
};
pub use tools::GbrainToolContributor;

/// The bi-temporal gbrain memory extension.
pub struct GbrainExtension {
    base_path: PathBuf,
    embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
}

impl GbrainExtension {
    /// Create the extension rooted at `base_path` (e.g. `<roder_home>/gbrain`).
    /// Pass an [`EmbeddingProvider`] to enable real dense embeddings; `None`
    /// uses the deterministic local fallback.
    pub fn new(base_path: PathBuf, embedding_provider: Option<Arc<dyn EmbeddingProvider>>) -> Self {
        Self {
            base_path,
            embedding_provider,
        }
    }
}

impl RoderExtension for GbrainExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-gbrain".to_string(),
            name: "Bi-temporal gbrain Memory".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some(
                "Bi-temporal, hybrid-retrieval organizational memory store (valid/transaction \
                 time, supersession, as-of date-travel)."
                    .to_string(),
            ),
            provides: vec![
                ProvidedService::MemoryStore("gbrain-bitemporal".to_string()),
                ProvidedService::ContextProvider("gbrain-context".to_string()),
                ProvidedService::ToolProvider("gbrain-tools".to_string()),
            ],
            required_capabilities: vec![CapabilityRequest::new("fs.readwrite.roder-home")],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        // One shared concrete store for tools + context (they need the rich
        // bi-temporal API); the factory creates further generic handles to the
        // same file for the app-server's MemoryStore selection.
        let store = Arc::new(GbrainStore::open(
            self.base_path.join("gbrain.sqlite3"),
            Embedder::new(self.embedding_provider.clone()),
        )?);
        let factory = Arc::new(GbrainStoreFactory::new(
            self.base_path.clone(),
            self.embedding_provider.clone(),
        ));
        registry.memory_store_factory(factory);
        registry.context_provider(Arc::new(GbrainContextProvider::new(store.clone())));
        registry.tool_contributor(Arc::new(GbrainToolContributor::new(store)));
        Ok(())
    }
}

/// Convenience constructor mirroring other extensions.
pub fn extension(
    base_path: PathBuf,
    embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
) -> GbrainExtension {
    GbrainExtension::new(base_path, embedding_provider)
}
