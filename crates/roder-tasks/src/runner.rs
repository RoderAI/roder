use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Result of quiescing app-server-owned background work. A clean result means
/// registered task processes reached a terminal process-registry state within
/// the caller's deadline; generic task cancellation is reported separately
/// because arbitrary executors do not all have a reaping acknowledgement.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BackgroundDrainOutcome {
    pub quiesced_task_ids: Vec<TaskId>,
    pub cancelled_task_ids: Vec<TaskId>,
    pub stopped_process_ids: Vec<ProcessId>,
    pub remaining_process_ids: Vec<ProcessId>,
}

impl BackgroundDrainOutcome {
    pub fn is_clean(&self) -> bool {
        self.remaining_process_ids.is_empty()
    }
}

use roder_api::events::{EventEnvelope, RoderEvent, ThreadId, TurnId};
use roder_api::extension::TaskExecutorId;
use roder_api::processes::{ProcessId, ProcessState};
use roder_api::remote_runner::{RemoteRunnerSession, RunnerDestination};
use roder_api::tasks::{
    TaskCancelled, TaskCompleted, TaskExecutionContext, TaskFailed, TaskHandle, TaskId, TaskOutput,
    TaskOutputSink, TaskOutputStream, TaskOutputWriter, TaskStarted, TaskState,
};
use time::OffsetDateTime;
use tokio::sync::{Mutex, Semaphore, broadcast};
use tokio::task::AbortHandle;

use crate::log_buffer::{BoundedLogBuffer, TaskLogEntry};
use crate::process_registry::{ProcessRegistry, ProcessRegistryConfig};
use crate::registry::TaskExecutorRegistry;

#[derive(Debug, Clone)]
pub struct BackgroundRunnerConfig {
    pub max_concurrent: usize,
    pub max_log_bytes: usize,
    pub auto_cancel_on_session_end: bool,
    /// Time a local process executor may handle cooperative termination before
    /// its own forced-kill path begins.
    pub process_grace_timeout: std::time::Duration,
    /// Time reserved to observe local forced-kill/reap before the runner aborts
    /// a still-active task future.
    pub process_kill_timeout: std::time::Duration,
    /// Terminal process descriptors retained for bounded lifecycle diagnostics.
    pub max_completed_process_diagnostics: usize,
}

impl Default for BackgroundRunnerConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 4,
            max_log_bytes: 64 * 1024,
            auto_cancel_on_session_end: true,
            process_grace_timeout: std::time::Duration::from_millis(250),
            process_kill_timeout: std::time::Duration::from_secs(1),
            max_completed_process_diagnostics: 64,
        }
    }
}

#[derive(Clone, Default)]
pub struct TaskSubmitOptions {
    pub thread_id: Option<ThreadId>,
    pub turn_id: Option<TurnId>,
    pub workspace_root: Option<String>,
    pub runner_destination: Option<RunnerDestination>,
    pub runner_session: Option<Arc<dyn RemoteRunnerSession>>,
    pub deadline: Option<OffsetDateTime>,
    pub metadata: serde_json::Value,
}

#[derive(Clone)]
pub struct BackgroundRunner {
    registry: TaskExecutorRegistry,
    config: BackgroundRunnerConfig,
    semaphore: Arc<Semaphore>,
    /// Serializes new task admission with a shutdown quiesce. Once quiesced,
    /// callers receive a deterministic error instead of creating work outside
    /// the bounded drain snapshot.
    admission: Arc<Mutex<()>>,
    accepting_tasks: Arc<AtomicBool>,
    tasks: Arc<Mutex<BTreeMap<TaskId, TaskRecord>>>,
    processes: ProcessRegistry,
    events: broadcast::Sender<RoderEvent>,
}

struct TaskRecord {
    handle: TaskHandle,
    log: BoundedLogBuffer,
    abort_handle: Option<AbortHandle>,
    thread_id: Option<ThreadId>,
    turn_id: Option<TurnId>,
}

