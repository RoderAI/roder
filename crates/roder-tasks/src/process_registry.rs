use std::collections::BTreeMap;
use std::sync::Arc;

use roder_api::events::RoderEvent;
use roder_api::processes::{
    ProcessDescriptor, ProcessExited, ProcessFailed, ProcessId, ProcessOutput, ProcessRegistrySink,
    ProcessState, ProcessStopResult, ProcessStopped, ProcessStopper, ProcessStopping,
};
use roder_api::tasks::TaskOutputStream;
use time::OffsetDateTime;
use tokio::sync::{Mutex, broadcast};

#[derive(Debug, Clone)]
pub struct ProcessRegistryConfig {
    pub max_completed: usize,
    pub max_output_bytes: usize,
}

impl Default for ProcessRegistryConfig {
    fn default() -> Self {
        Self {
            max_completed: 64,
            max_output_bytes: 64 * 1024,
        }
    }
}

#[derive(Clone)]
pub struct ProcessRegistry {
    inner: Arc<Mutex<ProcessRegistryInner>>,
    events: broadcast::Sender<RoderEvent>,
}

#[derive(Default)]
struct ProcessRegistryInner {
    config: ProcessRegistryConfig,
    processes: BTreeMap<ProcessId, ProcessRecord>,
}

struct ProcessRecord {
    descriptor: ProcessDescriptor,
    output: Vec<ProcessOutput>,
    output_bytes: usize,
    stopper: Option<Arc<dyn ProcessStopper>>,
}

