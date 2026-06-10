use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use roder_api::remote_runner::{
    RemoteRunnerSession, RunnerCommandRequest, RunnerCommandResult, RunnerFileReadRequest,
    RunnerFileReadResult, RunnerFileWriteRequest, RunnerPortRequest, RunnerPortResult,
    RunnerSessionState, RunnerSnapshotRef,
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
            exit_code: Some(exit_code),
            stdout: "remote ok\n".to_string(),
            stderr: String::new(),
        })
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
