use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::events::{ThreadId, TurnId};
use crate::extension::TaskExecutorId;
use crate::processes::ProcessRegistrySink;
use crate::remote_runner::{RemoteRunnerSession, RunnerDestination};
use crate::{ToolSchemaPolicy, normalize_tool_schema};

pub type TaskId = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskSpec {
    pub kind: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_timeout_seconds: Option<u64>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

impl TaskSpec {
    pub fn normalized_for_model(&self, policy: ToolSchemaPolicy) -> Self {
        let mut spec = self.clone();
        spec.input_schema = normalize_tool_schema(&spec.kind, &spec.input_schema, policy).schema;
        spec
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskHandle {
    pub task_id: TaskId,
    pub executor_id: TaskExecutorId,
    pub spec: TaskSpec,
    pub state: TaskState,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub started_at: Option<OffsetDateTime>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub finished_at: Option<OffsetDateTime>,
}

#[derive(Clone)]
pub struct TaskExecutionContext {
    pub task_id: TaskId,
    pub thread_id: Option<ThreadId>,
    pub turn_id: Option<TurnId>,
    pub workspace_root: Option<String>,
    pub runner_destination: Option<RunnerDestination>,
    pub runner_session: Option<Arc<dyn RemoteRunnerSession>>,
    pub deadline: Option<OffsetDateTime>,
    /// Bounded local-process graceful-stop budget selected by the task host.
    pub process_grace_timeout: Duration,
    /// Bounded local-process forced-kill/reap budget selected by the task host.
    pub process_kill_timeout: Duration,
    pub metadata: serde_json::Value,
    pub process_registry: Option<Arc<dyn ProcessRegistrySink>>,
    pub output: TaskOutputSink,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskExecutionResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub payload: serde_json::Value,
}

impl TaskExecutionResult {
    pub fn success(payload: serde_json::Value) -> Self {
        Self {
            exit_code: None,
            payload,
        }
    }
}

impl fmt::Debug for TaskExecutionContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TaskExecutionContext")
            .field("task_id", &self.task_id)
            .field("thread_id", &self.thread_id)
            .field("turn_id", &self.turn_id)
            .field("workspace_root", &self.workspace_root)
            .field("runner_destination", &self.runner_destination)
            .field(
                "runner_session",
                &self.runner_session.as_ref().map(|session| session.state()),
            )
            .field("deadline", &self.deadline)
            .field("process_grace_timeout", &self.process_grace_timeout)
            .field("process_kill_timeout", &self.process_kill_timeout)
            .field("metadata", &self.metadata)
            .field(
                "process_registry",
                &self.process_registry.as_ref().map(|_| "<process-registry>"),
            )
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskOutputStream {
    Stdout,
    Stderr,
    Log,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskStarted {
    pub task_id: TaskId,
    pub executor_id: TaskExecutorId,
    pub task_kind: String,
    #[serde(default)]
    pub queue_depth: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<ThreadId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskOutput {
    pub task_id: TaskId,
    pub stream: TaskOutputStream,
    pub chunk: String,
    #[serde(default)]
    pub dropped_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<ThreadId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskCompleted {
    pub task_id: TaskId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub payload: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<ThreadId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskFailed {
    pub task_id: TaskId,
    pub error: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partial_result: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<ThreadId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskCancelled {
    pub task_id: TaskId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<ThreadId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Clone)]
pub struct TaskOutputSink {
    writer: Arc<dyn TaskOutputWriter>,
}

impl Default for TaskOutputSink {
    fn default() -> Self {
        Self {
            writer: Arc::new(NoopTaskOutputWriter),
        }
    }
}

impl fmt::Debug for TaskOutputSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TaskOutputSink").finish_non_exhaustive()
    }
}

impl TaskOutputSink {
    pub fn new(writer: Arc<dyn TaskOutputWriter>) -> Self {
        Self { writer }
    }

    pub async fn write(
        &self,
        stream: TaskOutputStream,
        chunk: impl Into<String>,
    ) -> anyhow::Result<()> {
        self.writer.write(stream, chunk.into()).await
    }
}

#[async_trait::async_trait]
pub trait TaskOutputWriter: Send + Sync + 'static {
    async fn write(&self, stream: TaskOutputStream, chunk: String) -> anyhow::Result<()>;
}

struct NoopTaskOutputWriter;

#[async_trait::async_trait]
impl TaskOutputWriter for NoopTaskOutputWriter {
    async fn write(&self, _stream: TaskOutputStream, _chunk: String) -> anyhow::Result<()> {
        Ok(())
    }
}

#[async_trait::async_trait]
pub trait TaskExecutor: Send + Sync + 'static {
    fn id(&self) -> TaskExecutorId;

    fn spec(&self) -> TaskSpec;

    async fn execute(
        &self,
        ctx: TaskExecutionContext,
        input: serde_json::Value,
    ) -> anyhow::Result<TaskExecutionResult>;
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    struct NoopTaskExecutor;

    #[async_trait::async_trait]
    impl TaskExecutor for NoopTaskExecutor {
        fn id(&self) -> TaskExecutorId {
            "noop-task".to_string()
        }

        fn spec(&self) -> TaskSpec {
            TaskSpec {
                kind: "noop".to_string(),
                description: "No-op task".to_string(),
                input_schema: serde_json::json!({ "type": "object" }),
                default_timeout_seconds: Some(30),
                metadata: serde_json::json!({ "category": "test" }),
            }
        }

        async fn execute(
            &self,
            ctx: TaskExecutionContext,
            input: serde_json::Value,
        ) -> anyhow::Result<TaskExecutionResult> {
            Ok(TaskExecutionResult::success(serde_json::json!({
                "task_id": ctx.task_id,
                "input": input,
            })))
        }
    }

    #[test]
    fn task_handle_round_trips_json() {
        let handle = TaskHandle {
            task_id: "task-1".to_string(),
            executor_id: "process".to_string(),
            spec: TaskSpec {
                kind: "process".to_string(),
                description: "Run a process".to_string(),
                input_schema: serde_json::json!({ "type": "object" }),
                default_timeout_seconds: Some(60),
                metadata: serde_json::json!({}),
            },
            state: TaskState::Queued,
            created_at: OffsetDateTime::UNIX_EPOCH,
            started_at: None,
            finished_at: None,
        };

        let encoded = serde_json::to_string(&handle).expect("serialize task handle");
        let decoded: TaskHandle = serde_json::from_str(&encoded).expect("deserialize task handle");

        assert_eq!(decoded, handle);
    }

    #[test]
    fn task_events_round_trip_json() {
        let started = TaskStarted {
            task_id: "task-1".to_string(),
            executor_id: "process".to_string(),
            task_kind: "process".to_string(),
            queue_depth: 0,
            thread_id: Some("thread-a".to_string()),
            turn_id: Some("turn-a".to_string()),
            timestamp: OffsetDateTime::UNIX_EPOCH,
        };
        let output = TaskOutput {
            task_id: "task-1".to_string(),
            stream: TaskOutputStream::Stdout,
            chunk: "hello\n".to_string(),
            dropped_bytes: 0,
            thread_id: Some("thread-a".to_string()),
            turn_id: Some("turn-a".to_string()),
            timestamp: OffsetDateTime::UNIX_EPOCH,
        };

        assert_eq!(
            serde_json::from_value::<TaskStarted>(serde_json::to_value(&started).unwrap()).unwrap(),
            started
        );
        assert_eq!(
            serde_json::from_value::<TaskOutput>(serde_json::to_value(&output).unwrap()).unwrap(),
            output
        );
    }

    #[tokio::test]
    async fn task_executor_trait_is_object_safe() {
        let executor: Arc<dyn TaskExecutor> = Arc::new(NoopTaskExecutor);
        let result = executor
            .execute(
                TaskExecutionContext {
                    task_id: "task-1".to_string(),
                    thread_id: None,
                    turn_id: None,
                    workspace_root: None,
                    runner_destination: None,
                    runner_session: None,
                    deadline: None,
                    process_grace_timeout: Duration::from_millis(250),
                    process_kill_timeout: Duration::from_secs(1),
                    metadata: serde_json::json!({}),
                    process_registry: None,
                    output: TaskOutputSink::default(),
                },
                serde_json::json!({ "ok": true }),
            )
            .await
            .unwrap();

        assert_eq!(executor.id(), "noop-task");
        assert_eq!(executor.spec().kind, "noop");
        assert_eq!(result.payload["task_id"], "task-1");
    }
}
