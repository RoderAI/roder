//! `TaskExecutor` adapter backed by a process-hosted child (roadmap
//! phase 93).
//!
//! `tasks/execute` is acked immediately by the child; output then streams
//! back as `tasks/event` notifications (forwarded into the task's output
//! sink) until a terminal `completed`/`failed` event. If the host-side
//! execution future is dropped (e.g. `BackgroundRunner::cancel` aborts the
//! task), a guard notifies the child via `tasks/cancel` so the remote work
//! is not silently orphaned.

use std::sync::{Arc, RwLock};

use roder_api::process_extension::{
    METHOD_TASKS_CANCEL, METHOD_TASKS_EXECUTE, METHOD_TASKS_SPEC, ProcessTaskCancelParams,
    ProcessTaskEvent, ProcessTaskExecuteAck, ProcessTaskExecuteParams, ProcessTaskSpecParams,
    ProcessTaskSpecResult,
};
use roder_api::tasks::{TaskExecutionContext, TaskExecutionResult, TaskExecutor, TaskSpec};

use crate::process::ProcessHost;

pub struct ProcessTaskExecutor {
    host: Arc<ProcessHost>,
    executor_id: String,
    spec: Arc<RwLock<Option<TaskSpec>>>,
}

impl ProcessTaskExecutor {
    pub fn new(host: Arc<ProcessHost>, executor_id: String) -> Self {
        let executor = Self {
            host,
            executor_id,
            spec: Arc::new(RwLock::new(None)),
        };
        executor.spawn_spec_fetch();
        executor
    }

    /// Fetches the child-declared spec in the background so the sync
    /// `spec()` accessor can serve it once cached.
    fn spawn_spec_fetch(&self) {
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        let host = self.host.clone();
        let executor_id = self.executor_id.clone();
        let cache = self.spec.clone();
        tokio::spawn(async move {
            if let Ok(spec) = fetch_spec(&host, &executor_id).await
                && let Ok(mut slot) = cache.write()
            {
                *slot = Some(spec);
            }
        });
    }

    /// Awaits the child-declared spec, caching it for the sync accessor.
    pub async fn fetch_spec(&self) -> anyhow::Result<TaskSpec> {
        if let Some(spec) = self.spec.read().ok().and_then(|slot| slot.clone()) {
            return Ok(spec);
        }
        let spec = fetch_spec(&self.host, &self.executor_id).await?;
        if let Ok(mut slot) = self.spec.write() {
            *slot = Some(spec.clone());
        }
        Ok(spec)
    }

    /// Deterministic placeholder served before the child has answered
    /// `tasks/spec` once; the cached child spec replaces it afterwards.
    fn placeholder_spec(&self) -> TaskSpec {
        TaskSpec {
            kind: self.executor_id.clone(),
            description: format!(
                "Process-hosted task executor {} (spec pending first child fetch)",
                self.executor_id
            ),
            input_schema: serde_json::json!({ "type": "object" }),
            default_timeout_seconds: None,
            metadata: serde_json::json!({ "processHosted": true }),
        }
    }
}

async fn fetch_spec(host: &Arc<ProcessHost>, executor_id: &str) -> anyhow::Result<TaskSpec> {
    let result: ProcessTaskSpecResult = host
        .request(
            METHOD_TASKS_SPEC,
            serde_json::to_value(ProcessTaskSpecParams {
                executor_id: executor_id.to_string(),
            })?,
        )
        .await?;
    Ok(result.spec)
}

/// Notifies the child of a host-side cancellation when the execute future
/// is dropped before reaching a terminal event.
struct CancelOnDrop {
    host: Arc<ProcessHost>,
    executor_id: String,
    execution_id: String,
    armed: bool,
}

impl CancelOnDrop {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if !self.armed || tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        let host = self.host.clone();
        let params = ProcessTaskCancelParams {
            executor_id: self.executor_id.clone(),
            execution_id: self.execution_id.clone(),
            reason: Some("task cancelled by host".to_string()),
        };
        tokio::spawn(async move {
            host.unregister_task_stream(&params.execution_id).await;
            if let Ok(value) = serde_json::to_value(&params) {
                let _: Result<serde_json::Value, _> =
                    host.request(METHOD_TASKS_CANCEL, value).await;
            }
        });
    }
}

#[async_trait::async_trait]
impl TaskExecutor for ProcessTaskExecutor {
    fn id(&self) -> String {
        self.executor_id.clone()
    }

    fn spec(&self) -> TaskSpec {
        self.spec
            .read()
            .ok()
            .and_then(|slot| slot.clone())
            .unwrap_or_else(|| self.placeholder_spec())
    }

    async fn execute(
        &self,
        ctx: TaskExecutionContext,
        input: serde_json::Value,
    ) -> anyhow::Result<TaskExecutionResult> {
        let execution_id = uuid::Uuid::new_v4().to_string();
        let mut receiver = self.host.register_task_stream(execution_id.clone()).await?;

        let ack: ProcessTaskExecuteAck = self
            .host
            .request(
                METHOD_TASKS_EXECUTE,
                serde_json::to_value(ProcessTaskExecuteParams {
                    executor_id: self.executor_id.clone(),
                    execution_id: execution_id.clone(),
                    task_id: ctx.task_id.clone(),
                    thread_id: ctx.thread_id.clone(),
                    turn_id: ctx.turn_id.clone(),
                    workspace_root: ctx.workspace_root.clone(),
                    input,
                })?,
            )
            .await
            .inspect_err(|_| {
                let host = self.host.clone();
                let execution_id = execution_id.clone();
                if tokio::runtime::Handle::try_current().is_ok() {
                    tokio::spawn(async move {
                        host.unregister_task_stream(&execution_id).await;
                    });
                }
            })?;
        anyhow::ensure!(
            ack.execution_id == execution_id,
            "process extension acknowledged execution {:?} but {:?} was requested",
            ack.execution_id,
            execution_id
        );

        let mut cancel_guard = CancelOnDrop {
            host: self.host.clone(),
            executor_id: self.executor_id.clone(),
            execution_id: execution_id.clone(),
            armed: true,
        };

        loop {
            match receiver.recv().await {
                None => {
                    cancel_guard.disarm();
                    anyhow::bail!(
                        "task executor {} closed the execution stream without a terminal event",
                        self.executor_id
                    );
                }
                Some(Err(error)) => {
                    // Child crashed mid-execution; the stream error is
                    // already redacted by the host.
                    cancel_guard.disarm();
                    return Err(error.context(format!(
                        "task executor {} failed mid-execution",
                        self.executor_id
                    )));
                }
                Some(Ok(ProcessTaskEvent::Output { stream, chunk })) => {
                    let _ = ctx.output.write(stream, chunk).await;
                }
                Some(Ok(ProcessTaskEvent::Completed { result })) => {
                    cancel_guard.disarm();
                    return Ok(result);
                }
                Some(Ok(ProcessTaskEvent::Failed { error })) => {
                    cancel_guard.disarm();
                    anyhow::bail!("task executor {} failed: {error}", self.executor_id);
                }
            }
        }
    }
}
