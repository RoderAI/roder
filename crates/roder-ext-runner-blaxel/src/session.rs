use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use roder_api::remote_runner::{
    RemoteRunnerSession, RunnerCommandId, RunnerCommandRequest, RunnerCommandResult,
    RunnerFileReadRequest, RunnerFileReadResult, RunnerFileWriteRequest, RunnerPortRequest,
    RunnerPortResult, RunnerSessionState, RunnerSnapshotRef,
};
use serde_json::json;

use crate::client::{BlaxelClient, HTTP_REQUEST_TIMEOUT_SECONDS};
use crate::config::{BlaxelConfig, CleanupMode, PROVIDER_ID};

const DEFAULT_PROCESS_TIMEOUT_MS: u64 = 600_000;
const MAX_PROCESS_TIMEOUT_SECONDS: u64 = 24 * 60 * 60;
const CANCEL_CREATION_WINDOW: Duration = Duration::from_secs(HTTP_REQUEST_TIMEOUT_SECONDS + 5);
const CANCEL_REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
const CANCEL_RETRY_DELAY: Duration = Duration::from_millis(100);
const DROP_CLEANUP_TIMEOUT: Duration = Duration::from_secs(HTTP_REQUEST_TIMEOUT_SECONDS + 15);
const PROCESS_REAP_GRACE_SECONDS: u64 = HTTP_REQUEST_TIMEOUT_SECONDS + 30;
pub(crate) const CANCELLATION_DIR: &str = "/tmp/roder-cancelled-processes";

#[derive(Clone)]
struct TrackedProcess {
    name: String,
    cancelled: bool,
    reap_after: Instant,
}

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
    running_processes: Arc<Mutex<HashMap<RunnerCommandId, TrackedProcess>>>,
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
            running_processes: Arc::new(Mutex::new(HashMap::new())),
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
        let process_name = format!("roder-{}", uuid::Uuid::new_v4().simple());
        let tracked = TrackedProcess {
            name: process_name,
            cancelled: false,
            // The server lease begins only after process registration. Include
            // a complete HTTP registration horizon plus a completion grace so
            // cancellation state is never reaped while a late process could
            // still start or run.
            reap_after: Instant::now()
                + Duration::from_secs(timeout_seconds + PROCESS_REAP_GRACE_SECONDS),
        };
        let mut processes = self.running_processes.lock().unwrap();
        anyhow::ensure!(
            !processes.contains_key(command_id),
            "remote command id {command_id} is already active"
        );
        processes.insert(command_id.clone(), tracked.clone());
        Ok(tracked)
    }
}

struct ActiveProcessGuard {
    client: BlaxelClient,
    endpoint: String,
    command_id: RunnerCommandId,
    process_name: String,
    reap_after: Instant,
    running_processes: Arc<Mutex<HashMap<RunnerCommandId, TrackedProcess>>>,
    armed: bool,
}

#[derive(Debug, Clone, Copy)]
struct CancellationOutcome {
    cancelled: bool,
    safe_to_forget: bool,
}

impl ActiveProcessGuard {
    fn new(
        client: BlaxelClient,
        endpoint: String,
        command_id: RunnerCommandId,
        process_name: String,
        reap_after: Instant,
        running_processes: Arc<Mutex<HashMap<RunnerCommandId, TrackedProcess>>>,
    ) -> Self {
        Self {
            client,
            endpoint,
            command_id,
            process_name,
            reap_after,
            running_processes,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
        remove_process_mapping(
            &self.running_processes,
            &self.command_id,
            &self.process_name,
        );
    }
}

impl Drop for ActiveProcessGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let client = self.client.clone();
        let endpoint = self.endpoint.clone();
        let command_id = self.command_id.clone();
        let process_name = self.process_name.clone();
        let reap_after = self.reap_after;
        let running_processes = self.running_processes.clone();
        drop(runtime.spawn(async move {
            let outcome = tokio::time::timeout(
                DROP_CLEANUP_TIMEOUT,
                cancel_process_with_retry(&client, &endpoint, &process_name),
            )
            .await
            .ok();
            if outcome.is_some_and(|outcome| outcome.safe_to_forget) {
                forget_process(
                    &client,
                    &endpoint,
                    &running_processes,
                    &command_id,
                    &process_name,
                )
                .await;
            } else {
                schedule_process_reap(
                    client,
                    endpoint,
                    running_processes,
                    command_id,
                    process_name,
                    reap_after,
                );
            }
        }));
    }
}

fn remove_process_mapping(
    running_processes: &Mutex<HashMap<RunnerCommandId, TrackedProcess>>,
    command_id: &RunnerCommandId,
    process_name: &str,
) {
    let mut processes = running_processes.lock().unwrap();
    if processes
        .get(command_id)
        .is_some_and(|current| current.name == process_name)
    {
        processes.remove(command_id);
    }
}

fn cancellation_marker(process_name: &str) -> String {
    format!("{CANCELLATION_DIR}/{process_name}")
}

