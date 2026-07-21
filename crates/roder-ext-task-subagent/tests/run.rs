use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use roder_api::events::RoderEvent;
use roder_api::subagents::{
    SubagentDefinition, SubagentDispatcher, SubagentExitReason, SubagentPermissionMode,
    SubagentRequest, SubagentResult,
};
use roder_api::tasks::TaskState;
use roder_ext_task_subagent::{SubagentTaskExecutor, SubagentTaskExtension};
use roder_tasks::{
    BackgroundRunner, BackgroundRunnerConfig, TaskExecutorRegistry, TaskSubmitOptions,
};
use tokio::sync::Notify;

struct FakeDispatcher {
    started: Arc<Notify>,
    dropped: Arc<AtomicBool>,
    delay: Duration,
}

#[async_trait::async_trait]
impl SubagentDispatcher for FakeDispatcher {
    fn id(&self) -> String {
        "fake".to_string()
    }

    fn definitions(&self) -> Vec<SubagentDefinition> {
        vec![SubagentDefinition {
            agent_type: "explore".to_string(),
            description: "Explore".to_string(),
            tools: vec!["Read".to_string()],
            model: Some("test-model".to_string()),
            system_prompt: None,
            permission_mode: SubagentPermissionMode::ReadOnly,
            max_turns: Some(2),
            max_result_chars: Some(1000),
        }]
    }

    async fn dispatch(
        &self,
        parent_thread_id: String,
        parent_turn_id: String,
        request: SubagentRequest,
    ) -> anyhow::Result<SubagentResult> {
        struct DropFlag(Arc<AtomicBool>);
        impl Drop for DropFlag {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }

        let _drop_flag = DropFlag(Arc::clone(&self.dropped));
        self.started.notify_waiters();
        tokio::time::sleep(self.delay).await;
        Ok(SubagentResult {
            thread_id: format!("{parent_thread_id}-child"),
            turn_id: format!("{parent_turn_id}-child"),
            agent_type: request
                .subagent_type
                .unwrap_or_else(|| "explore".to_string()),
            model: request.model,
            final_message: "subagent complete".to_string(),
            usage: None,
            exit_reason: SubagentExitReason::Completed,
            transcript: Some(serde_json::json!([{ "role": "assistant", "text": "note" }])),
            metadata: serde_json::json!({ "ok": true }),
        })
    }
}

fn runner(delay: Duration) -> (BackgroundRunner, Arc<Notify>, Arc<AtomicBool>) {
    let started = Arc::new(Notify::new());
    let dropped = Arc::new(AtomicBool::new(false));
    let dispatcher = Arc::new(FakeDispatcher {
        started: Arc::clone(&started),
        dropped: Arc::clone(&dropped),
        delay,
    });
    let mut registry = TaskExecutorRegistry::default();
    registry
        .register(Arc::new(SubagentTaskExecutor::new(dispatcher)))
        .unwrap();
    (
        BackgroundRunner::new(
            registry,
            BackgroundRunnerConfig {
                max_concurrent: 1,
                max_log_bytes: 4096,
                auto_cancel_on_session_end: true,
                process_grace_timeout: Duration::from_millis(250),
                process_kill_timeout: Duration::from_secs(1),
                max_completed_process_diagnostics: 64,
            },
        ),
        started,
        dropped,
    )
}

#[tokio::test]
async fn subagent_task_forwards_result_and_transcript() {
    let (runner, _started, _dropped) = runner(Duration::from_millis(10));
    let mut events = runner.subscribe();
    let handle = runner
        .submit(
            "subagent",
            serde_json::json!({
                "description": "Inspect",
                "prompt": "Find context",
                "subagent_type": "explore",
                "model": "test-model"
            }),
            TaskSubmitOptions {
                thread_id: Some("thread-a".to_string()),
                turn_id: Some("turn-a".to_string()),
                ..TaskSubmitOptions::default()
            },
        )
        .await
        .unwrap();

    let completed = loop {
        if let RoderEvent::TaskCompleted(event) =
            tokio::time::timeout(Duration::from_secs(2), events.recv())
                .await
                .unwrap()
                .unwrap()
        {
            break event;
        }
    };

    assert_eq!(completed.payload["final_message"], "subagent complete");
    assert_eq!(completed.payload["agent_type"], "explore");
    let (logs, dropped) = runner.logs(&handle.task_id).await.unwrap();
    let log_text = logs
        .iter()
        .map(|entry| entry.chunk.as_str())
        .collect::<String>();
    assert!(log_text.contains("subagent complete"));
    assert!(log_text.contains("assistant"));
    assert_eq!(dropped, 0);
    assert_eq!(
        runner.get(&handle.task_id).await.unwrap().state,
        TaskState::Completed
    );
}

#[tokio::test]
async fn subagent_task_trace_payload_preserves_parent_child_ids() {
    let (runner, _started, _dropped) = runner(Duration::from_millis(10));
    let mut events = runner.subscribe();
    let _handle = runner
        .submit(
            "subagent",
            serde_json::json!({
                "description": "Inspect",
                "prompt": "Find context",
                "subagent_type": "explore"
            }),
            TaskSubmitOptions {
                thread_id: Some("thread-trace".to_string()),
                turn_id: Some("turn-trace".to_string()),
                ..TaskSubmitOptions::default()
            },
        )
        .await
        .unwrap();

    let completed = loop {
        if let RoderEvent::TaskCompleted(event) =
            tokio::time::timeout(Duration::from_secs(2), events.recv())
                .await
                .unwrap()
                .unwrap()
        {
            break event;
        }
    };

    assert_eq!(completed.thread_id.as_deref(), Some("thread-trace"));
    assert_eq!(completed.turn_id.as_deref(), Some("turn-trace"));
    assert_eq!(completed.payload["thread_id"], "thread-trace-child");
    assert_eq!(completed.payload["turn_id"], "turn-trace-child");
}

#[tokio::test]
async fn cancelling_subagent_task_aborts_dispatch_future() {
    let (runner, started, dropped) = runner(Duration::from_secs(10));
    let handle = runner
        .submit(
            "subagent",
            serde_json::json!({ "description": "Wait", "prompt": "Wait" }),
            TaskSubmitOptions::default(),
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), started.notified())
        .await
        .unwrap();

    let started_at = std::time::Instant::now();
    assert!(
        runner
            .cancel(&handle.task_id, Some("test".to_string()))
            .await
            .unwrap()
    );
    assert!(started_at.elapsed() < Duration::from_millis(100));
    tokio::task::yield_now().await;
    assert!(dropped.load(Ordering::SeqCst));
}

#[test]
fn extension_skips_executor_without_dispatcher() {
    let mut builder = roder_api::ExtensionRegistryBuilder::new();
    builder.install(SubagentTaskExtension).unwrap();
    let registry = builder.build().unwrap();

    assert!(registry.task_executors.is_empty());
}
