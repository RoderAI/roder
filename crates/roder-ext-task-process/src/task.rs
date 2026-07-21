use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

#[cfg(unix)]
use std::os::unix::process::CommandExt;

use anyhow::{Context, bail};
use roder_api::processes::{
    ProcessDescriptor, ProcessOrigin, ProcessState, ProcessStopper, command_summary,
};
use roder_api::remote_runner::RunnerCommandRequest;
use roder_api::tasks::{
    TaskExecutionContext, TaskExecutionResult, TaskExecutor, TaskOutputStream, TaskSpec,
};
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{Mutex, oneshot};

pub const PROCESS_TASK_EXECUTOR_ID: &str = "process";

#[derive(Debug, Clone, Deserialize)]
struct ProcessTaskInput {
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    env_overrides: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct ProcessTaskExecutor;

#[async_trait::async_trait]
impl TaskExecutor for ProcessTaskExecutor {
    fn id(&self) -> String {
        PROCESS_TASK_EXECUTOR_ID.to_string()
    }

    fn spec(&self) -> TaskSpec {
        TaskSpec {
            kind: PROCESS_TASK_EXECUTOR_ID.to_string(),
            description: "Run a background process inside the workspace.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["command"],
                "properties": {
                    "command": { "type": "string" },
                    "args": { "type": "array", "items": { "type": "string" } },
                    "cwd": { "type": "string" },
                    "env_overrides": {
                        "type": "object",
                        "additionalProperties": { "type": "string" }
                    }
                },
                "additionalProperties": false
            }),
            default_timeout_seconds: None,
            metadata: serde_json::json!({ "category": "process" }),
        }
    }

    async fn execute(
        &self,
        ctx: TaskExecutionContext,
        input: serde_json::Value,
    ) -> anyhow::Result<TaskExecutionResult> {
        let input: ProcessTaskInput =
            serde_json::from_value(input).context("deserialize process task input")?;
        if input.command.trim().is_empty() {
            bail!("process task command must not be empty");
        }

        if ctx.runner_session.is_some() {
            return execute_remote_process_task(ctx, input).await;
        }

        let cwd = resolve_cwd(ctx.workspace_root.as_deref(), input.cwd.as_deref())?;
        let command_parts = std::iter::once(input.command.clone())
            .chain(input.args.clone())
            .collect::<Vec<_>>();
        let mut command = Command::new(&input.command);
        command
            .args(&input.args)
            .current_dir(&cwd)
            .envs(&input.env_overrides)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        // Run the child as a process-group leader. Shutdown signals can then
        // cover descendants created by a shell task without touching the host
        // process group.
        unsafe {
            command.as_std_mut().pre_exec(|| {
                if libc::setpgid(0, 0) == -1 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }

        let mut child = command
            .spawn()
            .with_context(|| format!("spawn process task {:?}", input.command))?;
        let pid = child.id();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let output = Arc::new(ctx.output);
        let process_id = format!("task-{}", ctx.task_id);
        let (stop_tx, stop_rx) = oneshot::channel();
        if let Some(registry) = ctx.process_registry.as_ref() {
            registry
                .register_process(
                    ProcessDescriptor {
                        process_id: process_id.clone(),
                        origin: ProcessOrigin::BackgroundTask,
                        state: ProcessState::Running,
                        command: command_parts.clone(),
                        command_summary: command_summary(&command_parts),
                        cwd: Some(cwd.display().to_string()),
                        pid,
                        task_id: Some(ctx.task_id.clone()),
                        thread_id: ctx.thread_id.clone(),
                        turn_id: ctx.turn_id.clone(),
                        runner_destination_id: None,
                        runner_session_id: None,
                        stoppable: true,
                        started_at: time::OffsetDateTime::now_utc(),
                        updated_at: time::OffsetDateTime::now_utc(),
                        stdout_tail: None,
                        stderr_tail: None,
                    },
                    Some(Arc::new(ChannelProcessStopper::new(stop_tx))),
                )
                .await?;
        }

        let stdout_task = tokio::spawn(stream_pipe(
            stdout,
            TaskOutputStream::Stdout,
            Arc::clone(&output),
        ));
        let stderr_task = tokio::spawn(stream_pipe(
            stderr,
            TaskOutputStream::Stderr,
            Arc::clone(&output),
        ));
        let (status, stopped_by_registry) = tokio::select! {
            status = child.wait() => (status.context("wait for process task")?, false),
            _ = stop_rx => {
                let (status, stop_reason) = stop_local_process(
                    &mut child,
                    ctx.process_grace_timeout,
                    ctx.process_kill_timeout,
                ).await?;
                if let Some(registry) = ctx.process_registry.as_ref() {
                    registry
                        .mark_process_stopped(&process_id, Some(stop_reason))
                        .await?;
                }
                (status, true)
            }
        };
        stdout_task.await.context("join stdout reader")??;
        stderr_task.await.context("join stderr reader")??;
        if let Some(registry) = ctx.process_registry.as_ref()
            && !stopped_by_registry
        {
            let _ = registry
                .mark_process_exited(&process_id, status.code())
                .await;
        }

        Ok(TaskExecutionResult {
            exit_code: status.code(),
            payload: serde_json::json!({
                "command": input.command,
                "args": input.args,
                "cwd": cwd.display().to_string(),
                "success": status.success(),
            }),
        })
    }
}

/// Stops a directly owned local child with a bounded graceful signal on Unix,
/// then a bounded forced kill/reap fallback. The process registry is updated
/// only after this function returns, so `Stopped` never means merely "signal
/// sent" for a local child.
async fn stop_local_process(
    child: &mut tokio::process::Child,
    grace_timeout: std::time::Duration,
    kill_timeout: std::time::Duration,
) -> anyhow::Result<(std::process::ExitStatus, String)> {
    #[cfg(unix)]
    if let Some(pid) = child.id()
        && send_signal_to_process_group(pid, libc::SIGTERM)
    {
        match tokio::time::timeout(grace_timeout, child.wait()).await {
            Ok(status) => {
                return Ok((
                    status.context("wait for SIGTERM-stopped process task")?,
                    "stop requested (SIGTERM)".to_string(),
                ));
            }
            Err(_) => {}
        }
    }

    let status = tokio::time::timeout(kill_timeout, async {
        let sent_group_kill = {
            #[cfg(unix)]
            {
                child
                    .id()
                    .is_some_and(|pid| send_signal_to_process_group(pid, libc::SIGKILL))
            }
            #[cfg(not(unix))]
            {
                false
            }
        };
        if !sent_group_kill {
            match child.kill().await {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::InvalidInput => {}
                Err(error) => return Err(error),
            }
        }
        child.wait().await
    })
    .await
    .map_err(|_| anyhow::anyhow!("timed out waiting for forced process kill"))?
    .context("kill stopped process task")?;
    Ok((
        status,
        "stop requested (forced kill after grace)".to_string(),
    ))
}

#[cfg(unix)]
fn send_signal_to_process_group(pid: u32, signal: libc::c_int) -> bool {
    // SAFETY: the child is made a process-group leader in `pre_exec`. A
    // negative PID targets only that child-owned group, never Roder's group.
    unsafe { libc::kill(-(pid as libc::pid_t), signal) == 0 }
}

struct ChannelProcessStopper {
    stop_tx: Mutex<Option<oneshot::Sender<Option<String>>>>,
}

impl ChannelProcessStopper {
    fn new(stop_tx: oneshot::Sender<Option<String>>) -> Self {
        Self {
            stop_tx: Mutex::new(Some(stop_tx)),
        }
    }
}

#[async_trait::async_trait]
impl ProcessStopper for ChannelProcessStopper {
    async fn stop(&self, reason: Option<String>) -> anyhow::Result<()> {
        if let Some(stop_tx) = self.stop_tx.lock().await.take() {
            let _ = stop_tx.send(reason);
        }
        Ok(())
    }
}

async fn execute_remote_process_task(
    ctx: TaskExecutionContext,
    input: ProcessTaskInput,
) -> anyhow::Result<TaskExecutionResult> {
    let Some(session) = ctx.runner_session.clone() else {
        bail!("remote process task requires runner session");
    };
    let command_id = ctx.task_id.clone();
    let command_parts = std::iter::once(input.command.clone())
        .chain(input.args.clone())
        .collect::<Vec<_>>();
    let state = session.state();
    let process_id = format!("remote-{}", ctx.task_id);
    if let Some(registry) = ctx.process_registry.as_ref() {
        registry
            .register_process(
                ProcessDescriptor {
                    process_id: process_id.clone(),
                    origin: ProcessOrigin::RemoteRunner,
                    state: ProcessState::Running,
                    command: command_parts.clone(),
                    command_summary: command_summary(&command_parts),
                    cwd: input.cwd.clone(),
                    pid: None,
                    task_id: Some(ctx.task_id.clone()),
                    thread_id: ctx.thread_id.clone(),
                    turn_id: ctx.turn_id.clone(),
                    runner_destination_id: ctx
                        .runner_destination
                        .as_ref()
                        .map(|destination| destination.id.clone())
                        .or_else(|| Some(state.destination_id.clone())),
                    runner_session_id: Some(state.session_id.clone()),
                    stoppable: true,
                    started_at: time::OffsetDateTime::now_utc(),
                    updated_at: time::OffsetDateTime::now_utc(),
                    stdout_tail: None,
                    stderr_tail: None,
                },
                Some(Arc::new(RemoteCommandStopper {
                    session: Arc::clone(&session),
                    command_id: command_id.clone(),
                })),
            )
            .await?;
    }
    let output = match session
        .run_command(RunnerCommandRequest {
            command_id: command_id.clone(),
            program: input.command.clone(),
            args: input.args.clone(),
            cwd: input.cwd.as_deref().map(PathBuf::from),
            env: input.env_overrides.clone().into_iter().collect(),
        })
        .await
    {
        Ok(output) => output,
        Err(error) => {
            if let Some(registry) = ctx.process_registry.as_ref() {
                let _ = registry
                    .mark_process_failed(&process_id, error.to_string())
                    .await;
            }
            return Err(error);
        }
    };
    if !output.stdout.is_empty() {
        ctx.output
            .write(TaskOutputStream::Stdout, output.stdout.clone())
            .await?;
    }
    if !output.stderr.is_empty() {
        ctx.output
            .write(TaskOutputStream::Stderr, output.stderr.clone())
            .await?;
    }
    if let Some(registry) = ctx.process_registry.as_ref() {
        let _ = registry
            .mark_process_exited(&process_id, output.exit_code)
            .await;
    }
    Ok(TaskExecutionResult {
        exit_code: output.exit_code,
        payload: serde_json::json!({
            "command": input.command,
            "args": input.args,
            "cwd": input.cwd.unwrap_or_else(|| ".".to_string()),
            "runner_destination": ctx.runner_destination.as_ref().map(|destination| &destination.id),
            "runner_session": session.state().session_id,
            "success": output.exit_code == Some(0),
        }),
    })
}

struct RemoteCommandStopper {
    session: Arc<dyn roder_api::remote_runner::RemoteRunnerSession>,
    command_id: String,
}

#[async_trait::async_trait]
impl ProcessStopper for RemoteCommandStopper {
    async fn stop(&self, _reason: Option<String>) -> anyhow::Result<()> {
        let cancelled = self.session.cancel_command(&self.command_id).await?;
        if cancelled {
            Ok(())
        } else {
            bail!("remote runner did not cancel command {:?}", self.command_id)
        }
    }
}

async fn stream_pipe(
    pipe: Option<impl tokio::io::AsyncRead + Unpin>,
    stream: TaskOutputStream,
    output: Arc<roder_api::tasks::TaskOutputSink>,
) -> anyhow::Result<()> {
    let Some(pipe) = pipe else {
        return Ok(());
    };
    let mut reader = BufReader::new(pipe);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let bytes = reader.read_until(b'\n', &mut buf).await?;
        if bytes == 0 {
            break;
        }
        output
            .write(stream.clone(), String::from_utf8_lossy(&buf).to_string())
            .await?;
    }
    Ok(())
}

