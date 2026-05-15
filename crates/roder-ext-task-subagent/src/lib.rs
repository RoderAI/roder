mod task;

use std::sync::Arc;

use roder_api::{ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension};

pub use task::{SUBAGENT_TASK_EXECUTOR_ID, SubagentTaskExecutor};

pub struct SubagentTaskExtension;

impl RoderExtension for SubagentTaskExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-task-subagent".to_string(),
            name: "Subagent Task Executor".to_string(),
            version: semver::Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("Background subagent task executor".to_string()),
            provides: vec![ProvidedService::TaskExecutor(
                SUBAGENT_TASK_EXECUTOR_ID.to_string(),
            )],
            required_capabilities: Vec::new(),
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        if let Some(dispatcher) = registry.subagent_dispatchers.first().cloned() {
            registry.task_executor(Arc::new(SubagentTaskExecutor::new(dispatcher)));
        }
        Ok(())
    }
}
