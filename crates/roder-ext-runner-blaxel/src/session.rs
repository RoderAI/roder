use std::path::Path;
use std::sync::Mutex;

use async_trait::async_trait;
use roder_api::remote_runner::{
    RemoteRunnerSession, RunnerCommandId, RunnerCommandRequest, RunnerCommandResult,
    RunnerFileReadRequest, RunnerFileReadResult, RunnerFileWriteRequest, RunnerPortRequest,
    RunnerPortResult, RunnerSessionState, RunnerSnapshotRef,
};
use serde_json::json;

use crate::cancellation::{
    ActiveProcessGuard, RunningProcesses, TrackedProcess, cancel_registered_process,
    cancellation_marker, shell_quote, tagged_environment,
};
use crate::client::BlaxelClient;
use crate::config::{BlaxelConfig, CleanupMode, PROVIDER_ID};

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
    working_dir: String,
    cleanup: CleanupMode,
    // Echoed into session state so resume/rejoin reconstruct the config.
    base_url: String,
    workspace: Option<String>,
    region: Option<String>,
    image: String,
    memory_mb: u32,
    inner: Mutex<Inner>,
    running_processes: RunningProcesses,
}

impl BlaxelRunnerSession {
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
            working_dir: config.working_dir.clone(),
            cleanup: config.cleanup,
            base_url: config.base_url.clone(),
            workspace: config.workspace.clone(),
            region: config.region.clone(),
            image: config.image.clone(),
            memory_mb: config.memory_mb,
            inner: Mutex::new(Inner {
                endpoint_url,
                paused,
            }),
            running_processes: Default::default(),
        }
    }

    fn endpoint(&self) -> String {
        self.inner.lock().unwrap().endpoint_url.clone()
    }

    fn set_paused(&self, paused: bool) {
        self.inner.lock().unwrap().paused = paused;
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
    ) -> anyhow::Result<TrackedProcess> {
        let tracked = TrackedProcess::new(timeout_seconds);
        let mut processes = self.running_processes.lock().unwrap();
        anyhow::ensure!(
            !processes.contains_key(command_id),
            "remote command id {command_id} is already active"
        );
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
        if let Some(workspace) = &self.workspace {
            metadata["workspace"] = json!(workspace);
        }
        if let Some(region) = &self.region {
            metadata["region"] = json!(region);
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
        let timeout_seconds = process_timeout_seconds(request.timeout_ms);
        let tracked = self.register_process(&request.command_id, timeout_seconds)?;
        let process_name = tracked.name.clone();
        let mut process_guard = ActiveProcessGuard::new(
            self.client.clone(),
            self.endpoint(),
            request.command_id.clone(),
            tracked.clone(),
            self.running_processes.clone(),
        );
        let endpoint = self.ensure_active().await?;
        if self
            .running_processes
            .lock()
            .unwrap()
            .get(&request.command_id)
            .is_some_and(|process| process.cancelled)
        {
            anyhow::bail!(
                "remote command {} was cancelled before start",
                request.command_id
            );
        }
        let command = shell_join(&request.program, &request.args);
        let command = format!(
            "if [ -e {} ]; then exit 130; fi; exec {command}",
            shell_quote(&cancellation_marker(&process_name))
        );
        let environment = tagged_environment(&request, &tracked.tag);
        let cwd = request
            .cwd
            .as_ref()
            .map(|cwd| self.resolve_path(cwd))
            .unwrap_or_else(|| self.working_dir.clone());
        let result = self
            .client
            .exec(
                &endpoint,
                &process_name,
                &command,
                Some(&cwd),
                &environment,
                timeout_seconds,
            )
            .await?;
        if result.exit_code.is_some() {
            process_guard.complete();
        }
        Ok(RunnerCommandResult {
            command_id: request.command_id,
            exit_code: result.exit_code,
            stdout: result.stdout,
            stderr: result.stderr,
        })
    }

    async fn cancel_command(&self, command_id: &RunnerCommandId) -> anyhow::Result<bool> {
        Ok(cancel_registered_process(
            self.client.clone(),
            self.endpoint(),
            self.running_processes.clone(),
            command_id.clone(),
        )
        .await)
    }

    async fn read_file(
        &self,
        request: RunnerFileReadRequest,
    ) -> anyhow::Result<RunnerFileReadResult> {
        let endpoint = self.ensure_active().await?;
        let path = self.resolve_path(&request.path);
        let contents = self.client.read_file(&endpoint, &path).await?;
        Ok(RunnerFileReadResult {
            path: request.path,
            contents,
        })
    }

    async fn write_file(&self, request: RunnerFileWriteRequest) -> anyhow::Result<()> {
        let endpoint = self.ensure_active().await?;
        let path = self.resolve_path(&request.path);
        if let Some(parent) = Path::new(&path).parent() {
            let parent = parent.to_string_lossy();
            if !parent.is_empty() {
                self.client.make_dir(&endpoint, &parent).await.ok();
            }
        }
        self.client
            .write_file(&endpoint, &path, &request.contents)
            .await
    }

    async fn expose_port(&self, request: RunnerPortRequest) -> anyhow::Result<RunnerPortResult> {
        let endpoint = self.ensure_active().await?;
        let _ = endpoint;
        let url = self
            .client
            .create_preview(&self.sandbox_name, request.port, true)
            .await?;
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
        Ok(self.state())
    }

    async fn resume(&self) -> anyhow::Result<RunnerSessionState> {
        let endpoint = self.endpoint();
        self.client.wake(&endpoint).await?;
        self.set_paused(false);
        Ok(self.state())
    }

    async fn detach(&self) -> anyhow::Result<RunnerSessionState> {
        // The sandbox stays alive; the caller persists this state to rejoin.
        Ok(self.state())
    }

    async fn close(&self) -> anyhow::Result<()> {
        if self.cleanup.deletes_on_close() {
            self.client.delete_sandbox(&self.sandbox_name).await?;
        }
        Ok(())
    }
}
