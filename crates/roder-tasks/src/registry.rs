use std::collections::BTreeMap;
use std::sync::Arc;

use roder_api::extension::TaskExecutorId;
use roder_api::tasks::{TaskExecutor, TaskSpec};

#[derive(Default, Clone)]
pub struct TaskExecutorRegistry {
    executors: BTreeMap<TaskExecutorId, Arc<dyn TaskExecutor>>,
}

impl TaskExecutorRegistry {
    pub fn register(&mut self, executor: Arc<dyn TaskExecutor>) -> anyhow::Result<()> {
        let id = executor.id();
        if self.executors.contains_key(&id) {
            anyhow::bail!("task executor {id:?} is already registered");
        }
        self.executors.insert(id, executor);
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn TaskExecutor>> {
        self.executors.get(id).cloned()
    }

    pub fn specs(&self) -> Vec<TaskSpec> {
        self.executors
            .values()
            .map(|executor| executor.spec())
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.executors.is_empty()
    }
}
