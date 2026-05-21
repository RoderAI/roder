use std::sync::Arc;

use roder_api::events::RoderEvent;
use roder_api::extension::TaskExecutorId;
use roder_api::tasks::{
    TaskExecutionContext, TaskExecutionResult, TaskExecutor, TaskSpec, TaskState,
};
use roder_tasks::{
    BackgroundRunner, BackgroundRunnerConfig, TaskExecutorRegistry, TaskSubmitOptions,
};

struct MetadataEchoExecutor;

#[async_trait::async_trait]
impl TaskExecutor for MetadataEchoExecutor {
    fn id(&self) -> TaskExecutorId {
        "metadata.echo".to_string()
    }

    fn spec(&self) -> TaskSpec {
        TaskSpec {
            kind: "metadata.echo".to_string(),
            description: "Echo task metadata.".to_string(),
            input_schema: serde_json::json!({ "type": "object" }),
            default_timeout_seconds: None,
            metadata: serde_json::Value::Null,
        }
    }

    async fn execute(
        &self,
        ctx: TaskExecutionContext,
        _input: serde_json::Value,
    ) -> anyhow::Result<TaskExecutionResult> {
        Ok(TaskExecutionResult::success(ctx.metadata))
    }
}

#[tokio::test]
async fn automation_metadata_reaches_task_executor_and_task_list() {
    let mut registry = TaskExecutorRegistry::default();
    registry.register(Arc::new(MetadataEchoExecutor)).unwrap();
    let runner = BackgroundRunner::new(registry, BackgroundRunnerConfig::default());
    let mut events = runner.subscribe();
    let metadata = serde_json::json!({
        "automationId": "automation-1",
        "automationRunId": "run-1"
    });

    let handle = runner
        .submit(
            "metadata.echo",
            serde_json::json!({}),
            TaskSubmitOptions {
                metadata: metadata.clone(),
                ..TaskSubmitOptions::default()
            },
        )
        .await
        .unwrap();

    for _ in 0..50 {
        if let Some(task) = runner.get(&handle.task_id).await
            && task.state == TaskState::Completed
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    let listed = runner.list().await;
    assert!(
        listed
            .iter()
            .any(|task| task.task_id == handle.task_id && task.spec.kind == "metadata.echo")
    );
    loop {
        let event = events.recv().await.unwrap();
        if let RoderEvent::TaskCompleted(completed) = event
            && completed.task_id == handle.task_id
        {
            assert_eq!(completed.payload, metadata);
            break;
        }
    }
}
