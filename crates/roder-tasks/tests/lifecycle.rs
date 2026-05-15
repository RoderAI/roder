use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use roder_api::events::{EventEnvelope, EventSource, RoderEvent};
use roder_api::tasks::{TaskExecutionContext, TaskExecutor, TaskOutputStream, TaskSpec, TaskState};
use roder_tasks::{
    BackgroundRunner, BackgroundRunnerConfig, TaskExecutorRegistry, TaskSubmitOptions,
};
use time::OffsetDateTime;
use tokio::sync::Notify;

struct TestExecutor {
    id: &'static str,
    notify_started: Arc<Notify>,
    delay: Duration,
    output: Option<&'static str>,
    running_count: Arc<AtomicUsize>,
    max_running_count: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl TaskExecutor for TestExecutor {
    fn id(&self) -> String {
        self.id.to_string()
    }

    fn spec(&self) -> TaskSpec {
        TaskSpec {
            kind: self.id.to_string(),
            description: "test executor".to_string(),
            input_schema: serde_json::json!({ "type": "object" }),
            default_timeout_seconds: None,
            metadata: serde_json::json!({}),
        }
    }

    async fn execute(
        &self,
        ctx: TaskExecutionContext,
        _input: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let running = self.running_count.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_running_count.fetch_max(running, Ordering::SeqCst);
        self.notify_started.notify_waiters();
        if let Some(output) = self.output {
            ctx.output.write(TaskOutputStream::Stdout, output).await?;
        }
        tokio::time::sleep(self.delay).await;
        self.running_count.fetch_sub(1, Ordering::SeqCst);
        Ok(serde_json::json!({ "ok": true }))
    }
}

fn runner(
    max_concurrent: usize,
    max_log_bytes: usize,
) -> (BackgroundRunner, Arc<Notify>, Arc<AtomicUsize>) {
    let notify_started = Arc::new(Notify::new());
    let running_count = Arc::new(AtomicUsize::new(0));
    let max_running_count = Arc::new(AtomicUsize::new(0));
    let mut registry = TaskExecutorRegistry::default();
    registry
        .register(Arc::new(TestExecutor {
            id: "test",
            notify_started: Arc::clone(&notify_started),
            delay: Duration::from_millis(25),
            output: Some("hello background task\n"),
            running_count: Arc::clone(&running_count),
            max_running_count: Arc::clone(&max_running_count),
        }))
        .unwrap();

    (
        BackgroundRunner::new(
            registry,
            BackgroundRunnerConfig {
                max_concurrent,
                max_log_bytes,
                auto_cancel_on_session_end: true,
            },
        ),
        notify_started,
        max_running_count,
    )
}

#[tokio::test]
async fn submit_run_and_complete_emits_events_and_logs() {
    let (runner, _notify, _max_running) = runner(2, 1024);
    let mut events = runner.subscribe();
    let handle = runner
        .submit(
            "test",
            serde_json::json!({}),
            TaskSubmitOptions {
                thread_id: Some("thread-a".to_string()),
                turn_id: Some("turn-a".to_string()),
                ..TaskSubmitOptions::default()
            },
        )
        .await
        .unwrap();

    let mut saw_started = false;
    let mut saw_output = false;
    let mut saw_completed = false;
    for _ in 0..4 {
        match tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .unwrap()
            .unwrap()
        {
            RoderEvent::TaskStarted(started) => {
                assert_eq!(started.task_id, handle.task_id);
                assert_eq!(started.queue_depth, 0);
                saw_started = true;
            }
            RoderEvent::TaskOutput(output) => {
                assert_eq!(output.chunk, "hello background task\n");
                saw_output = true;
            }
            RoderEvent::TaskCompleted(completed) => {
                assert_eq!(completed.task_id, handle.task_id);
                saw_completed = true;
                break;
            }
            _ => {}
        }
    }

    assert!(saw_started);
    assert!(saw_output);
    assert!(saw_completed);
    assert_eq!(
        runner.get(&handle.task_id).await.unwrap().state,
        TaskState::Completed
    );
    let (logs, dropped) = runner.logs(&handle.task_id).await.unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].chunk, "hello background task\n");
    assert_eq!(dropped, 0);
}

#[tokio::test]
async fn cancel_is_prompt_and_idempotent() {
    let (runner, notify_started, _max_running) = runner(1, 1024);
    let handle = runner
        .submit("test", serde_json::json!({}), TaskSubmitOptions::default())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), notify_started.notified())
        .await
        .unwrap();

    let started = std::time::Instant::now();
    assert!(
        runner
            .cancel(&handle.task_id, Some("test".to_string()))
            .await
            .unwrap()
    );
    assert!(started.elapsed() < Duration::from_millis(100));
    assert!(!runner.cancel(&handle.task_id, None).await.unwrap());
    assert_eq!(
        runner.get(&handle.task_id).await.unwrap().state,
        TaskState::Cancelled
    );
}

#[tokio::test]
async fn deadline_expiry_fails_task() {
    let (runner, _notify, _max_running) = runner(1, 1024);
    let handle = runner
        .submit(
            "test",
            serde_json::json!({}),
            TaskSubmitOptions {
                deadline: Some(OffsetDateTime::now_utc() + time::Duration::milliseconds(1)),
                ..TaskSubmitOptions::default()
            },
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(75)).await;

    assert_eq!(
        runner.get(&handle.task_id).await.unwrap().state,
        TaskState::Failed
    );
}

#[tokio::test]
async fn max_concurrent_limits_execution_and_reports_queue_depth() {
    let (runner, _notify, max_running) = runner(1, 1024);
    let mut events = runner.subscribe();
    let first = runner
        .submit("test", serde_json::json!({}), TaskSubmitOptions::default())
        .await
        .unwrap();
    let second = runner
        .submit("test", serde_json::json!({}), TaskSubmitOptions::default())
        .await
        .unwrap();

    let mut starts = Vec::new();
    while starts.len() < 2 {
        if let RoderEvent::TaskStarted(started) =
            tokio::time::timeout(Duration::from_secs(1), events.recv())
                .await
                .unwrap()
                .unwrap()
        {
            starts.push(started);
        }
    }

    assert_eq!(starts[0].task_id, first.task_id);
    assert_eq!(starts[0].queue_depth, 1);
    assert_eq!(starts[1].task_id, second.task_id);
    assert_eq!(max_running.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn auto_cancel_on_session_end_cancels_running_thread_tasks() {
    let (runner, notify_started, _max_running) = runner(1, 1024);
    let handle = runner
        .submit(
            "test",
            serde_json::json!({}),
            TaskSubmitOptions {
                thread_id: Some("thread-a".to_string()),
                turn_id: Some("turn-a".to_string()),
                ..TaskSubmitOptions::default()
            },
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), notify_started.notified())
        .await
        .unwrap();

    runner
        .handle_event(&EventEnvelope {
            event_id: "event-1".to_string(),
            seq: 1,
            timestamp: OffsetDateTime::now_utc(),
            source: EventSource::Core,
            kind: "turn.completed".to_string(),
            thread_id: Some("thread-a".to_string()),
            turn_id: Some("turn-a".to_string()),
            event: RoderEvent::TurnCompleted(roder_api::events::TurnCompleted {
                thread_id: "thread-a".to_string(),
                turn_id: "turn-a".to_string(),
                timestamp: OffsetDateTime::now_utc(),
            }),
        })
        .await
        .unwrap();

    assert_eq!(
        runner.get(&handle.task_id).await.unwrap().state,
        TaskState::Cancelled
    );
}
