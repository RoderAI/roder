mod agent_def;
mod dispatcher;
mod loader;
mod transcript;

use std::path::PathBuf;
use std::sync::Arc;

use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use roder_api::subagents::SubagentDispatcher;
use semver::Version;

pub use agent_def::{AgentDefinitionSource, parse_agent_definition};
pub use dispatcher::{InProcessDispatcher, InProcessDispatcherConfig, InferenceEngineRegistry};
pub use loader::{AgentLoadConfig, load_agent_definitions};
pub use transcript::{BoundedTranscript, truncate_text};

pub struct SubagentsExtension {
    dispatcher: Arc<InProcessDispatcher>,
}

impl SubagentsExtension {
    pub fn new(dispatcher: Arc<InProcessDispatcher>) -> Self {
        Self { dispatcher }
    }
}

impl RoderExtension for SubagentsExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-subagents".to_string(),
            name: "In-process subagents".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("In-process subagent dispatcher and disk agent loader".to_string()),
            provides: vec![ProvidedService::SubagentDispatcher(self.dispatcher.id())],
            required_capabilities: vec![],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.subagent_dispatcher(self.dispatcher.clone());
        Ok(())
    }
}

pub fn extension(dispatcher: Arc<InProcessDispatcher>) -> SubagentsExtension {
    SubagentsExtension::new(dispatcher)
}

pub fn default_user_agent_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".roder").join("agents"))
}
