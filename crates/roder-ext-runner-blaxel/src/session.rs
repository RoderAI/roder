use std::collections::HashSet;
use std::path::Path;
use std::sync::Mutex;

use async_trait::async_trait;
use roder_api::remote_runner::{
    RemoteRunnerSession, RunnerCommandId, RunnerCommandObservation, RunnerCommandRequest,
    RunnerCommandResult, RunnerFileReadRequest, RunnerFileReadResult, RunnerFileWriteRequest,
    RunnerPortRequest, RunnerPortResult, RunnerSessionState, RunnerSnapshotRef,
};
use serde_json::json;

use crate::cancellation::{
    ActiveProcessGuard, RunningProcesses, TrackedProcess, cancel_registered_process,
    cancellation_marker, shell_quote, tagged_environment,
};
use crate::client::BlaxelClient;
use crate::config::{BlaxelConfig, CleanupMode, PROVIDER_ID, SandboxLifecycle};
use crate::standby::StandbyGrace;

mod commands;

const DEFAULT_PROCESS_TIMEOUT_MS: u64 = 600_000;
const MAX_PROCESS_TIMEOUT_SECONDS: u64 = 24 * 60 * 60;

/// Mutable session runtime guarded by a mutex. Never held across `.await`.
struct Inner {
    endpoint_url: String,
    paused: bool,
}

/// A live (or rejoinable) Blaxel sandbox bound to a Roder thread.
pub struct BlaxelRunnerSession {
    client: BlaxelClient,
    destination_id: String,
    sandbox_name: String,
    external_id: Option<String>,
    fork_allocation: Option<crate::forks::ForkAllocation>,
    working_dir: String,
    cleanup: CleanupMode,
    // Echoed into session state so resume/rejoin reconstruct the config.
    base_url: String,
    workspace: Option<String>,
    explicit_workspace: bool,
    region: Option<String>,
    egress_gateway: Option<String>,
    egress_workspace: Option<String>,
    image: String,
    memory_mb: u32,
    ports: Vec<crate::SandboxPort>,
    ttl: Option<String>,
    standby_after_seconds: Option<u64>,
    lifecycle: Option<SandboxLifecycle>,
    inner: Mutex<Inner>,
    running_processes: RunningProcesses,
    durable_commands: Mutex<HashSet<RunnerCommandId>>,
    standby_grace: StandbyGrace,
}

impl BlaxelRunnerSession {
    pub(crate) fn with_explicit_workspace(mut self, explicit: bool) -> Self {
        self.explicit_workspace = explicit;
        self
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        client: BlaxelClient,
        config: &BlaxelConfig,
        destination_id: String,
        sandbox_name: String,
        external_id: Option<String>,
        endpoint_url: String,
        paused: bool,
    ) -> Self {
        Self {
            client,
            destination_id,
            sandbox_name,
            external_id,
            fork_allocation: None,
            working_dir: config.working_dir.clone(),
            cleanup: config.cleanup,
            base_url: config.base_url.clone(),
            workspace: config.workspace.clone(),
            explicit_workspace: false,
            region: config.region.clone(),
            egress_gateway: config.egress_gateway.clone(),
            egress_workspace: config.egress_workspace.clone(),
            image: config.image.clone(),
            memory_mb: config.memory_mb,
            ports: config.ports.clone(),
            ttl: config.ttl.clone(),
            standby_after_seconds: config.standby_after_seconds,
            lifecycle: config.lifecycle.clone(),
            inner: Mutex::new(Inner {
                endpoint_url,
                paused,
            }),
            running_processes: Default::default(),
            durable_commands: Default::default(),
            standby_grace: StandbyGrace::new(config.standby_after_seconds),
        }
    }

    pub(crate) fn with_fork_allocation(mut self, allocation: crate::forks::ForkAllocation) -> Self {
        self.fork_allocation = Some(allocation);
        self
    }

    fn endpoint(&self) -> String {
        self.inner.lock().unwrap().endpoint_url.clone()
    }

    fn set_paused(&self, paused: bool) {
        self.inner.lock().unwrap().paused = paused;
    }

    fn is_paused(&self) -> bool {
        self.inner.lock().unwrap().paused
    }

    /// Return the endpoint, waking the sandbox first if it was paused.
    async fn ensure_active(&self) -> anyhow::Result<String> {
        let (endpoint, paused) = {
            let inner = self.inner.lock().unwrap();
            (inner.endpoint_url.clone(), inner.paused)
        };
        if paused {
            self.client.wake(&endpoint).await?;
            self.set_paused(false);
        }
        Ok(endpoint)
    }

