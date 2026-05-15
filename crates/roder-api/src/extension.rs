use std::sync::Arc;

use semver::Version;
use serde::{Deserialize, Serialize};

pub type ExtensionId = String;
pub type ApiVersion = String;
pub type InferenceEngineId = String;
pub type ContextProviderId = String;
pub type ContextPlannerId = String;
pub type SessionStoreId = String;
pub type CheckpointStoreId = String;
pub type MemoryStoreId = String;
pub type ToolProviderId = String;
pub type SubagentDispatcherId = String;
pub type PolicyContributorId = String;
pub type EventSinkId = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapabilityRequest {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProvidedService {
    InferenceEngine(InferenceEngineId),
    ContextProvider(ContextProviderId),
    ContextPlanner(ContextPlannerId),
    SessionStore(SessionStoreId),
    CheckpointStore(CheckpointStoreId),
    MemoryStore(MemoryStoreId),
    ToolProvider(ToolProviderId),
    SubagentDispatcher(SubagentDispatcherId),
    PolicyContributor(PolicyContributorId),
    EventSink(EventSinkId),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionManifest {
    pub id: ExtensionId,
    pub name: String,
    pub version: Version,
    pub api_version: ApiVersion,
    pub description: Option<String>,
    pub provides: Vec<ProvidedService>,
    pub required_capabilities: Vec<CapabilityRequest>,
}

pub trait RoderExtension: Send + Sync + 'static {
    fn manifest(&self) -> ExtensionManifest;

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()>;
}

#[derive(Clone)]
pub struct ExtensionRegistry {
    pub manifests: Vec<ExtensionManifest>,
    pub inference_engines: Vec<Arc<dyn crate::inference::InferenceEngine>>,
    pub context_providers: Vec<Arc<dyn crate::context::ContextProvider>>,
    pub context_planners: Vec<Arc<dyn crate::context::ContextPlanner>>,
    pub session_stores: Vec<Arc<dyn crate::session::SessionStoreFactory>>,
    pub checkpoint_stores: Vec<Arc<dyn crate::session::CheckpointStoreFactory>>,
    pub memory_stores: Vec<Arc<dyn crate::memory::MemoryStoreFactory>>,
    pub tools: Vec<Arc<dyn crate::tools::ToolContributor>>,
    pub subagent_dispatchers: Vec<Arc<dyn crate::subagents::SubagentDispatcher>>,
    pub policy_contributors: Vec<Arc<dyn crate::context::PolicyContributor>>,
    pub event_sinks: Vec<Arc<dyn crate::extension::EventSink>>,
}

impl ExtensionRegistry {
    pub fn inference_engine(&self, id: &str) -> Option<Arc<dyn crate::inference::InferenceEngine>> {
        self.inference_engines
            .iter()
            .find(|engine| engine.id() == id)
            .cloned()
    }

    pub fn default_inference_engine(&self) -> Option<Arc<dyn crate::inference::InferenceEngine>> {
        self.inference_engines.first().cloned()
    }

    pub fn provided_services(&self) -> Vec<ProvidedService> {
        self.manifests
            .iter()
            .flat_map(|manifest| manifest.provides.iter().cloned())
            .collect()
    }

    pub fn subagent_dispatcher(
        &self,
        id: &str,
    ) -> Option<Arc<dyn crate::subagents::SubagentDispatcher>> {
        self.subagent_dispatchers
            .iter()
            .find(|dispatcher| dispatcher.id() == id)
            .cloned()
    }
}

pub struct ExtensionRegistryBuilder {
    manifests: Vec<ExtensionManifest>,
    pub inference_engines: Vec<Arc<dyn crate::inference::InferenceEngine>>,
    pub context_providers: Vec<Arc<dyn crate::context::ContextProvider>>,
    pub context_planners: Vec<Arc<dyn crate::context::ContextPlanner>>,
    pub session_stores: Vec<Arc<dyn crate::session::SessionStoreFactory>>,
    pub checkpoint_stores: Vec<Arc<dyn crate::session::CheckpointStoreFactory>>,
    pub memory_stores: Vec<Arc<dyn crate::memory::MemoryStoreFactory>>,
    pub tools: Vec<Arc<dyn crate::tools::ToolContributor>>,
    pub subagent_dispatchers: Vec<Arc<dyn crate::subagents::SubagentDispatcher>>,
    pub policy_contributors: Vec<Arc<dyn crate::context::PolicyContributor>>,
    pub event_sinks: Vec<Arc<dyn crate::extension::EventSink>>,
}

impl Default for ExtensionRegistryBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ExtensionRegistryBuilder {
    pub fn new() -> Self {
        Self {
            manifests: Vec::new(),
            inference_engines: Vec::new(),
            context_providers: Vec::new(),
            context_planners: Vec::new(),
            session_stores: Vec::new(),
            checkpoint_stores: Vec::new(),
            memory_stores: Vec::new(),
            tools: Vec::new(),
            subagent_dispatchers: Vec::new(),
            policy_contributors: Vec::new(),
            event_sinks: Vec::new(),
        }
    }

    pub fn install<E: RoderExtension>(&mut self, ext: E) -> anyhow::Result<()> {
        let manifest = ext.manifest();
        if self
            .manifests
            .iter()
            .any(|existing| existing.id == manifest.id)
        {
            anyhow::bail!("extension {} is already installed", manifest.id);
        }
        ext.install(self)?;
        self.manifests.push(manifest);
        Ok(())
    }

    pub fn build(self) -> anyhow::Result<ExtensionRegistry> {
        Ok(ExtensionRegistry {
            manifests: self.manifests,
            inference_engines: self.inference_engines,
            context_providers: self.context_providers,
            context_planners: self.context_planners,
            session_stores: self.session_stores,
            checkpoint_stores: self.checkpoint_stores,
            memory_stores: self.memory_stores,
            tools: self.tools,
            subagent_dispatchers: self.subagent_dispatchers,
            policy_contributors: self.policy_contributors,
            event_sinks: self.event_sinks,
        })
    }

    pub fn manifest(&mut self, manifest: ExtensionManifest) {
        self.manifests.push(manifest);
    }

    pub fn inference_engine(&mut self, engine: Arc<dyn crate::inference::InferenceEngine>) {
        self.inference_engines.push(engine);
    }

    pub fn context_provider(&mut self, provider: Arc<dyn crate::context::ContextProvider>) {
        self.context_providers.push(provider);
    }

    pub fn context_planner(&mut self, planner: Arc<dyn crate::context::ContextPlanner>) {
        self.context_planners.push(planner);
    }

    pub fn session_store_factory(&mut self, store: Arc<dyn crate::session::SessionStoreFactory>) {
        self.session_stores.push(store);
    }

    pub fn checkpoint_store_factory(
        &mut self,
        store: Arc<dyn crate::session::CheckpointStoreFactory>,
    ) {
        self.checkpoint_stores.push(store);
    }

    pub fn memory_store_factory(&mut self, store: Arc<dyn crate::memory::MemoryStoreFactory>) {
        self.memory_stores.push(store);
    }

    pub fn tool_contributor(&mut self, contributor: Arc<dyn crate::tools::ToolContributor>) {
        self.tools.push(contributor);
    }

    pub fn subagent_dispatcher(
        &mut self,
        dispatcher: Arc<dyn crate::subagents::SubagentDispatcher>,
    ) {
        self.subagent_dispatchers.push(dispatcher);
    }

    pub fn policy_contributor(&mut self, contributor: Arc<dyn crate::context::PolicyContributor>) {
        self.policy_contributors.push(contributor);
    }

    pub fn event_sink(&mut self, sink: Arc<dyn crate::extension::EventSink>) {
        self.event_sinks.push(sink);
    }
}

#[async_trait::async_trait]
pub trait EventSink: Send + Sync + 'static {
    async fn handle_event(&self, envelope: &crate::events::EventEnvelope) -> anyhow::Result<()>;
}
