mod agent_def;
mod agent_swarm;
mod dispatcher;
mod loader;
mod tool;
mod trace;
mod transcript;

use std::path::PathBuf;
use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use roder_api::subagents::SubagentDispatcher;
use semver::Version;

pub use agent_def::{AgentDefinitionSource, parse_agent_definition};
pub use agent_swarm::{
    AGENT_SWARM_TOOL, AgentSwarmCancel, AgentSwarmChildLauncher, AgentSwarmChildRun,
    AgentSwarmProgressObserver, AgentSwarmTool, DispatcherChildLauncher, run_agent_swarm,
    run_agent_swarm_with_observer,
};
pub use dispatcher::{InProcessDispatcher, InProcessDispatcherConfig, InferenceEngineRegistry};
pub use loader::{AgentLoadConfig, load_agent_definitions};
pub use tool::{TaskTool, TaskToolConfig, TaskToolContributor, namespaced_task_tool_name};
pub use transcript::{BoundedTranscript, truncate_text};

pub struct SubagentsExtension {
    dispatcher: Arc<InProcessDispatcher>,
    task_tool_config: TaskToolConfig,
}

impl SubagentsExtension {
    pub fn new(dispatcher: Arc<InProcessDispatcher>) -> Self {
        Self {
            dispatcher,
            task_tool_config: TaskToolConfig::default(),
        }
    }

    pub fn with_task_tool_config(
        dispatcher: Arc<InProcessDispatcher>,
        task_tool_config: TaskToolConfig,
    ) -> Self {
        Self {
            dispatcher,
            task_tool_config,
        }
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
            provides: vec![
                ProvidedService::SubagentDispatcher(self.dispatcher.id()),
                ProvidedService::ToolProvider(self.task_tool_config.provider_id.clone()),
            ],
            required_capabilities: vec![CapabilityRequest::new("process.spawn.roder")],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.subagent_dispatcher(self.dispatcher.clone());
        registry.tool_contributor(Arc::new(TaskToolContributor::new(
            self.task_tool_config.clone(),
            self.dispatcher.clone(),
        )));
        Ok(())
    }
}

pub fn extension(dispatcher: Arc<InProcessDispatcher>) -> SubagentsExtension {
    SubagentsExtension::new(dispatcher)
}

pub fn default_user_agent_dir() -> Option<PathBuf> {
    std::env::var_os("RODER_DATA_DIR")
        .or_else(|| std::env::var_os("RODER_CONFIG_DIR"))
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".roder")))
        .map(|home| home.join("agents"))
}
