use std::fmt;
use std::sync::Arc;

use anyhow::Context;
use roder_api::capabilities::CapabilityRequest;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use roder_api::memory::MemoryStoreFactory;
use roder_ext_memory::MemoryContextProvider;
use roder_ext_memory::tools::MemoryToolContributor;
use semver::Version;

mod client;
mod store;

pub use store::{HonchoMemoryStore, HonchoMemoryStoreFactory, STORE_ID};

pub const API_KEY_ENV: &str = "HONCHO_API_KEY";
pub const BASE_URL_ENV: &str = "HONCHO_BASE_URL";
pub const WORKSPACE_ID_ENV: &str = "HONCHO_WORKSPACE_ID";
pub const PEER_ID_ENV: &str = "HONCHO_PEER_ID";
pub const SESSION_ID_ENV: &str = "HONCHO_SESSION_ID";
pub const LIVE_ENV: &str = "RODER_LIVE_HONCHO";

pub const DEFAULT_BASE_URL: &str = "https://api.honcho.dev";
pub const DEFAULT_PEER_ID: &str = "roder-memory";

/// Connection settings for a Honcho-backed memory store. The api key is
/// resolved from the environment and must never be persisted to config files.
#[derive(Clone)]
pub struct HonchoMemoryConfig {
    pub api_key: String,
    pub base_url: String,
    pub workspace_id: String,
    /// Peer that authors memory messages in Honcho.
    pub peer_id: String,
    /// When set, every scope writes into this single Honcho session instead
    /// of one derived session per scope; embedding hosts use this to pin a
    /// runtime to a session of their choosing.
    pub session_id: Option<String>,
}

impl fmt::Debug for HonchoMemoryConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HonchoMemoryConfig")
            .field("api_key", &"<redacted>")
            .field("base_url", &self.base_url)
            .field("workspace_id", &self.workspace_id)
            .field("peer_id", &self.peer_id)
            .field("session_id", &self.session_id)
            .finish()
    }
}

impl HonchoMemoryConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        let api_key = env_nonempty(API_KEY_ENV)
            .with_context(|| format!("honcho memory store requires {API_KEY_ENV}"))?;
        let workspace_id = env_nonempty(WORKSPACE_ID_ENV)
            .with_context(|| format!("honcho memory store requires {WORKSPACE_ID_ENV}"))?;
        Ok(Self {
            api_key,
            base_url: env_nonempty(BASE_URL_ENV)
                .ok()
                .unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
            workspace_id,
            peer_id: env_nonempty(PEER_ID_ENV)
                .ok()
                .unwrap_or_else(|| DEFAULT_PEER_ID.to_string()),
            session_id: env_nonempty(SESSION_ID_ENV).ok(),
        })
    }
}

fn env_nonempty(name: &str) -> anyhow::Result<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .with_context(|| format!("{name} is not set"))
}

pub struct HonchoMemoryExtension {
    config: HonchoMemoryConfig,
}

impl HonchoMemoryExtension {
    pub fn new(config: HonchoMemoryConfig) -> Self {
        Self { config }
    }
}

impl RoderExtension for HonchoMemoryExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-honcho".to_string(),
            name: "Honcho Memory".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some(
                "Honcho-backed memory store; retrieval is delegated to Honcho's hosted semantic search"
                    .to_string(),
            ),
            provides: vec![
                ProvidedService::MemoryStore(STORE_ID.to_string()),
                ProvidedService::ContextProvider("memory-context".to_string()),
                ProvidedService::ToolProvider("memory-tools".to_string()),
            ],
            required_capabilities: vec![
                CapabilityRequest::new("network.http"),
                CapabilityRequest::new(format!("secret.read.{API_KEY_ENV}")),
            ],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        let factory = Arc::new(HonchoMemoryStoreFactory::new(self.config.clone()));
        let store = factory.create();
        registry.memory_store_factory(factory);
        registry.context_provider(Arc::new(MemoryContextProvider::new(store.clone())));
        registry.tool_contributor(Arc::new(MemoryToolContributor::new(store)));
        Ok(())
    }
}

pub fn extension(config: HonchoMemoryConfig) -> HonchoMemoryExtension {
    HonchoMemoryExtension::new(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_redacts_api_key() {
        let config = HonchoMemoryConfig {
            api_key: "super-secret".to_string(),
            base_url: DEFAULT_BASE_URL.to_string(),
            workspace_id: "ws".to_string(),
            peer_id: DEFAULT_PEER_ID.to_string(),
            session_id: None,
        };
        let debug = format!("{config:?}");
        assert!(!debug.contains("super-secret"));
        assert!(debug.contains("<redacted>"));
    }
}
