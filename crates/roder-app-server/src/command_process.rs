use std::process::ExitStatus;
use std::sync::Arc;
use std::time::Duration;

use roder_api::processes::{
    ProcessDescriptor, ProcessOrigin, ProcessOutput, ProcessState, ProcessStopper, command_summary,
};
use roder_api::tasks::TaskOutputStream;
use roder_protocol::{CommandExecParams, JsonRpcError};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::{Mutex, oneshot};

use crate::AppServer;

pub(crate) struct RegisteredCommandOutput {
    pub(crate) status: ExitStatus,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
}

pub(crate) async fn run_registered_command(
    server: &AppServer,
    process_id: &str,
    mut command: Command,
    params: &CommandExecParams,
) -> Result<RegisteredCommandOutput, JsonRpcError> {
    command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(internal_error)?;
    let pid = child.id();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (stop_tx, stop_rx) = oneshot::channel();
    let process_registry = server.tasks.processes();
    let now = time::OffsetDateTime::now_utc();
    process_registry
        .register(
            ProcessDescriptor {
                process_id: process_id.to_string(),
                origin: ProcessOrigin::CommandExec,
                state: ProcessState::Running,
                command: params.command.clone(),
                command_summary: command_summary(&params.command),
                cwd: params.cwd.clone(),
                pid,
                task_id: None,
                thread_id: Some("app-server".to_string()),
                turn_id: Some("command/exec".to_string()),
                runner_destination_id: None,
                runner_session_id: None,
                stoppable: true,
                started_at: now,
                updated_at: now,
                stdout_tail: None,
                stderr_tail: None,
            },
            Some(Arc::new(CommandStopper::new(stop_tx))),
        )
        .await
        .map_err(internal_error)?;

    let stdout_task = tokio::spawn(read_pipe(stdout));
    let stderr_task = tokio::spawn(read_pipe(stderr));
    let wait_future = async {
        tokio::select! {
            status = child.wait() => {
                status.map(|status| (status, CommandExitKind::Exited))
            }
            _ = stop_rx => {
                child.kill().await?;
                child.wait().await.map(|status| (status, CommandExitKind::Stopped))
            }
        }
    };
    let (status, exit_kind) = if params.disable_timeout {
        wait_future.await.map_err(internal_error)?
    } else {
        let timeout_ms = params.timeout_ms.unwrap_or(30_000);
        match tokio::time::timeout(Duration::from_millis(timeout_ms), wait_future).await {
            Ok(result) => result.map_err(internal_error)?,
            Err(_) => {
                process_registry
                    .mark_failed(
                        process_id,
                        format!("command timed out after {timeout_ms}ms"),
                    )
                    .await
                    .map_err(internal_error)?;
                return Err(JsonRpcError {
                    code: -32000,
                    message: format!("command timed out after {timeout_ms}ms"),
                    data: None,
                });
            }
        }
    };
    let stdout = stdout_task
        .await
        .map_err(internal_error)?
        .map_err(internal_error)?;
    let stderr = stderr_task
        .await
        .map_err(internal_error)?
        .map_err(internal_error)?;
    append_command_process_output(
        &process_registry,
        process_id,
        TaskOutputStream::Stdout,
        &stdout,
    )
    .await?;
    append_command_process_output(
        &process_registry,
        process_id,
        TaskOutputStream::Stderr,
        &stderr,
    )
    .await?;
    match exit_kind {
        CommandExitKind::Exited => {
            process_registry
                .mark_exited(process_id, status.code())
                .await
                .map_err(internal_error)?;
        }
        CommandExitKind::Stopped => {
            process_registry
                .mark_stopped(process_id, Some("stop requested".to_string()))
                .await
                .map_err(internal_error)?;
        }
    }
    Ok(RegisteredCommandOutput {
        status,
        stdout,
        stderr,
    })
}

enum CommandExitKind {
    Exited,
    Stopped,
}

struct CommandStopper {
    stop_tx: Mutex<Option<oneshot::Sender<Option<String>>>>,
}

impl CommandStopper {
    fn new(stop_tx: oneshot::Sender<Option<String>>) -> Self {
        Self {
            stop_tx: Mutex::new(Some(stop_tx)),
        }
    }
}

#[async_trait::async_trait]
impl ProcessStopper for CommandStopper {
    async fn stop(&self, reason: Option<String>) -> anyhow::Result<()> {
        if let Some(stop_tx) = self.stop_tx.lock().await.take() {
            let _ = stop_tx.send(reason);
        }
        Ok(())
    }
}

async fn read_pipe(pipe: Option<impl tokio::io::AsyncRead + Unpin>) -> std::io::Result<Vec<u8>> {
    let Some(mut pipe) = pipe else {
        return Ok(Vec::new());
    };
    let mut output = Vec::new();
    pipe.read_to_end(&mut output).await?;
    Ok(output)
}

async fn append_command_process_output(
    process_registry: &roder_tasks::ProcessRegistry,
    process_id: &str,
    stream: TaskOutputStream,
    bytes: &[u8],
) -> Result<(), JsonRpcError> {
    if bytes.is_empty() {
        return Ok(());
    }
    process_registry
        .append_output(ProcessOutput {
            process_id: process_id.to_string(),
            stream,
            chunk: String::from_utf8_lossy(bytes).to_string(),
            dropped_bytes: 0,
            thread_id: Some("app-server".to_string()),
            turn_id: Some("command/exec".to_string()),
            timestamp: time::OffsetDateTime::now_utc(),
        })
        .await
        .map_err(internal_error)
}

fn internal_error(err: impl std::fmt::Display) -> JsonRpcError {
    let details = format!("{err:#}");
    JsonRpcError {
        code: -32000,
        message: details.clone(),
        data: Some(serde_json::json!({ "details": details })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use roder_api::policy_mode::PolicyMode;
    use roder_core::Runtime;

    #[tokio::test]
    async fn command_exec_process_tracking_lists_and_stops_running_command() {
        let runtime = Arc::new(Runtime::fake().unwrap());
        runtime
            .set_policy_mode(
                PolicyMode::AcceptAll,
                Some("test command process tracking".to_string()),
            )
            .await
            .unwrap();
        let server = Arc::new(AppServer::new(runtime));
        let process_registry = server.tasks.processes();
        let request_server = Arc::clone(&server);
        let request = tokio::spawn(async move {
            request_server
                .handle_command_exec(CommandExecParams {
                    command: vec!["sh".to_string(), "-c".to_string(), "sleep 5".to_string()],
                    process_id: Some("command-process-test".to_string()),
                    tty: false,
                    stream_stdin: false,
                    stream_stdout_stderr: false,
                    output_bytes_cap: None,
                    disable_output_cap: false,
                    disable_timeout: false,
                    timeout_ms: Some(10_000),
                    cwd: None,
                    env: None,
                    size: None,
                    sandbox_policy: None,
                })
                .await
        });

        for _ in 0..50 {
            if process_registry.get("command-process-test").await.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let active = process_registry.list(false).await;
        assert!(
            active
                .iter()
                .any(|process| process.process_id == "command-process-test"
                    && matches!(process.state, ProcessState::Running))
        );

        let stopped = process_registry
            .stop("command-process-test", Some("test stop".to_string()))
            .await
            .unwrap();
        assert!(stopped.stopped);

        let response = request.await.unwrap().unwrap();
        assert_eq!(response["exitCode"], -1);
        let process = process_registry.get("command-process-test").await.unwrap();
        assert!(matches!(process.state, ProcessState::Stopped));
    }
}