impl ProcessRegistry {
    pub fn new(config: ProcessRegistryConfig) -> Self {
        let (events, _) = broadcast::channel(1024);
        Self {
            inner: Arc::new(Mutex::new(ProcessRegistryInner {
                config,
                processes: BTreeMap::new(),
            })),
            events,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RoderEvent> {
        self.events.subscribe()
    }

    pub async fn register(
        &self,
        mut process: ProcessDescriptor,
        stopper: Option<Arc<dyn ProcessStopper>>,
    ) -> anyhow::Result<ProcessDescriptor> {
        process.updated_at = OffsetDateTime::now_utc();
        if process.started_at > process.updated_at {
            process.started_at = process.updated_at;
        }
        let registered = process.clone();
        {
            let mut inner = self.inner.lock().await;
            inner.processes.insert(
                process.process_id.clone(),
                ProcessRecord {
                    descriptor: process,
                    output: Vec::new(),
                    output_bytes: 0,
                    stopper,
                },
            );
            inner.prune_completed();
        }
        self.emit(RoderEvent::ProcessStarted(
            roder_api::processes::ProcessStarted {
                process: registered.clone(),
                timestamp: OffsetDateTime::now_utc(),
            },
        ));
        Ok(registered)
    }

    pub async fn list(&self, include_completed: bool) -> Vec<ProcessDescriptor> {
        self.inner
            .lock()
            .await
            .processes
            .values()
            .filter(|record| include_completed || !is_terminal(&record.descriptor.state))
            .map(|record| record.descriptor.clone())
            .collect()
    }

    pub async fn get(&self, process_id: &str) -> Option<ProcessDescriptor> {
        self.inner
            .lock()
            .await
            .processes
            .get(process_id)
            .map(|record| record.descriptor.clone())
    }

    pub async fn output(&self, process_id: &str) -> Vec<ProcessOutput> {
        self.inner
            .lock()
            .await
            .processes
            .get(process_id)
            .map(|record| record.output.clone())
            .unwrap_or_default()
    }

    pub async fn output_for_task(&self, task_id: &str) -> Vec<ProcessOutput> {
        self.inner
            .lock()
            .await
            .processes
            .values()
            .find(|record| record.descriptor.task_id.as_deref() == Some(task_id))
            .map(|record| record.output.clone())
            .unwrap_or_default()
    }

    pub async fn append_output(&self, output: ProcessOutput) -> anyhow::Result<()> {
        let stored = {
            let mut inner = self.inner.lock().await;
            let max_output_bytes = inner.config.max_output_bytes;
            let Some(record) = inner.processes.get_mut(&output.process_id) else {
                anyhow::bail!("unknown process {:?}", output.process_id);
            };
            let stream = output.stream.clone();
            let chunk = output.chunk.clone();
            let chunk_len = chunk.len();
            record.output.push(output.clone());
            record.output_bytes = record.output_bytes.saturating_add(chunk_len);
            while record.output_bytes > max_output_bytes {
                let Some(removed) = record.output.first().cloned() else {
                    break;
                };
                record.output.remove(0);
                record.output_bytes = record.output_bytes.saturating_sub(removed.chunk.len());
            }
            match stream {
                TaskOutputStream::Stdout => record.descriptor.stdout_tail = Some(chunk),
                TaskOutputStream::Stderr => record.descriptor.stderr_tail = Some(chunk),
                TaskOutputStream::Log => {}
            }
            record.descriptor.updated_at = OffsetDateTime::now_utc();
            output
        };
        self.emit(RoderEvent::ProcessOutput(stored));
        Ok(())
    }

    pub async fn mark_exited(
        &self,
        process_id: &str,
        exit_code: Option<i32>,
    ) -> anyhow::Result<()> {
        let process = self
            .update_terminal(process_id, ProcessState::Exited { exit_code })
            .await?;
        self.emit(RoderEvent::ProcessExited(ProcessExited {
            process,
            exit_code,
            timestamp: OffsetDateTime::now_utc(),
        }));
        Ok(())
    }

    pub async fn mark_failed(&self, process_id: &str, error: String) -> anyhow::Result<()> {
        let process = self
            .update_terminal(
                process_id,
                ProcessState::Failed {
                    error: error.clone(),
                },
            )
            .await?;
        self.emit(RoderEvent::ProcessFailed(ProcessFailed {
            process,
            error,
            timestamp: OffsetDateTime::now_utc(),
        }));
        Ok(())
    }

    pub async fn mark_stopped(
        &self,
        process_id: &str,
        reason: Option<String>,
    ) -> anyhow::Result<()> {
        let process = self
            .update_terminal(process_id, ProcessState::Stopped)
            .await?;
        self.emit(RoderEvent::ProcessStopped(ProcessStopped {
            process,
            reason,
            timestamp: OffsetDateTime::now_utc(),
        }));
        Ok(())
    }

    pub async fn stop(
        &self,
        process_id: &str,
        reason: Option<String>,
    ) -> anyhow::Result<ProcessStopResult> {
        let (stopper, process) = {
            let mut inner = self.inner.lock().await;
            let Some(record) = inner.processes.get_mut(process_id) else {
                anyhow::bail!("unknown process {process_id:?}");
            };
            if is_terminal(&record.descriptor.state) || !record.descriptor.stoppable {
                return Ok(ProcessStopResult {
                    process_id: process_id.to_string(),
                    stopped: false,
                    process: Some(record.descriptor.clone()),
                });
            }
            record.descriptor.state = ProcessState::Stopping;
            record.descriptor.updated_at = OffsetDateTime::now_utc();
            let process = record.descriptor.clone();
            (record.stopper.clone(), process)
        };
        self.emit(RoderEvent::ProcessStopping(ProcessStopping {
            process_id: process_id.to_string(),
            reason: reason.clone(),
            timestamp: OffsetDateTime::now_utc(),
        }));
        if let Some(stopper) = stopper
            && let Err(error) = stopper.stop(reason).await
        {
            let mut inner = self.inner.lock().await;
            if let Some(record) = inner.processes.get_mut(process_id)
                && matches!(record.descriptor.state, ProcessState::Stopping)
            {
                record.descriptor.state = ProcessState::Running;
                record.descriptor.updated_at = OffsetDateTime::now_utc();
            }
            return Err(error);
        }
        Ok(ProcessStopResult {
            process_id: process_id.to_string(),
            stopped: true,
            process: Some(process),
        })
    }

    pub async fn stop_all(&self, reason: Option<String>) -> Vec<ProcessStopResult> {
        let process_ids = {
            self.inner
                .lock()
                .await
                .processes
                .values()
                .filter(|record| {
                    record.descriptor.stoppable && !is_terminal(&record.descriptor.state)
                })
                .map(|record| record.descriptor.process_id.clone())
                .collect::<Vec<_>>()
        };
        let mut results = Vec::new();
        for process_id in process_ids {
            match self.stop(&process_id, reason.clone()).await {
                Ok(result) => results.push(result),
                Err(_) => results.push(ProcessStopResult {
                    process_id,
                    stopped: false,
                    process: None,
                }),
            }
        }
        results
    }

    pub async fn append_task_output(
        &self,
        task_id: &str,
        stream: TaskOutputStream,
        chunk: String,
        dropped_bytes: u64,
        thread_id: Option<String>,
        turn_id: Option<String>,
    ) -> anyhow::Result<()> {
        let process_id = {
            self.inner
                .lock()
                .await
                .processes
                .values()
                .find(|record| record.descriptor.task_id.as_deref() == Some(task_id))
                .map(|record| record.descriptor.process_id.clone())
        };
        if let Some(process_id) = process_id {
            self.append_output(ProcessOutput {
                process_id,
                stream,
                chunk,
                dropped_bytes,
                thread_id,
                turn_id,
                timestamp: OffsetDateTime::now_utc(),
            })
            .await?;
        }
        Ok(())
    }

    async fn update_terminal(
        &self,
        process_id: &str,
        state: ProcessState,
    ) -> anyhow::Result<ProcessDescriptor> {
        let process = {
            let mut inner = self.inner.lock().await;
            let Some(record) = inner.processes.get_mut(process_id) else {
                anyhow::bail!("unknown process {process_id:?}");
            };
            if is_terminal(&record.descriptor.state) {
                return Ok(record.descriptor.clone());
            }
            record.descriptor.state = state;
            record.descriptor.stoppable = false;
            record.descriptor.updated_at = OffsetDateTime::now_utc();
            record.stopper = None;
            let process = record.descriptor.clone();
            inner.prune_completed();
            process
        };
        Ok(process)
    }

    fn emit(&self, event: RoderEvent) {
        let _ = self.events.send(event);
    }
}

impl Default for ProcessRegistry {
    fn default() -> Self {
        Self::new(ProcessRegistryConfig::default())
    }
}

#[async_trait::async_trait]
impl ProcessRegistrySink for ProcessRegistry {
    async fn register_process(
        &self,
        process: ProcessDescriptor,
        stopper: Option<Arc<dyn ProcessStopper>>,
    ) -> anyhow::Result<ProcessDescriptor> {
        self.register(process, stopper).await
    }

    async fn append_process_output(&self, output: ProcessOutput) -> anyhow::Result<()> {
        self.append_output(output).await
    }

    async fn mark_process_exited(
        &self,
        process_id: &str,
        exit_code: Option<i32>,
    ) -> anyhow::Result<()> {
        self.mark_exited(process_id, exit_code).await
    }

    async fn mark_process_failed(&self, process_id: &str, error: String) -> anyhow::Result<()> {
        self.mark_failed(process_id, error).await
    }

    async fn mark_process_stopped(
        &self,
        process_id: &str,
        reason: Option<String>,
    ) -> anyhow::Result<()> {
        self.mark_stopped(process_id, reason).await
    }
}

impl ProcessRegistryInner {
    fn prune_completed(&mut self) {
        let completed = self
            .processes
            .values()
            .filter(|record| is_terminal(&record.descriptor.state))
            .count();
        if completed <= self.config.max_completed {
            return;
        }
        let remove_count = completed - self.config.max_completed;
        let mut terminal = self
            .processes
            .values()
            .filter(|record| is_terminal(&record.descriptor.state))
            .map(|record| {
                (
                    record.descriptor.updated_at,
                    record.descriptor.process_id.clone(),
                )
            })
            .collect::<Vec<_>>();
        terminal.sort_by_key(|(updated_at, _)| *updated_at);
        for (_, process_id) in terminal.into_iter().take(remove_count) {
            self.processes.remove(&process_id);
        }
    }
}

fn is_terminal(state: &ProcessState) -> bool {
    matches!(
        state,
        ProcessState::Exited { .. } | ProcessState::Failed { .. } | ProcessState::Stopped
    )
}