    /// The grace lease keeps running through new work; finishing the work
    /// replaces it only once it is no longer fresh.
    async fn begin_operation(&self) -> anyhow::Result<String> {
        self.ensure_active().await
    }

    async fn finish_operation(&self, endpoint: &str) -> anyhow::Result<()> {
        if self.is_paused() {
            return Ok(());
        }
        self.standby_grace.refresh(&self.client, endpoint).await
    }

    /// Re-arm grace after a completed native API attempt. If both the native
    /// operation and grace refresh fail, preserve the operation's primary error.
    async fn finish_attempt<T>(
        &self,
        endpoint: &str,
        result: anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        match result {
            Ok(value) => {
                self.finish_operation(endpoint).await?;
                Ok(value)
            }
            Err(error) => {
                let _ = self.finish_operation(endpoint).await;
                Err(error)
            }
        }
    }

    /// Start the initial grace period after create/rejoin setup completes.
    pub(crate) async fn refresh_standby_grace(&self) -> anyhow::Result<()> {
        let endpoint = self.endpoint();
        self.finish_operation(&endpoint).await
    }

    fn resolve_path(&self, path: &Path) -> String {
        if path.is_absolute() {
            path.to_string_lossy().to_string()
        } else {
            format!(
                "{}/{}",
                self.working_dir.trim_end_matches('/'),
                path.to_string_lossy().trim_start_matches('/')
            )
        }
    }

    fn register_process(
        &self,
        command_id: &RunnerCommandId,
        timeout_seconds: u64,
        durable: bool,
    ) -> anyhow::Result<TrackedProcess> {
        let mut tracked = TrackedProcess::new(timeout_seconds);
        if durable {
            tracked.name = commands::durable_process_name(command_id)?;
            anyhow::ensure!(
                self.durable_commands
                    .lock()
                    .unwrap()
                    .insert(command_id.clone()),
                "Durable command was already attempted"
            );
        }
        let mut processes = self.running_processes.lock().unwrap();
        if let Some(current) = processes.get(command_id) {
            anyhow::ensure!(
                current.can_be_replaced(),
                "remote command id {command_id} is already active"
            );
        }
        processes.insert(command_id.clone(), tracked.clone());
        Ok(tracked)
    }
}

fn process_timeout_seconds(timeout_ms: Option<u64>) -> u64 {
    let timeout_ms = timeout_ms.unwrap_or(DEFAULT_PROCESS_TIMEOUT_MS).max(1);
    (timeout_ms / 1000 + u64::from(!timeout_ms.is_multiple_of(1000)))
        .min(MAX_PROCESS_TIMEOUT_SECONDS)
}

/// Join a program and its args into a single shell command, quoting args that
/// contain characters the shell would otherwise interpret.
fn shell_join(program: &str, args: &[String]) -> String {
    let mut command = shell_quote(program);
    for arg in args {
        command.push(' ');
        command.push_str(&shell_quote(arg));
    }
    command
}

#[async_trait]
impl RemoteRunnerSession for BlaxelRunnerSession {
    fn state(&self) -> RunnerSessionState {
        let (endpoint_url, paused) = {
            let inner = self.inner.lock().unwrap();
            (inner.endpoint_url.clone(), inner.paused)
        };
        let mut metadata = json!({
            "base_url": self.base_url,
            "sandbox_name": self.sandbox_name,
            "sandbox_endpoint_url": endpoint_url,
            "working_dir": self.working_dir,
            "image": self.image,
            "memory": self.memory_mb,
            "cleanup": self.cleanup.as_str(),
            "paused": paused,
        });
        if self.explicit_workspace {
            metadata["explicit_workspace"] = json!(true);
        }
        if self.fork_allocation.is_some() {
            metadata["fork_adopted"] = json!(true);
        }
        if let Some(workspace) = &self.workspace {
            metadata["workspace"] = json!(workspace);
        }
        if let Some(region) = &self.region {
            metadata["region"] = json!(region);
        }
        if let Some(gateway) = &self.egress_gateway {
            metadata["egress_gateway"] = json!(gateway);
        }
        if let Some(workspace) = &self.egress_workspace {
            metadata["egress_workspace"] = json!(workspace);
        }
        if let Some(ttl) = &self.ttl {
            metadata["ttl"] = json!(ttl);
        }
        if !self.ports.is_empty() {
            metadata["ports"] = json!(self.ports);
        }
        if let Some(seconds) = self.standby_after_seconds {
            metadata["standby_after"] = json!(format!("{seconds}s"));
        }
        if let Some(lifecycle) = &self.lifecycle {
            metadata["lifecycle"] = lifecycle.config_value();
        }
        if let Some(external_id) = &self.external_id {
            metadata["external_id"] = json!(external_id);
        }
        RunnerSessionState {
            provider_id: PROVIDER_ID.to_string(),
            session_id: self.sandbox_name.clone(),
            destination_id: self.destination_id.clone(),
            snapshot: None,
            metadata,
        }
    }

