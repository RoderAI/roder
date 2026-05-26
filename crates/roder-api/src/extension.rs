use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};

use crate::capabilities::{CapabilityDenial, CapabilityGrant, CapabilityRequest, CapabilityStatus};

pub type ExtensionId = String;
pub type ApiVersion = String;
pub type InferenceEngineId = String;
pub type ContextProviderId = String;
pub type ContextPlannerId = String;
pub type ThreadStoreId = String;
pub type CheckpointStoreId = String;
pub type MemoryStoreId = String;
pub type EmbeddingProviderId = String;
pub type ToolProviderId = String;
pub type SubagentDispatcherId = String;
pub type PolicyContributorId = String;
pub type EventSinkId = String;
pub type TaskExecutorId = String;
pub type NotificationSinkId = String;
pub type InteractiveRegionHandlerId = String;
pub type SpeechTranscriberId = String;

pub const SUPPORTED_EXTENSION_API_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ProvidedService {
    InferenceEngine(InferenceEngineId),
    ContextProvider(ContextProviderId),
    ContextPlanner(ContextPlannerId),
    ThreadStore(ThreadStoreId),
    CheckpointStore(CheckpointStoreId),
    MemoryStore(MemoryStoreId),
    EmbeddingProvider(EmbeddingProviderId),
    ToolProvider(ToolProviderId),
    SubagentDispatcher(SubagentDispatcherId),
    PolicyContributor(PolicyContributorId),
    EventSink(EventSinkId),
    TaskExecutor(TaskExecutorId),
    NotificationSink(NotificationSinkId),
    InteractiveRegionHandler(InteractiveRegionHandlerId),
    SpeechTranscriber(SpeechTranscriberId),
    RemoteRunnerProvider(crate::remote_runner::RemoteRunnerProviderId),
    StatusSegment(crate::tui_status::StatusSegmentId),
    PaletteSource(crate::tui_status::PaletteSourceId),
    CodeIndexProvider(crate::code_index::CodeIndexProviderId),
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
    pub capability_statuses: BTreeMap<ExtensionId, Vec<CapabilityStatus>>,
    pub inference_engines: Vec<Arc<dyn crate::inference::InferenceEngine>>,
    pub context_providers: Vec<Arc<dyn crate::context::ContextProvider>>,
    pub context_planners: Vec<Arc<dyn crate::context::ContextPlanner>>,
    pub thread_stores: Vec<Arc<dyn crate::thread::ThreadStoreFactory>>,
    pub checkpoint_stores: Vec<Arc<dyn crate::thread::CheckpointStoreFactory>>,
    pub memory_stores: Vec<Arc<dyn crate::memory::MemoryStoreFactory>>,
    pub embedding_providers: Vec<Arc<dyn crate::embeddings::EmbeddingProvider>>,
    pub tools: Vec<Arc<dyn crate::tools::ToolContributor>>,
    pub subagent_dispatchers: Vec<Arc<dyn crate::subagents::SubagentDispatcher>>,
    pub policy_contributors: Vec<Arc<dyn crate::context::PolicyContributor>>,
    pub event_sinks: Vec<Arc<dyn crate::extension::EventSink>>,
    pub task_executors: Vec<Arc<dyn crate::tasks::TaskExecutor>>,
    pub notification_sinks: Vec<Arc<dyn crate::notifications::NotificationSink>>,
    pub interactive_region_handlers: Vec<Arc<dyn crate::interactive::InteractiveRegionHandler>>,
    pub speech_transcribers: Vec<Arc<dyn crate::speech::SpeechTranscriber>>,
    pub remote_runner_providers: Vec<Arc<dyn crate::remote_runner::RemoteRunnerProvider>>,
    pub status_segments: Vec<crate::tui_status::StatusSegment>,
    pub palette_sources: Vec<crate::tui_status::PaletteSourceDescriptor>,
    pub code_index_providers: Vec<Arc<dyn crate::code_index::CodeIndexProvider>>,
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

    pub fn speech_transcriber(
        &self,
        id: &str,
    ) -> Option<Arc<dyn crate::speech::SpeechTranscriber>> {
        self.speech_transcribers
            .iter()
            .find(|transcriber| transcriber.id() == id)
            .cloned()
    }

    pub fn provided_services(&self) -> Vec<ProvidedService> {
        self.manifests
            .iter()
            .flat_map(|manifest| manifest.provides.iter().cloned())
            .collect()
    }

    pub fn capability_statuses(&self, extension_id: &str) -> &[CapabilityStatus] {
        self.capability_statuses
            .get(extension_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
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
    granted_capabilities: BTreeMap<ExtensionId, BTreeSet<String>>,
    denied_capabilities: BTreeMap<ExtensionId, BTreeMap<String, String>>,
    pub inference_engines: Vec<Arc<dyn crate::inference::InferenceEngine>>,
    pub context_providers: Vec<Arc<dyn crate::context::ContextProvider>>,
    pub context_planners: Vec<Arc<dyn crate::context::ContextPlanner>>,
    pub thread_stores: Vec<Arc<dyn crate::thread::ThreadStoreFactory>>,
    pub checkpoint_stores: Vec<Arc<dyn crate::thread::CheckpointStoreFactory>>,
    pub memory_stores: Vec<Arc<dyn crate::memory::MemoryStoreFactory>>,
    pub embedding_providers: Vec<Arc<dyn crate::embeddings::EmbeddingProvider>>,
    pub tools: Vec<Arc<dyn crate::tools::ToolContributor>>,
    pub subagent_dispatchers: Vec<Arc<dyn crate::subagents::SubagentDispatcher>>,
    pub policy_contributors: Vec<Arc<dyn crate::context::PolicyContributor>>,
    pub event_sinks: Vec<Arc<dyn crate::extension::EventSink>>,
    pub task_executors: Vec<Arc<dyn crate::tasks::TaskExecutor>>,
    pub notification_sinks: Vec<Arc<dyn crate::notifications::NotificationSink>>,
    pub interactive_region_handlers: Vec<Arc<dyn crate::interactive::InteractiveRegionHandler>>,
    pub speech_transcribers: Vec<Arc<dyn crate::speech::SpeechTranscriber>>,
    pub remote_runner_providers: Vec<Arc<dyn crate::remote_runner::RemoteRunnerProvider>>,
    pub status_segments: Vec<crate::tui_status::StatusSegment>,
    pub palette_sources: Vec<crate::tui_status::PaletteSourceDescriptor>,
    pub code_index_providers: Vec<Arc<dyn crate::code_index::CodeIndexProvider>>,
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
            granted_capabilities: BTreeMap::new(),
            denied_capabilities: BTreeMap::new(),
            inference_engines: Vec::new(),
            context_providers: Vec::new(),
            context_planners: Vec::new(),
            thread_stores: Vec::new(),
            checkpoint_stores: Vec::new(),
            memory_stores: Vec::new(),
            embedding_providers: Vec::new(),
            tools: Vec::new(),
            subagent_dispatchers: Vec::new(),
            policy_contributors: Vec::new(),
            event_sinks: Vec::new(),
            task_executors: Vec::new(),
            notification_sinks: Vec::new(),
            interactive_region_handlers: Vec::new(),
            speech_transcribers: Vec::new(),
            remote_runner_providers: Vec::new(),
            status_segments: Vec::new(),
            palette_sources: Vec::new(),
            code_index_providers: Vec::new(),
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
        let validation = self.validate()?;
        Ok(ExtensionRegistry {
            manifests: self.manifests,
            capability_statuses: validation.capability_statuses,
            inference_engines: self.inference_engines,
            context_providers: self.context_providers,
            context_planners: self.context_planners,
            thread_stores: self.thread_stores,
            checkpoint_stores: self.checkpoint_stores,
            memory_stores: self.memory_stores,
            embedding_providers: self.embedding_providers,
            tools: self.tools,
            subagent_dispatchers: self.subagent_dispatchers,
            policy_contributors: self.policy_contributors,
            event_sinks: self.event_sinks,
            task_executors: self.task_executors,
            notification_sinks: self.notification_sinks,
            interactive_region_handlers: self.interactive_region_handlers,
            speech_transcribers: self.speech_transcribers,
            remote_runner_providers: self.remote_runner_providers,
            status_segments: self.status_segments,
            palette_sources: self.palette_sources,
            code_index_providers: self.code_index_providers,
        })
    }

    pub fn manifest(&mut self, manifest: ExtensionManifest) {
        self.manifests.push(manifest);
    }

    pub fn grant_capability(&mut self, extension_id: impl Into<String>, grant: CapabilityGrant) {
        self.granted_capabilities
            .entry(extension_id.into())
            .or_default()
            .insert(grant.id);
    }

    pub fn deny_capability(&mut self, extension_id: impl Into<String>, denial: CapabilityDenial) {
        self.denied_capabilities
            .entry(extension_id.into())
            .or_default()
            .insert(denial.id, denial.reason);
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

    pub fn thread_store_factory(&mut self, store: Arc<dyn crate::thread::ThreadStoreFactory>) {
        self.thread_stores.push(store);
    }

    pub fn checkpoint_store_factory(
        &mut self,
        store: Arc<dyn crate::thread::CheckpointStoreFactory>,
    ) {
        self.checkpoint_stores.push(store);
    }

    pub fn memory_store_factory(&mut self, store: Arc<dyn crate::memory::MemoryStoreFactory>) {
        self.memory_stores.push(store);
    }

    pub fn embedding_provider(&mut self, provider: Arc<dyn crate::embeddings::EmbeddingProvider>) {
        self.embedding_providers.push(provider);
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

    pub fn task_executor(&mut self, executor: Arc<dyn crate::tasks::TaskExecutor>) {
        self.task_executors.push(executor);
    }

    pub fn notification_sink(&mut self, sink: Arc<dyn crate::notifications::NotificationSink>) {
        self.notification_sinks.push(sink);
    }

    pub fn interactive_region_handler(
        &mut self,
        handler: Arc<dyn crate::interactive::InteractiveRegionHandler>,
    ) {
        self.interactive_region_handlers.push(handler);
    }

    pub fn speech_transcriber(&mut self, transcriber: Arc<dyn crate::speech::SpeechTranscriber>) {
        self.speech_transcribers.push(transcriber);
    }

    pub fn remote_runner_provider(
        &mut self,
        provider: Arc<dyn crate::remote_runner::RemoteRunnerProvider>,
    ) {
        self.remote_runner_providers.push(provider);
    }

    pub fn status_segment(&mut self, segment: crate::tui_status::StatusSegment) {
        self.status_segments.push(segment);
    }

    pub fn palette_source(&mut self, source: crate::tui_status::PaletteSourceDescriptor) {
        self.palette_sources.push(source);
    }

    pub fn code_index_provider(&mut self, provider: Arc<dyn crate::code_index::CodeIndexProvider>) {
        self.code_index_providers.push(provider);
    }

    fn validate(&self) -> anyhow::Result<RegistryValidation> {
        validate_manifests(&self.manifests)?;
        validate_actual_services(self)?;
        validate_tool_contributors(&self.tools)?;
        let capability_statuses = validate_capabilities(
            &self.manifests,
            &self.granted_capabilities,
            &self.denied_capabilities,
        )?;
        Ok(RegistryValidation {
            capability_statuses,
        })
    }
}

#[async_trait::async_trait]
pub trait EventSink: Send + Sync + 'static {
    fn id(&self) -> EventSinkId;

    async fn handle_event(&self, envelope: &crate::events::EventEnvelope) -> anyhow::Result<()>;
}

struct RegistryValidation {
    capability_statuses: BTreeMap<ExtensionId, Vec<CapabilityStatus>>,
}

fn validate_manifests(manifests: &[ExtensionManifest]) -> anyhow::Result<()> {
    let mut extension_ids = BTreeSet::new();
    let mut services = BTreeMap::<ProvidedService, ExtensionId>::new();
    for manifest in manifests {
        if manifest.id.trim().is_empty() {
            anyhow::bail!("extension manifest has an empty id");
        }
        if !extension_ids.insert(manifest.id.clone()) {
            anyhow::bail!("duplicate extension id {}", manifest.id);
        }
        validate_api_version(manifest)?;
        for service in &manifest.provides {
            if let Some(existing) = services.insert(service.clone(), manifest.id.clone()) {
                anyhow::bail!(
                    "duplicate provided service {} declared by {} and {}",
                    service_label(service),
                    existing,
                    manifest.id
                );
            }
        }
    }
    Ok(())
}

fn validate_api_version(manifest: &ExtensionManifest) -> anyhow::Result<()> {
    let supported = Version::parse(SUPPORTED_EXTENSION_API_VERSION)?;
    let requirement = VersionReq::parse(&manifest.api_version).or_else(|_| {
        Version::parse(&manifest.api_version).map(|version| VersionReq {
            comparators: vec![semver::Comparator {
                op: semver::Op::Exact,
                major: version.major,
                minor: Some(version.minor),
                patch: Some(version.patch),
                pre: version.pre,
            }],
        })
    })?;
    if requirement.matches(&supported) {
        Ok(())
    } else {
        anyhow::bail!(
            "extension {} requires unsupported API version {}; supported {}",
            manifest.id,
            manifest.api_version,
            SUPPORTED_EXTENSION_API_VERSION
        )
    }
}

fn validate_actual_services(builder: &ExtensionRegistryBuilder) -> anyhow::Result<()> {
    let declared = builder
        .manifests
        .iter()
        .flat_map(|manifest| manifest.provides.iter().cloned())
        .collect::<BTreeSet<_>>();
    let actual = actual_services(builder)?;
    for service in &declared {
        if !actual.contains(service) {
            anyhow::bail!(
                "manifest declares provided service {} but no matching service was installed",
                service_label(service)
            );
        }
    }
    validate_duplicate_actual_services(&actual)
}

fn validate_duplicate_actual_services(actual: &[ProvidedService]) -> anyhow::Result<()> {
    let mut seen = BTreeSet::new();
    for service in actual {
        if !seen.insert(service.clone()) {
            anyhow::bail!("duplicate installed service {}", service_label(service));
        }
    }
    Ok(())
}

fn actual_services(builder: &ExtensionRegistryBuilder) -> anyhow::Result<Vec<ProvidedService>> {
    let mut services = Vec::new();
    services.extend(
        builder
            .inference_engines
            .iter()
            .map(|service| ProvidedService::InferenceEngine(service.id())),
    );
    services.extend(
        builder
            .context_providers
            .iter()
            .map(|service| ProvidedService::ContextProvider(service.id())),
    );
    services.extend(
        builder
            .context_planners
            .iter()
            .map(|service| ProvidedService::ContextPlanner(service.id())),
    );
    services.extend(
        builder
            .thread_stores
            .iter()
            .map(|service| ProvidedService::ThreadStore(service.id())),
    );
    services.extend(
        builder
            .checkpoint_stores
            .iter()
            .map(|service| ProvidedService::CheckpointStore(service.id())),
    );
    services.extend(
        builder
            .memory_stores
            .iter()
            .map(|service| ProvidedService::MemoryStore(service.id())),
    );
    services.extend(
        builder
            .embedding_providers
            .iter()
            .map(|service| ProvidedService::EmbeddingProvider(service.descriptor().id)),
    );
    services.extend(
        builder
            .tools
            .iter()
            .map(|service| ProvidedService::ToolProvider(service.id())),
    );
    services.extend(
        builder
            .subagent_dispatchers
            .iter()
            .map(|service| ProvidedService::SubagentDispatcher(service.id())),
    );
    services.extend(
        builder
            .policy_contributors
            .iter()
            .map(|service| ProvidedService::PolicyContributor(service.id())),
    );
    services.extend(
        builder
            .event_sinks
            .iter()
            .map(|service| ProvidedService::EventSink(service.id())),
    );
    services.extend(
        builder
            .task_executors
            .iter()
            .map(|service| ProvidedService::TaskExecutor(service.id())),
    );
    services.extend(
        builder
            .notification_sinks
            .iter()
            .map(|service| ProvidedService::NotificationSink(service.id())),
    );
    services.extend(
        builder
            .interactive_region_handlers
            .iter()
            .map(|service| ProvidedService::InteractiveRegionHandler(service.id())),
    );
    services.extend(
        builder
            .speech_transcribers
            .iter()
            .map(|service| ProvidedService::SpeechTranscriber(service.id())),
    );
    services.extend(
        builder
            .remote_runner_providers
            .iter()
            .map(|service| ProvidedService::RemoteRunnerProvider(service.id())),
    );
    services.extend(
        builder
            .status_segments
            .iter()
            .map(|service| ProvidedService::StatusSegment(service.id.clone())),
    );
    services.extend(
        builder
            .palette_sources
            .iter()
            .map(|service| ProvidedService::PaletteSource(service.id.clone())),
    );
    services.extend(
        builder
            .code_index_providers
            .iter()
            .map(|service| ProvidedService::CodeIndexProvider(service.id())),
    );
    Ok(services)
}

fn validate_tool_contributors(
    contributors: &[Arc<dyn crate::tools::ToolContributor>],
) -> anyhow::Result<()> {
    let mut registry = crate::tools::ToolRegistry::default();
    for contributor in contributors {
        contributor.contribute(&mut registry)?;
    }
    Ok(())
}

fn validate_capabilities(
    manifests: &[ExtensionManifest],
    granted: &BTreeMap<ExtensionId, BTreeSet<String>>,
    denied: &BTreeMap<ExtensionId, BTreeMap<String, String>>,
) -> anyhow::Result<BTreeMap<ExtensionId, Vec<CapabilityStatus>>> {
    let mut statuses = BTreeMap::new();
    for manifest in manifests {
        let mut seen = BTreeSet::new();
        let mut extension_statuses = Vec::new();
        for request in &manifest.required_capabilities {
            if !seen.insert(request.id.clone()) {
                anyhow::bail!(
                    "extension {} declares capability {} more than once",
                    manifest.id,
                    request.id
                );
            }
            if let Some(reason) = denied
                .get(&manifest.id)
                .and_then(|denials| denials.get(&request.id))
            {
                anyhow::bail!(
                    "extension {} requires denied capability {}: {}",
                    manifest.id,
                    request.id,
                    reason
                );
            }
            let decision = if granted
                .get(&manifest.id)
                .is_some_and(|grants| grants.contains(&request.id))
            {
                crate::capabilities::CapabilityDecision::Granted
            } else {
                crate::capabilities::CapabilityDecision::Requested
            };
            extension_statuses.push(CapabilityStatus {
                id: request.id.clone(),
                decision,
                reason: request.reason.clone(),
            });
        }
        statuses.insert(manifest.id.clone(), extension_statuses);
    }
    Ok(statuses)
}

fn service_label(service: &ProvidedService) -> String {
    match service {
        ProvidedService::InferenceEngine(id) => format!("InferenceEngine({id})"),
        ProvidedService::ContextProvider(id) => format!("ContextProvider({id})"),
        ProvidedService::ContextPlanner(id) => format!("ContextPlanner({id})"),
        ProvidedService::ThreadStore(id) => format!("ThreadStore({id})"),
        ProvidedService::CheckpointStore(id) => format!("CheckpointStore({id})"),
        ProvidedService::MemoryStore(id) => format!("MemoryStore({id})"),
        ProvidedService::EmbeddingProvider(id) => format!("EmbeddingProvider({id})"),
        ProvidedService::ToolProvider(id) => format!("ToolProvider({id})"),
        ProvidedService::SubagentDispatcher(id) => format!("SubagentDispatcher({id})"),
        ProvidedService::PolicyContributor(id) => format!("PolicyContributor({id})"),
        ProvidedService::EventSink(id) => format!("EventSink({id})"),
        ProvidedService::TaskExecutor(id) => format!("TaskExecutor({id})"),
        ProvidedService::NotificationSink(id) => format!("NotificationSink({id})"),
        ProvidedService::InteractiveRegionHandler(id) => {
            format!("InteractiveRegionHandler({id})")
        }
        ProvidedService::SpeechTranscriber(id) => format!("SpeechTranscriber({id})"),
        ProvidedService::RemoteRunnerProvider(id) => format!("RemoteRunnerProvider({id})"),
        ProvidedService::StatusSegment(id) => format!("StatusSegment({id})"),
        ProvidedService::PaletteSource(id) => format!("PaletteSource({id})"),
        ProvidedService::CodeIndexProvider(id) => format!("CodeIndexProvider({id})"),
    }
}

#[cfg(test)]
mod tests {
    use crate::tui_status::{PaletteSourceDescriptor, StatusCell, StatusSegment, StatusStyle};

    use super::*;

    #[test]
    fn provided_service_status_segment_round_trips_json() {
        let service = ProvidedService::StatusSegment("mode".to_string());
        let encoded = serde_json::to_value(&service).expect("serialize status segment service");
        assert_eq!(encoded, serde_json::json!({ "StatusSegment": "mode" }));

        let decoded = serde_json::from_value::<ProvidedService>(encoded)
            .expect("deserialize status segment service");
        assert_eq!(decoded, service);
    }

    #[test]
    fn provided_service_palette_source_round_trips_json() {
        let service = ProvidedService::PaletteSource("commands".to_string());
        let encoded = serde_json::to_value(&service).expect("serialize palette source service");
        assert_eq!(encoded, serde_json::json!({ "PaletteSource": "commands" }));

        let decoded = serde_json::from_value::<ProvidedService>(encoded)
            .expect("deserialize palette source service");
        assert_eq!(decoded, service);
    }

    #[test]
    fn provided_service_task_executor_round_trips_json() {
        let service = ProvidedService::TaskExecutor("process".to_string());
        let encoded = serde_json::to_value(&service).expect("serialize task executor service");
        assert_eq!(encoded, serde_json::json!({ "TaskExecutor": "process" }));

        let decoded = serde_json::from_value::<ProvidedService>(encoded)
            .expect("deserialize task executor service");
        assert_eq!(decoded, service);
    }

    #[test]
    fn provided_service_code_index_provider_round_trips_json() {
        let service = ProvidedService::CodeIndexProvider("local-code-index".to_string());
        let encoded =
            serde_json::to_value(&service).expect("serialize code index provider service");
        assert_eq!(
            encoded,
            serde_json::json!({ "CodeIndexProvider": "local-code-index" })
        );

        let decoded = serde_json::from_value::<ProvidedService>(encoded)
            .expect("deserialize code index provider service");
        assert_eq!(decoded, service);
    }

    #[test]
    fn provided_service_notification_sink_round_trips_json() {
        let service = ProvidedService::NotificationSink("terminal-bell".to_string());
        let encoded = serde_json::to_value(&service).expect("serialize notification sink service");
        assert_eq!(
            encoded,
            serde_json::json!({ "NotificationSink": "terminal-bell" })
        );

        let decoded = serde_json::from_value::<ProvidedService>(encoded)
            .expect("deserialize notification sink service");
        assert_eq!(decoded, service);
    }

    #[test]
    fn provided_service_interactive_region_handler_round_trips_json() {
        let service = ProvidedService::InteractiveRegionHandler("links".to_string());
        let encoded =
            serde_json::to_value(&service).expect("serialize interactive region handler service");
        assert_eq!(
            encoded,
            serde_json::json!({ "InteractiveRegionHandler": "links" })
        );

        let decoded = serde_json::from_value::<ProvidedService>(encoded)
            .expect("deserialize interactive region handler service");
        assert_eq!(decoded, service);
    }

    #[test]
    fn provided_service_remote_runner_provider_round_trips_json() {
        let service = ProvidedService::RemoteRunnerProvider("unix-local".to_string());
        let encoded =
            serde_json::to_value(&service).expect("serialize remote runner provider service");
        assert_eq!(
            encoded,
            serde_json::json!({ "RemoteRunnerProvider": "unix-local" })
        );

        let decoded = serde_json::from_value::<ProvidedService>(encoded)
            .expect("deserialize remote runner provider service");
        assert_eq!(decoded, service);
    }

    #[test]
    fn registry_builder_records_status_segments() {
        let mut builder = ExtensionRegistryBuilder::new();
        builder.status_segment(StatusSegment::new("custom", 42, 6, |_| StatusCell {
            text: "ready".to_string(),
            style: StatusStyle::Accent,
            tooltip: None,
        }));

        let registry = builder.build().expect("build registry");
        assert_eq!(registry.status_segments.len(), 1);
        assert_eq!(registry.status_segments[0].id, "custom");
        assert_eq!(registry.status_segments[0].priority, 42);
        assert_eq!(registry.status_segments[0].min_width, 6);
    }

    #[test]
    fn registry_builder_records_palette_sources() {
        let mut builder = ExtensionRegistryBuilder::new();
        builder.palette_source(PaletteSourceDescriptor {
            id: "commands".to_string(),
            label: "Commands".to_string(),
            priority: 100,
        });

        let registry = builder.build().expect("build registry");
        assert_eq!(registry.palette_sources.len(), 1);
        assert_eq!(registry.palette_sources[0].id, "commands");
        assert_eq!(registry.palette_sources[0].label, "Commands");
        assert_eq!(registry.palette_sources[0].priority, 100);
    }
}
