use std::sync::Arc;

use roder_api::events::RoderEvent;
use roder_api::extension::TaskExecutorId;
use roder_api::processes::{ProcessDescriptor, ProcessOrigin, ProcessState};
use roder_api::tasks::{
    TaskExecutionContext, TaskExecutionResult, TaskExecutor, TaskSpec, TaskState,
};
use roder_tasks::{
    BackgroundRunner, BackgroundRunnerConfig, TaskExecutorRegistry, TaskSubmitOptions,
};
use time::OffsetDateTime;

struct MetadataEchoExecutor;

fn process_descriptor(process_id: &str) -> ProcessDescriptor {
    ProcessDescriptor {
        process_id: process_id.to_string(),
        origin: ProcessOrigin::BackgroundTask,
        state: ProcessState::Running,
        command: vec!["sleep".to_string(), "1".to_string()],
        command_summary: "sleep 1".to_string(),
        cwd: None,
        pid: None,
        task_id: None,
        thread_id: None,
        turn_id: None,
        runner_destination_id: None,
        runner_session_id: None,
        stoppable: true,
        started_at: OffsetDateTime::UNIX_EPOCH,
        updated_at: OffsetDateTime::UNIX_EPOCH,
        stdout_tail: None,
        stderr_tail: None,
    }
}

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

#[tokio::test]
async fn runner_bounds_completed_process_diagnostics_from_its_config() {
    let runner = BackgroundRunner::new(
        TaskExecutorRegistry::default(),
        BackgroundRunnerConfig {
            max_completed_process_diagnostics: 1,
            ..BackgroundRunnerConfig::default()
        },
    );
    let processes = runner.processes();

    processes
        .register(process_descriptor("old"), None)
        .await
        .unwrap();
    processes.mark_exited("old", Some(0)).await.unwrap();
    processes
        .register(process_descriptor("new"), None)
        .await
        .unwrap();
    processes.mark_exited("new", Some(0)).await.unwrap();

    let completed = processes.list(true).await;
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].process_id, "new");
}
