use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use roder_api::remote_runner::{
    RemoteRunnerSession, RunnerCommandId, RunnerCommandRequest, RunnerCommandResult,
    RunnerFileReadRequest, RunnerFileReadResult, RunnerFileWriteRequest, RunnerPortRequest,
    RunnerPortResult, RunnerSessionState, RunnerSnapshotRef,
};

/**
 * In-memory fake runner for tool tests: files live in a map keyed by the
 * runner-relative path and every command request is recorded. `test`/`rm`
 * consult the map so the apply-patch flow behaves like a real runner; other
 * programs return canned success output.
 */
#[derive(Default)]
pub(crate) struct RecordingRunnerState {
    pub(crate) files: Mutex<HashMap<String, Vec<u8>>>,
    pub(crate) commands: Mutex<Vec<RunnerCommandRequest>>,
    pub(crate) command_delay_ms: AtomicU64,
    pub(crate) command_error: Mutex<Option<String>>,
    pub(crate) command_stdout: Mutex<Option<String>>,
    pub(crate) command_stderr: Mutex<Option<String>>,
    pub(crate) command_exit_code: Mutex<Option<i32>>,
    pub(crate) cancelled_commands: Mutex<Vec<RunnerCommandId>>,
    pub(crate) cancel_never_resolves: AtomicBool,
}

pub(crate) struct RecordingRunnerSession {
    pub(crate) state: Arc<RecordingRunnerState>,
}

#[async_trait::async_trait]
impl RemoteRunnerSession for RecordingRunnerSession {
    fn state(&self) -> RunnerSessionState {
        RunnerSessionState {
            provider_id: "recording".to_string(),
            session_id: "recording-session".to_string(),
            destination_id: "recording".to_string(),
            snapshot: None,
            metadata: serde_json::Value::Null,
        }
    }

    async fn run_command(
        &self,
        request: RunnerCommandRequest,
    ) -> anyhow::Result<RunnerCommandResult> {
        self.state.commands.lock().unwrap().push(request.clone());
        let delay_ms = self.state.command_delay_ms.load(Ordering::SeqCst);
        if delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        }
        if let Some(error) = self.state.command_error.lock().unwrap().clone() {
            anyhow::bail!(error);
        }
        let exit_code = match request.program.as_str() {
            "test" => {
                let path = request.args.last().cloned().unwrap_or_default();
                if self.state.files.lock().unwrap().contains_key(&path) {
                    0
                } else {
                    1
                }
            }
            "rm" => {
                let path = request.args.last().cloned().unwrap_or_default();
                match self.state.files.lock().unwrap().remove(&path) {
                    Some(_) => 0,
                    None => 1,
                }
            }
            _ => 0,
        };
        Ok(RunnerCommandResult {
            command_id: request.command_id,
            exit_code: Some(
                self.state
                    .command_exit_code
                    .lock()
                    .unwrap()
                    .unwrap_or(exit_code),
            ),
            stdout: self
                .state
                .command_stdout
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_else(|| "remote ok\n".to_string()),
            stderr: self
                .state
                .command_stderr
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_default(),
        })
    }

    async fn cancel_command(&self, command_id: &RunnerCommandId) -> anyhow::Result<bool> {
        self.state
            .cancelled_commands
            .lock()
            .unwrap()
            .push(command_id.clone());
        if self.state.cancel_never_resolves.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        Ok(true)
    }

    async fn read_file(
        &self,
        request: RunnerFileReadRequest,
    ) -> anyhow::Result<RunnerFileReadResult> {
        let key = request.path.to_string_lossy().to_string();
        let contents = self
            .state
            .files
            .lock()
            .unwrap()
            .get(&key)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no such file on runner: {key}"))?;
        Ok(RunnerFileReadResult {
            path: request.path,
            contents,
        })
    }

    async fn write_file(&self, request: RunnerFileWriteRequest) -> anyhow::Result<()> {
        self.state
            .files
            .lock()
            .unwrap()
            .insert(request.path.to_string_lossy().to_string(), request.contents);
        Ok(())
    }

    async fn expose_port(&self, _request: RunnerPortRequest) -> anyhow::Result<RunnerPortResult> {
        anyhow::bail!("recording runner does not expose ports")
    }

    async fn snapshot(&self) -> anyhow::Result<Option<RunnerSnapshotRef>> {
        Ok(None)
    }

    async fn close(&self) -> anyhow::Result<()> {
        Ok(())
    }
}