async fn forget_process(
    client: &BlaxelClient,
    endpoint: &str,
    running_processes: &Mutex<HashMap<RunnerCommandId, TrackedProcess>>,
    command_id: &RunnerCommandId,
    process_name: &str,
) {
    remove_process_mapping(running_processes, command_id, process_name);
    let _ = tokio::time::timeout(
        CANCEL_REQUEST_TIMEOUT,
        client.delete_file(endpoint, &cancellation_marker(process_name)),
    )
    .await;
}

fn schedule_process_reap(
    client: BlaxelClient,
    endpoint: String,
    running_processes: Arc<Mutex<HashMap<RunnerCommandId, TrackedProcess>>>,
    command_id: RunnerCommandId,
    process_name: String,
    reap_after: Instant,
) {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return;
    };
    drop(runtime.spawn(async move {
        tokio::time::sleep_until(tokio::time::Instant::from_std(reap_after)).await;
        forget_process(
            &client,
            &endpoint,
            &running_processes,
            &command_id,
            &process_name,
        )
        .await;
    }));
}

async fn cancel_process_with_retry(
    client: &BlaxelClient,
    endpoint: &str,
    process_name: &str,
) -> CancellationOutcome {
    // A marker closes the POST-vs-DELETE registration race: if Blaxel accepts
    // the named process after all early DELETEs observed 404, the shell guard
    // exits before it executes the user command.
    let mut tombstoned = false;

    let deadline = Instant::now() + CANCEL_CREATION_WINDOW;
    loop {
        if !tombstoned {
            tombstoned = matches!(
                tokio::time::timeout(
                    CANCEL_REQUEST_TIMEOUT,
                    client.write_file(endpoint, &cancellation_marker(process_name), b"cancelled",),
                )
                .await,
                Ok(Ok(()))
            );
        }
        match tokio::time::timeout(
            CANCEL_REQUEST_TIMEOUT,
            client.kill_process(endpoint, process_name),
        )
        .await
        {
            Ok(Ok(true)) => {
                return CancellationOutcome {
                    cancelled: true,
                    safe_to_forget: true,
                };
            }
            Ok(Ok(false)) => {}
            Ok(Err(_)) => {}
            Err(_) => {}
        }
        if Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(CANCEL_RETRY_DELAY).await;
    }
    let observed = tokio::time::timeout(
        CANCEL_REQUEST_TIMEOUT,
        client.get_process(endpoint, process_name),
    )
    .await;
    match observed {
        Ok(Ok(Some(process))) if process.is_terminal() => CancellationOutcome {
            cancelled: matches!(process.status.as_deref(), Some("killed") | Some("stopped")),
            safe_to_forget: true,
        },
        Ok(Ok(None)) if tombstoned => CancellationOutcome {
            // A process accepted after this observation exits at its marker
            // guard before the user command can execute.
            cancelled: true,
            safe_to_forget: true,
        },
        _ => CancellationOutcome {
            cancelled: false,
            safe_to_forget: false,
        },
    }
}

#[cfg(test)]
mod cancellation_window_tests {
    use super::*;

    #[test]
    fn cancellation_window_covers_process_registration_request() {
        assert!(
            CANCEL_CREATION_WINDOW > Duration::from_secs(HTTP_REQUEST_TIMEOUT_SECONDS),
            "the cancellation window must outlive a process registration request"
        );
        assert!(
            DROP_CLEANUP_TIMEOUT > CANCEL_CREATION_WINDOW,
            "drop cleanup must allow the complete cancellation window"
        );
    }
}

fn process_timeout_seconds(timeout_ms: Option<u64>) -> u64 {
    let timeout_ms = timeout_ms.unwrap_or(DEFAULT_PROCESS_TIMEOUT_MS).max(1);
    (timeout_ms / 1000 + u64::from(timeout_ms % 1000 != 0)).min(MAX_PROCESS_TIMEOUT_SECONDS)
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

fn shell_quote(value: &str) -> String {
    if !value.is_empty()
        && value.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'/' | b':' | b'=')
        })
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
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
        let process_name = tracked.name;
        let mut process_guard = ActiveProcessGuard::new(
            self.client.clone(),
            self.endpoint(),
            request.command_id.clone(),
            process_name.clone(),
            tracked.reap_after,
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
                &request.env,
                timeout_seconds,
            )
            .await?;
        process_guard.disarm();
        Ok(RunnerCommandResult {
            command_id: request.command_id,
            exit_code: result.exit_code,
            stdout: result.stdout,
            stderr: result.stderr,
        })
    }

    async fn cancel_command(&self, command_id: &RunnerCommandId) -> anyhow::Result<bool> {
        let tracked = {
            let mut processes = self.running_processes.lock().unwrap();
            let Some(process) = processes.get_mut(command_id) else {
                return Ok(false);
            };
            process.cancelled = true;
            process.clone()
        };
        let outcome =
            cancel_process_with_retry(&self.client, &self.endpoint(), &tracked.name).await;
        if outcome.safe_to_forget {
            forget_process(
                &self.client,
                &self.endpoint(),
                &self.running_processes,
                command_id,
                &tracked.name,
            )
            .await;
        } else {
            schedule_process_reap(
                self.client.clone(),
                self.endpoint(),
                self.running_processes.clone(),
                command_id.clone(),
                tracked.name,
                tracked.reap_after,
            );
        }
        Ok(outcome.cancelled)
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