impl BackgroundRunner {
    pub fn new(registry: TaskExecutorRegistry, config: BackgroundRunnerConfig) -> Self {
        let (events, _) = broadcast::channel(1024);
        let processes = ProcessRegistry::new(ProcessRegistryConfig {
            max_completed: config.max_completed_process_diagnostics,
            ..ProcessRegistryConfig::default()
        });
        if tokio::runtime::Handle::try_current().is_ok() {
            let mut process_events = processes.subscribe();
            let task_events = events.clone();
            tokio::spawn(async move {
                while let Ok(event) = process_events.recv().await {
                    let _ = task_events.send(event);
                }
            });
        }
        Self {
            registry,
            semaphore: Arc::new(Semaphore::new(config.max_concurrent.max(1))),
            config,
            admission: Arc::new(Mutex::new(())),
            accepting_tasks: Arc::new(AtomicBool::new(true)),
            tasks: Arc::new(Mutex::new(BTreeMap::new())),
            processes,
            events,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RoderEvent> {
        self.events.subscribe()
    }

    pub fn processes(&self) -> ProcessRegistry {
        self.processes.clone()
    }

    pub async fn submit(
        &self,
        executor_id: impl Into<TaskExecutorId>,
        input: serde_json::Value,
        options: TaskSubmitOptions,
    ) -> anyhow::Result<TaskHandle> {
        let admission = self.admission.lock().await;
        anyhow::ensure!(
            self.accepting_tasks.load(Ordering::Acquire),
            "background runner is quiescing and cannot accept new tasks"
        );
        let executor_id = executor_id.into();
        let executor = self
            .registry
            .get(&executor_id)
            .ok_or_else(|| anyhow::anyhow!("unknown task executor {executor_id:?}"))?;
        let spec = executor.spec();
        let task_id = uuid::Uuid::new_v4().to_string();
        let handle = TaskHandle {
            task_id: task_id.clone(),
            executor_id: executor_id.clone(),
            spec: spec.clone(),
            state: TaskState::Queued,
            created_at: OffsetDateTime::now_utc(),
            started_at: None,
            finished_at: None,
        };

        {
            let mut tasks = self.tasks.lock().await;
            tasks.insert(
                task_id.clone(),
                TaskRecord {
                    handle: handle.clone(),
                    log: BoundedLogBuffer::new(self.config.max_log_bytes),
                    abort_handle: None,
                    thread_id: options.thread_id.clone(),
                    turn_id: options.turn_id.clone(),
                },
            );
        }

        let runner = self.clone();
        let task_id_for_spawn = task_id.clone();
        let spawn_options = options.clone();
        let join = tokio::spawn(async move {
            runner
                .run_task(
                    task_id_for_spawn,
                    executor_id,
                    executor,
                    input,
                    spawn_options,
                )
                .await;
        });
        let abort_handle = join.abort_handle();
        {
            let mut tasks = self.tasks.lock().await;
            if let Some(record) = tasks.get_mut(&task_id) {
                record.abort_handle = Some(abort_handle);
            }
        }

        drop(admission);
        Ok(handle)
    }

    /// Rejects new work and returns the IDs of active tasks captured under the
    /// same admission gate. Callers can stop child processes first, then cancel
    /// these task joins without a new submission slipping into the gap.
    pub async fn quiesce(&self) -> Vec<TaskId> {
        let admission = self.admission.lock().await;
        self.accepting_tasks.store(false, Ordering::Release);
        let active_task_ids = self
            .tasks
            .lock()
            .await
            .iter()
            .filter(|(_, record)| {
                !matches!(
                    record.handle.state,
                    TaskState::Completed | TaskState::Failed | TaskState::Cancelled
                )
            })
            .map(|(task_id, _)| task_id.clone())
            .collect();
        drop(admission);
        active_task_ids
    }

    /// Cancels the supplied task IDs, returning only IDs whose active task
    /// state was transitioned to `Cancelled`.
    pub async fn cancel_many(&self, task_ids: Vec<TaskId>, reason: Option<String>) -> Vec<TaskId> {
        let mut cancelled = Vec::new();
        for task_id in task_ids {
            if self.cancel(&task_id, reason.clone()).await.unwrap_or(false) {
                cancelled.push(task_id);
            }
        }
        cancelled
    }

    /// Convenience shutdown helper for owners that do not need to order task
    /// cancellation around registered process stoppers.
    pub async fn cancel_all(&self, reason: Option<String>) -> Vec<TaskId> {
        let task_ids = self.quiesce().await;
        self.cancel_many(task_ids, reason).await
    }

    /// Quiesces background task admission, asks registered processes to stop
    /// cooperatively, then falls back to task cancellation after a short grace
    /// window. The deadline is bounded and the remaining process IDs preserve
    /// enough diagnostic state for a later recovery inspection.
    pub async fn drain(
        &self,
        timeout: std::time::Duration,
        reason: Option<String>,
    ) -> BackgroundDrainOutcome {
        let started_at = tokio::time::Instant::now();
        let quiesced_task_ids = self.quiesce().await;
        let process_stop_results = self.processes.stop_all(reason.clone()).await;
        let stopped_process_ids = process_stop_results
            .iter()
            .filter(|result| result.stopped)
            .map(|result| result.process_id.clone())
            .collect::<Vec<_>>();

        let graceful_window = timeout.min(self.config.process_grace_timeout);
        let mut remaining_process_ids = self.wait_for_active_processes(graceful_window).await;

        if !remaining_process_ids.is_empty() {
            let remaining_timeout = timeout.saturating_sub(started_at.elapsed());
            let force_window = remaining_timeout.min(self.config.process_kill_timeout);
            remaining_process_ids = self.wait_for_active_processes(force_window).await;
        }
        let cancelled_task_ids = self.cancel_many(quiesced_task_ids.clone(), reason).await;

        if !remaining_process_ids.is_empty() {
            let remaining_timeout = timeout.saturating_sub(started_at.elapsed());
            remaining_process_ids = self.wait_for_active_processes(remaining_timeout).await;
        }

        BackgroundDrainOutcome {
            quiesced_task_ids,
            cancelled_task_ids,
            stopped_process_ids,
            remaining_process_ids,
        }
    }

    /// Re-opens task admission after an embedding host deliberately keeps the
    /// server alive following a bounded drain timeout.
    pub fn resume_accepting_tasks(&self) {
        self.accepting_tasks.store(true, Ordering::Release);
    }

    async fn wait_for_active_processes(&self, timeout: std::time::Duration) -> Vec<ProcessId> {
        let started_at = tokio::time::Instant::now();
        loop {
            let remaining_process_ids = self
                .processes
                .list(false)
                .await
                .into_iter()
                .filter(|process| {
                    matches!(
                        process.state,
                        ProcessState::Starting | ProcessState::Running | ProcessState::Stopping
                    )
                })
                .map(|process| process.process_id)
                .collect::<Vec<_>>();
            if remaining_process_ids.is_empty() || started_at.elapsed() >= timeout {
                return remaining_process_ids;
            }
            let remaining_timeout = timeout.saturating_sub(started_at.elapsed());
            tokio::time::sleep(remaining_timeout.min(std::time::Duration::from_millis(10))).await;
        }
    }

    pub async fn cancel(&self, task_id: &str, reason: Option<String>) -> anyhow::Result<bool> {
        let cancelled = {
            let mut tasks = self.tasks.lock().await;
            let Some(record) = tasks.get_mut(task_id) else {
                anyhow::bail!("unknown task {task_id:?}");
            };
            if matches!(
                record.handle.state,
                TaskState::Completed | TaskState::Failed | TaskState::Cancelled
            ) {
                return Ok(false);
            }
            record.handle.state = TaskState::Cancelled;
            record.handle.finished_at = Some(OffsetDateTime::now_utc());
            if let Some(abort_handle) = record.abort_handle.take() {
                abort_handle.abort();
            }
            true
        };

        if cancelled {
            self.emit(RoderEvent::TaskCancelled(TaskCancelled {
                task_id: task_id.to_string(),
                reason,
                thread_id: self.thread_id(task_id).await,
                turn_id: self.turn_id(task_id).await,
                timestamp: OffsetDateTime::now_utc(),
            }));
        }

        Ok(cancelled)
    }

    pub async fn list(&self) -> Vec<TaskHandle> {
        self.tasks
            .lock()
            .await
            .values()
            .map(|record| record.handle.clone())
            .collect()
    }

    pub async fn get(&self, task_id: &str) -> Option<TaskHandle> {
        self.tasks
            .lock()
            .await
            .get(task_id)
            .map(|record| record.handle.clone())
    }

    pub async fn logs(&self, task_id: &str) -> Option<(Vec<TaskLogEntry>, u64)> {
        self.tasks
            .lock()
            .await
            .get(task_id)
            .map(|record| (record.log.entries(), record.log.dropped_bytes()))
    }

    pub async fn handle_event(&self, envelope: &EventEnvelope) -> anyhow::Result<()> {
        if !self.config.auto_cancel_on_session_end {
            return Ok(());
        }
        if !matches!(
            envelope.kind.as_str(),
            "session.ended" | "turn.completed" | "turn.failed" | "turn.interrupted"
        ) {
            return Ok(());
        }
        let Some(thread_id) = envelope.thread_id.as_deref() else {
            return Ok(());
        };
        let task_ids = {
            self.tasks
                .lock()
                .await
                .iter()
                .filter_map(|(task_id, record)| {
                    let active = !matches!(
                        record.handle.state,
                        TaskState::Completed | TaskState::Failed | TaskState::Cancelled
                    );
                    let same_thread =
                        active && self.record_thread_id(record).as_deref() == Some(thread_id);
                    same_thread.then(|| task_id.clone())
                })
                .collect::<Vec<_>>()
        };
        for task_id in task_ids {
            self.cancel(&task_id, Some("session ended".to_string()))
                .await?;
        }
        Ok(())
    }

    async fn run_task(
        &self,
        task_id: TaskId,
        executor_id: TaskExecutorId,
        executor: Arc<dyn roder_api::tasks::TaskExecutor>,
        input: serde_json::Value,
        options: TaskSubmitOptions,
    ) {
        let permit = match self.semaphore.clone().acquire_owned().await {
            Ok(permit) => permit,
            Err(_) => return,
        };
        let _permit = permit;

        let queue_depth = {
            let mut tasks = self.tasks.lock().await;
            let queue_depth = tasks
                .values()
                .filter(|record| record.handle.state == TaskState::Queued)
                .count()
                .saturating_sub(1);
            if let Some(record) = tasks.get_mut(&task_id) {
                if record.handle.state == TaskState::Cancelled {
                    return;
                }
                record.handle.state = TaskState::Running;
                record.handle.started_at = Some(OffsetDateTime::now_utc());
            }
            queue_depth
        };

        self.emit(RoderEvent::TaskStarted(TaskStarted {
            task_id: task_id.clone(),
            executor_id,
            task_kind: executor.spec().kind,
            thread_id: options.thread_id.clone(),
            turn_id: options.turn_id.clone(),
            queue_depth,
            timestamp: OffsetDateTime::now_utc(),
        }));

        let ctx = TaskExecutionContext {
            task_id: task_id.clone(),
            thread_id: options.thread_id.clone(),
            turn_id: options.turn_id.clone(),
            workspace_root: options.workspace_root,
            runner_destination: options.runner_destination,
            runner_session: options.runner_session,
            deadline: options.deadline,
            process_grace_timeout: self.config.process_grace_timeout,
            process_kill_timeout: self.config.process_kill_timeout,
            metadata: options.metadata,
            process_registry: Some(Arc::new(self.processes.clone())),
            output: TaskOutputSink::new(Arc::new(RunnerOutputWriter {
                runner: self.clone(),
                task_id: task_id.clone(),
                thread_id: options.thread_id.clone(),
                turn_id: options.turn_id.clone(),
            })),
        };

        let mut timeout_partial_result = None;
        let result = if let Some(deadline) = options.deadline {
            let now = OffsetDateTime::now_utc();
            let duration = (deadline - now).unsigned_abs();
            let deadline_instant = if deadline > now {
                tokio::time::Instant::now() + duration
            } else {
                tokio::time::Instant::now()
            };
            match tokio::time::timeout_at(deadline_instant, executor.execute(ctx, input)).await {
                Ok(result) => result,
                Err(_) => {
                    let partial = self.partial_result(&task_id).await;
                    timeout_partial_result = Some(partial.clone());
                    self.emit(RoderEvent::TaskOutput(TaskOutput {
                        task_id: task_id.clone(),
                        stream: TaskOutputStream::Log,
                        chunk: format!("task deadline expired; partial result: {partial}"),
                        dropped_bytes: 0,
                        thread_id: options.thread_id.clone(),
                        turn_id: options.turn_id.clone(),
                        timestamp: OffsetDateTime::now_utc(),
                    }));
                    Err(anyhow::anyhow!("task deadline expired"))
                }
            }
        } else {
            executor.execute(ctx, input).await
        };

        match result {
            Ok(payload) => {
                self.finish_task(&task_id, TaskState::Completed).await;
                self.emit(RoderEvent::TaskCompleted(TaskCompleted {
                    task_id,
                    exit_code: payload.exit_code,
                    payload: payload.payload,
                    thread_id: options.thread_id,
                    turn_id: options.turn_id,
                    timestamp: OffsetDateTime::now_utc(),
                }));
            }
            Err(error) => {
                self.finish_task(&task_id, TaskState::Failed).await;
                self.emit(RoderEvent::TaskFailed(TaskFailed {
                    task_id,
                    error: error.to_string(),
                    error_kind: timeout_partial_result
                        .as_ref()
                        .map(|_| "deadline_timeout".to_string()),
                    partial_result: timeout_partial_result,
                    thread_id: options.thread_id,
                    turn_id: options.turn_id,
                    timestamp: OffsetDateTime::now_utc(),
                }));
            }
        }
    }

    async fn finish_task(&self, task_id: &str, state: TaskState) {
        let mut tasks = self.tasks.lock().await;
        if let Some(record) = tasks.get_mut(task_id) {
            if record.handle.state == TaskState::Cancelled {
                return;
            }
            record.handle.state = state;
            record.handle.finished_at = Some(OffsetDateTime::now_utc());
            record.abort_handle = None;
        }
    }

    async fn append_output(
        &self,
        task_id: &str,
        stream: TaskOutputStream,
        chunk: String,
        thread_id: Option<ThreadId>,
        turn_id: Option<TurnId>,
    ) -> anyhow::Result<()> {
        let dropped_bytes = {
            let mut tasks = self.tasks.lock().await;
            let Some(record) = tasks.get_mut(task_id) else {
                anyhow::bail!("unknown task {task_id:?}");
            };
            record.log.push(stream.clone(), chunk.clone())
        };
        let _ = self
            .processes
            .append_task_output(
                task_id,
                stream.clone(),
                chunk.clone(),
                dropped_bytes,
                thread_id.clone(),
                turn_id.clone(),
            )
            .await;
        self.emit(RoderEvent::TaskOutput(TaskOutput {
            task_id: task_id.to_string(),
            stream,
            chunk,
            dropped_bytes,
            thread_id,
            turn_id,
            timestamp: OffsetDateTime::now_utc(),
        }));
        Ok(())
    }

    async fn partial_result(&self, task_id: &str) -> String {
        let Some((logs, dropped)) = self.logs(task_id).await else {
            return "no task output captured before timeout".to_string();
        };
        if logs.is_empty() {
            return "no task output captured before timeout".to_string();
        }
        let mut text = logs
            .iter()
            .rev()
            .take(3)
            .map(|entry| entry.chunk.trim())
            .collect::<Vec<_>>();
        text.reverse();
        let mut partial = text.join("\n");
        if dropped > 0 {
            partial.push_str(&format!("\n... {dropped} bytes dropped"));
        }
        partial
    }

    fn emit(&self, event: RoderEvent) {
        let _ = self.events.send(event);
    }

    async fn thread_id(&self, task_id: &str) -> Option<ThreadId> {
        self.tasks
            .lock()
            .await
            .get(task_id)
            .and_then(|record| self.record_thread_id(record))
    }

    async fn turn_id(&self, task_id: &str) -> Option<TurnId> {
        self.tasks
            .lock()
            .await
            .get(task_id)
            .and_then(|record| self.record_turn_id(record))
    }

    fn record_thread_id(&self, record: &TaskRecord) -> Option<ThreadId> {
        record.thread_id.clone()
    }

    fn record_turn_id(&self, record: &TaskRecord) -> Option<TurnId> {
        record.turn_id.clone()
    }
}

struct RunnerOutputWriter {
    runner: BackgroundRunner,
    task_id: TaskId,
    thread_id: Option<ThreadId>,
    turn_id: Option<TurnId>,
}

#[async_trait::async_trait]
impl TaskOutputWriter for RunnerOutputWriter {
    async fn write(&self, stream: TaskOutputStream, chunk: String) -> anyhow::Result<()> {
        self.runner
            .append_output(
                &self.task_id,
                stream,
                chunk,
                self.thread_id.clone(),
                self.turn_id.clone(),
            )
            .await
    }
}
