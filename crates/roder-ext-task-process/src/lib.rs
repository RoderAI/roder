mod task;

use std::sync::Arc;

use roder_api::{ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension};

pub use task::{PROCESS_TASK_EXECUTOR_ID, ProcessTaskExecutor};

pub struct ProcessTaskExtension;

impl RoderExtension for ProcessTaskExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-task-process".to_string(),
            name: "Process Task Executor".to_string(),
            version: semver::Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("Background process task executor".to_string()),
            provides: vec![ProvidedService::TaskExecutor(
                PROCESS_TASK_EXECUTOR_ID.to_string(),
            )],
            required_capabilities: Vec::new(),
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.task_executor(Arc::new(ProcessTaskExecutor));
        Ok(())
    }
}