    async fn run_command(
        &self,
        request: RunnerCommandRequest,
    ) -> anyhow::Result<RunnerCommandResult> {
        self.execute_command(request, false).await
    }

    async fn run_durable_command(
        &self,
        request: RunnerCommandRequest,
    ) -> anyhow::Result<RunnerCommandResult> {
        self.check_durable_target()?;
        self.execute_command(request, true).await
    }

    async fn observe_durable_command(
        &self,
        command_id: &RunnerCommandId,
    ) -> anyhow::Result<RunnerCommandObservation> {
        self.check_durable_target()?;
        let name = commands::durable_process_name(command_id)?;
        self.client.observe_process(&self.endpoint(), &name).await
    }

    async fn cancel_command(&self, command_id: &RunnerCommandId) -> anyhow::Result<bool> {
        let endpoint = self.begin_operation().await?;
        let cancelled = cancel_registered_process(
            self.client.clone(),
            endpoint.clone(),
            self.running_processes.clone(),
            command_id.clone(),
        )
        .await;
        self.finish_operation(&endpoint).await?;
        Ok(cancelled)
    }

    async fn read_file(
        &self,
        request: RunnerFileReadRequest,
    ) -> anyhow::Result<RunnerFileReadResult> {
        let endpoint = self.begin_operation().await?;
        let path = self.resolve_path(&request.path);
        let result = self.client.read_file(&endpoint, &path).await;
        let contents = self.finish_attempt(&endpoint, result).await?;
        Ok(RunnerFileReadResult {
            path: request.path,
            contents,
        })
    }

    async fn write_file(&self, request: RunnerFileWriteRequest) -> anyhow::Result<()> {
        let endpoint = self.begin_operation().await?;
        let path = self.resolve_path(&request.path);
        if let Some(parent) = Path::new(&path).parent() {
            let parent = parent.to_string_lossy();
            if !parent.is_empty() {
                self.client.make_dir(&endpoint, &parent).await.ok();
            }
        }
        let result = self
            .client
            .write_file(&endpoint, &path, &request.contents)
            .await;
        self.finish_attempt(&endpoint, result).await
    }

    async fn expose_port(&self, request: RunnerPortRequest) -> anyhow::Result<RunnerPortResult> {
        let endpoint = self.begin_operation().await?;
        let result = self
            .client
            .create_preview(&self.sandbox_name, request.port, true)
            .await;
        let url = self.finish_attempt(&endpoint, result).await?;
        Ok(RunnerPortResult {
            port: request.port,
            url,
        })
    }

    async fn snapshot(&self) -> anyhow::Result<Option<RunnerSnapshotRef>> {
        // Blaxel persists memory + filesystem automatically on standby; there
        // is no explicit snapshot artifact to reference.
        Ok(None)
    }

    async fn pause(&self) -> anyhow::Result<RunnerSessionState> {
        // Blaxel scales to standby once connections drop; mark intent so the
        // next operation wakes the sandbox.
        self.set_paused(true);
        let endpoint = self.endpoint();
        if let Err(error) = self.standby_grace.cancel(&self.client, &endpoint).await {
            self.set_paused(false);
            return Err(error);
        }
        Ok(self.state())
    }

    async fn resume(&self) -> anyhow::Result<RunnerSessionState> {
        let endpoint = self.endpoint();
        let result = self.client.wake(&endpoint).await;
        if result.is_ok() {
            self.set_paused(false);
        }
        self.finish_attempt(&endpoint, result).await?;
        Ok(self.state())
    }

    async fn detach(&self) -> anyhow::Result<RunnerSessionState> {
        // The sandbox stays alive; the caller persists this state to rejoin.
        Ok(self.state())
    }

    async fn close(&self) -> anyhow::Result<()> {
        self.set_paused(true);
        if self.cleanup.deletes_on_close()
            && let Some(allocation) = &self.fork_allocation
        {
            allocation.client.delete_child(&allocation.request).await?;
            return Ok(());
        }
        let endpoint = self.endpoint();
        let grace_result = self.standby_grace.cancel(&self.client, &endpoint).await;
        if self.cleanup.deletes_on_close() {
            // Deleting the sandbox subsumes a failed process kill, and cleanup
            // must not be skipped because the grace endpoint was unavailable.
            self.client.delete_sandbox(&self.sandbox_name).await?;
            return Ok(());
        }
        grace_result
    }
}