fn resolve_cwd(workspace_root: Option<&str>, cwd: Option<&str>) -> anyhow::Result<PathBuf> {
    let Some(root) = workspace_root else {
        return match cwd {
            Some(cwd) => Ok(PathBuf::from(cwd)),
            None => std::env::current_dir().context("resolve current directory"),
        };
    };
    let root = std::fs::canonicalize(root).with_context(|| format!("canonicalize root {root}"))?;
    let candidate = match cwd {
        Some(cwd) => {
            let path = Path::new(cwd);
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                root.join(path)
            }
        }
        None => root.clone(),
    };
    let candidate = std::fs::canonicalize(&candidate)
        .with_context(|| format!("canonicalize cwd {}", candidate.display()))?;
    if !candidate.starts_with(&root) {
        bail!(
            "process task cwd {} escapes workspace root {}",
            candidate.display(),
            root.display()
        );
    }
    Ok(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_cwd_rejects_paths_outside_workspace() {
        let root = std::env::current_dir().unwrap();
        let outside = root.parent().unwrap_or(&root);
        let err = resolve_cwd(
            Some(root.to_str().unwrap()),
            Some(outside.to_str().unwrap()),
        )
        .unwrap_err();

        assert!(err.to_string().contains("escapes workspace root"));
    }

    #[test]
    fn schema_snapshot_covers_process_task_input() {
        let executor = ProcessTaskExecutor;
        let spec = executor
            .spec()
            .normalized_for_model(roder_api::ToolSchemaPolicy::strict());
        let schema = serde_json::to_string(&spec.input_schema).unwrap();

        assert!(schema.starts_with(r#"{"type":"object","required":["command"],"properties":"#));
        assert!(schema.contains(
            r#""env_overrides":{"type":"object","additionalProperties":{"type":"string"}}"#
        ));
        assert!(schema.contains(r#""additionalProperties":false"#));
    }
}
