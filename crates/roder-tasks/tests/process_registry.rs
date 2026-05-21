use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use roder_api::processes::{ProcessDescriptor, ProcessOrigin, ProcessState, ProcessStopper};
use roder_api::tasks::TaskOutputStream;
use roder_tasks::{ProcessRegistry, ProcessRegistryConfig};
use time::OffsetDateTime;

struct FlagStopper(Arc<AtomicBool>);

#[async_trait::async_trait]
impl ProcessStopper for FlagStopper {
    async fn stop(&self, _reason: Option<String>) -> anyhow::Result<()> {
        self.0.store(true, Ordering::SeqCst);
        Ok(())
    }
}

fn descriptor(process_id: &str, task_id: Option<&str>) -> ProcessDescriptor {
    ProcessDescriptor {
        process_id: process_id.to_string(),
        origin: ProcessOrigin::BackgroundTask,
        state: ProcessState::Running,
        command: vec!["sleep".to_string(), "10".to_string()],
        command_summary: "sleep 10".to_string(),
        cwd: Some("/tmp".to_string()),
        pid: Some(1234),
        task_id: task_id.map(str::to_string),
        thread_id: Some("thread-1".to_string()),
        turn_id: Some("turn-1".to_string()),
        runner_destination_id: None,
        runner_session_id: None,
        stoppable: true,
        started_at: OffsetDateTime::UNIX_EPOCH,
        updated_at: OffsetDateTime::UNIX_EPOCH,
        stdout_tail: None,
        stderr_tail: None,
    }
}

#[tokio::test]
async fn process_registry_registers_lists_outputs_and_marks_exit() {
    let registry = ProcessRegistry::default();
    let mut events = registry.subscribe();
    registry
        .register(descriptor("process-1", Some("task-1")), None)
        .await
        .unwrap();

    assert_eq!(registry.list(false).await.len(), 1);
    registry
        .append_task_output(
            "task-1",
            TaskOutputStream::Stdout,
            "ready\n".to_string(),
            0,
            Some("thread-1".to_string()),
            Some("turn-1".to_string()),
        )
        .await
        .unwrap();
    assert_eq!(registry.output("process-1").await.len(), 1);
    assert_eq!(
        registry
            .get("process-1")
            .await
            .unwrap()
            .stdout_tail
            .as_deref(),
        Some("ready\n")
    );

    registry.mark_exited("process-1", Some(0)).await.unwrap();
    assert!(registry.list(false).await.is_empty());
    assert_eq!(registry.list(true).await.len(), 1);

    let mut kinds = Vec::new();
    for _ in 0..3 {
        kinds.push(events.recv().await.unwrap().kind().to_string());
    }
    assert_eq!(
        kinds,
        vec!["process.started", "process.output", "process.exited"]
    );
}

#[tokio::test]
async fn process_registry_stop_is_idempotent_and_uses_stopper() {
    let registry = ProcessRegistry::default();
    let stopped = Arc::new(AtomicBool::new(false));
    registry
        .register(
            descriptor("process-1", None),
            Some(Arc::new(FlagStopper(Arc::clone(&stopped)))),
        )
        .await
        .unwrap();

    let result = registry
        .stop("process-1", Some("test".to_string()))
        .await
        .unwrap();
    assert!(result.stopped);
    assert!(stopped.load(Ordering::SeqCst));

    registry
        .mark_stopped("process-1", Some("test".to_string()))
        .await
        .unwrap();
    let result = registry.stop("process-1", None).await.unwrap();
    assert!(!result.stopped);
}

#[tokio::test]
async fn process_registry_retains_only_recent_completed_descriptors() {
    let registry = ProcessRegistry::new(ProcessRegistryConfig {
        max_completed: 1,
        max_output_bytes: 1024,
    });
    registry
        .register(descriptor("old", None), None)
        .await
        .unwrap();
    registry.mark_exited("old", Some(0)).await.unwrap();
    registry
        .register(descriptor("new", None), None)
        .await
        .unwrap();
    registry.mark_exited("new", Some(0)).await.unwrap();

    let listed = registry.list(true).await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].process_id, "new");
}
