//! Process-hosted extension adapter (roadmap phases 64 and 93).
//!
//! Installs a non-Rust child process as an ordinary Roder extension: the
//! manifest declares the provided services (inference engines, event sinks,
//! tool providers), the host bridges them to canonical `roder-api` traits,
//! and the child speaks newline-delimited JSON-RPC over stdio. See
//! `docs/roder-process-extensions.md`.

use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::extension::{ExtensionManifest, ExtensionRegistryBuilder, RoderExtension};
use roder_api::process_extension::ProcessProvidedService;
use semver::Version;

mod events;
mod inference;
pub mod manifest;
mod process;
mod subagents;
mod tasks;
mod tools;

pub use events::ProcessEventSink;
pub use inference::ProcessInferenceEngine;
pub use manifest::{LoadedProcessExtension, load_process_extension};
pub use process::ProcessHost;
pub use subagents::ProcessSubagentDispatcher;
pub use tasks::ProcessTaskExecutor;
pub use tools::ProcessToolContributor;

/// A manifest-backed process extension ready to install into the registry.
pub struct ProcessHostExtension {
    host: Arc<ProcessHost>,
}

impl ProcessHostExtension {
    pub fn new(loaded: LoadedProcessExtension) -> Self {
        Self {
            host: Arc::new(ProcessHost::new(loaded)),
        }
    }

    /// Shared host handle (e.g. for shutdown or draining extension events).
    pub fn host(&self) -> Arc<ProcessHost> {
        self.host.clone()
    }
}

impl RoderExtension for ProcessHostExtension {
    fn manifest(&self) -> ExtensionManifest {
        let loaded = self.host.loaded();
        ExtensionManifest {
            id: loaded.manifest.id.clone(),
            name: loaded.manifest.name.clone(),
            version: Version::parse(&loaded.manifest.version)
                .unwrap_or_else(|_| Version::new(0, 0, 0)),
            api_version: loaded.manifest.api_version.clone(),
            description: loaded.manifest.description.clone(),
            provides: loaded.manifest.provides.iter().map(Into::into).collect(),
            required_capabilities: loaded
                .manifest
                .required_capabilities
                .iter()
                .map(CapabilityRequest::new)
                .collect(),
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        for service in &self.host.loaded().manifest.provides {
            match service {
                ProcessProvidedService::InferenceEngine { id } => {
                    registry.inference_engine(Arc::new(ProcessInferenceEngine::new(
                        self.host.clone(),
                        id.clone(),
                    )));
                }
                ProcessProvidedService::EventSink { id } => {
                    registry.event_sink(Arc::new(ProcessEventSink::new(
                        self.host.clone(),
                        id.clone(),
                    )));
                }
                ProcessProvidedService::SubagentDispatcher { id } => {
                    registry.subagent_dispatcher(Arc::new(ProcessSubagentDispatcher::new(
                        self.host.clone(),
                        id.clone(),
                    )));
                }
                ProcessProvidedService::TaskExecutor { id } => {
                    registry.task_executor(Arc::new(ProcessTaskExecutor::new(
                        self.host.clone(),
                        id.clone(),
                    )));
                }
                ProcessProvidedService::ToolProvider { id, tools } => {
                    registry.tool_contributor(Arc::new(ProcessToolContributor::new(
                        self.host.clone(),
                        id.clone(),
                        tools.clone(),
                    )));
                }
            }
        }
        Ok(())
    }
}
