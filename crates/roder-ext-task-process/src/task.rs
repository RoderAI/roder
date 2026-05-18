use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use anyhow::{Context, bail};
use roder_api::remote_runner::RunnerCommandRequest;
use roder_api::tasks::{
    TaskExecutionContext, TaskExecutionResult, TaskExecutor, TaskOutputStream, TaskSpec,
};
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

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
                "properties": {
                    "command": { "type": "string" },
                    "args": { "type": "array", "items": { "type": "string" } },
                    "cwd": { "type": "string" },
                    "env_overrides": {
                        "type": "object",
                        "additionalProperties": { "type": "string" }
                    }
                },
                "required": ["command"]
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
        let mut command = Command::new(&input.command);
        command
            .args(&input.args)
            .current_dir(&cwd)
            .envs(&input.env_overrides)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = command
            .spawn()
            .with_context(|| format!("spawn process task {:?}", input.command))?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let output = Arc::new(ctx.output);

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
        let status = child.wait().await.context("wait for process task")?;
        stdout_task.await.context("join stdout reader")??;
        stderr_task.await.context("join stderr reader")??;

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

async fn execute_remote_process_task(
    ctx: TaskExecutionContext,
    input: ProcessTaskInput,
) -> anyhow::Result<TaskExecutionResult> {
    let Some(session) = ctx.runner_session else {
        bail!("remote process task requires runner session");
    };
    let output = session
        .run_command(RunnerCommandRequest {
            command_id: ctx.task_id.clone(),
            program: input.command.clone(),
            args: input.args.clone(),
            cwd: input.cwd.as_deref().map(PathBuf::from),
            env: input.env_overrides.clone().into_iter().collect(),
        })
        .await?;
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
}
