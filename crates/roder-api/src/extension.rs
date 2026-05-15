use std::sync::Arc;
use serde::{Serialize, Deserialize};
use semver::Version;

pub type ExtensionId = String;
pub type ApiVersion = String;
pub type InferenceEngineId = String;
pub type ContextProviderId = String;
pub type ContextPlannerId = String;
pub type SessionStoreId = String;
pub type CheckpointStoreId = String;
pub type MemoryStoreId = String;
pub type ToolProviderId = String;
pub type PolicyContributorId = String;
pub type EventSinkId = String;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityRequest {
    pub name: String,
}

#[derive(Debug, Clone)]
pub enum ProvidedService {
    InferenceEngine(InferenceEngineId),
    ContextProvider(ContextProviderId),
    ContextPlanner(ContextPlannerId),
    SessionStore(SessionStoreId),
    CheckpointStore(CheckpointStoreId),
    MemoryStore(MemoryStoreId),
    ToolProvider(ToolProviderId),
    PolicyContributor(PolicyContributorId),
    EventSink(EventSinkId),
}

#[derive(Debug, Clone)]
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

    fn install(
        &self,
        registry: &mut ExtensionRegistryBuilder,
    ) -> anyhow::Result<()>;
}

pub struct ExtensionRegistryBuilder {
    // providers
    pub inference_engines: Vec<Arc<dyn crate::inference::InferenceEngine>>,
    pub context_providers: Vec<Arc<dyn crate::context::ContextProvider>>,
    pub context_planners: Vec<Arc<dyn crate::context::ContextPlanner>>,
    pub session_stores: Vec<Arc<dyn crate::session::SessionStoreFactory>>,
    pub checkpoint_stores: Vec<Arc<dyn crate::session::CheckpointStoreFactory>>,
    pub memory_stores: Vec<Arc<dyn crate::memory::MemoryStoreFactory>>,

    // contributors
    pub tools: Vec<Arc<dyn crate::tools::ToolContributor>>,
    pub policy_contributors: Vec<Arc<dyn crate::context::PolicyContributor>>,
    pub event_sinks: Vec<Arc<dyn crate::extension::EventSink>>,
}

impl ExtensionRegistryBuilder {
    pub fn new() -> Self {
        Self {
            inference_engines: Vec::new(),
            context_providers: Vec::new(),
            context_planners: Vec::new(),
            session_stores: Vec::new(),
            checkpoint_stores: Vec::new(),
            memory_stores: Vec::new(),
            tools: Vec::new(),
            policy_contributors: Vec::new(),
            event_sinks: Vec::new(),
        }
    }

    pub fn install<E: RoderExtension>(&mut self, ext: E) -> anyhow::Result<()> {
        ext.install(self)
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
    pub fn checkpoint_store_factory(&mut self, store: Arc<dyn crate::session::CheckpointStoreFactory>) {
        self.checkpoint_stores.push(store);
    }
    pub fn memory_store_factory(&mut self, store: Arc<dyn crate::memory::MemoryStoreFactory>) {
        self.memory_stores.push(store);
    }
    pub fn tool_contributor(&mut self, contributor: Arc<dyn crate::tools::ToolContributor>) {
        self.tools.push(contributor);
    }
    pub fn policy_contributor(&mut self, contributor: Arc<dyn crate::context::PolicyContributor>) {
        self.policy_contributors.push(contributor);
    }
    pub fn event_sink(&mut self, sink: Arc<dyn crate::extension::EventSink>) {
        self.event_sinks.push(sink);
    }
}

pub trait EventSink: Send + Sync + 'static {}
