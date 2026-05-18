use std::collections::BTreeMap;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use roder_api::tools::{
    ToolCall, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult, ToolSpec,
};
use serde::Deserialize;
use serde_json::json;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::{ChildStdin, Command};
use tokio::sync::Mutex;

use crate::files::{parse, require_nonempty, result};
use crate::workspace::Workspace;

const DEFAULT_YIELD_MS: u64 = 1000;
const DEFAULT_MAX_OUTPUT_TOKENS: usize = 6000;
const MAX_BUFFER_BYTES: usize = 1024 * 1024;

pub(crate) fn register(registry: &mut ToolRegistry, workspace: Workspace) -> anyhow::Result<()> {
    let manager = Arc::new(ExecSessionManager::default());
    registry.register(Arc::new(ExecCommandTool {
        workspace,
        manager: manager.clone(),
    }))?;
    registry.register(Arc::new(WriteStdinTool { manager }))
}

#[derive(Debug, Default)]
struct ExecSessionManager {
    next_id: AtomicU64,
    sessions: Mutex<BTreeMap<u64, Arc<ExecSession>>>,
}

#[derive(Debug)]
struct ExecSession {
    command: String,
    cwd: String,
    started: Instant,
    stdin: Mutex<Option<ChildStdin>>,
    output: Mutex<String>,
    cursor: Mutex<usize>,
    exit: Mutex<Option<ExecExit>>,
    tty: bool,
}

#[derive(Debug, Clone, Copy)]
struct ExecExit {
    exit_code: i32,
    timed_out: bool,
}

#[derive(Debug)]
struct ExecCommandTool {
    workspace: Workspace,
    manager: Arc<ExecSessionManager>,
}

#[derive(Debug)]
struct WriteStdinTool {
    manager: Arc<ExecSessionManager>,
}

#[async_trait::async_trait]
impl ToolExecutor for ExecCommandTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "exec_command".to_string(),
            description: "Runs a command in a PTY-like session, returning output or a session ID for ongoing interaction.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "cmd": {
                        "type": "string",
                        "description": "Shell command to execute."
                    },
                    "workdir": {
                        "type": "string",
                        "description": "Optional working directory to run the command in; defaults to the turn cwd."
                    },
                    "shell": {
                        "type": "string",
                        "description": "Shell binary to launch. Defaults to the user's default shell."
                    },
                    "tty": {
                        "type": "boolean",
                        "description": "Whether to request a TTY-style wrapper for the command."
                    },
                    "yield_time_ms": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "How long to wait for output before yielding."
                    },
                    "max_output_tokens": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Maximum output tokens to return. Excess output is truncated."
                    },
                    "login": {
                        "type": "boolean",
                        "description": "Whether to run the shell with login semantics. Defaults to true."
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Optional wall-clock timeout for the process."
                    }
                },
                "required": ["cmd"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        ctx.require_workspace()?;
        ctx.require_process_runner()?;
        let args = parse::<ExecCommandArgs>(&call)?;
        let command = args.cmd.trim().to_string();
        require_nonempty(&command, "cmd")?;
        let cwd_path = match args.workdir.as_deref() {
            Some(workdir) if !workdir.trim().is_empty() => {
                self.workspace.resolve_existing(workdir)?
            }
            _ => self.workspace.root().to_path_buf(),
        };
        let cwd = self.workspace.display(&cwd_path);
        let session_id = self.manager.next_session_id();
        let shell = args
            .shell
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(default_shell);
        let login = args.login.unwrap_or(true);
        let tty = args.tty.unwrap_or(false);
        let mut child = build_command(&shell, &command, login, tty)
            .current_dir(&cwd_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let session = Arc::new(ExecSession {
            command,
            cwd,
            started: Instant::now(),
            stdin: Mutex::new(stdin),
            output: Mutex::new(String::new()),
            cursor: Mutex::new(0),
            exit: Mutex::new(None),
            tty,
        });

        if let Some(stdout) = stdout {
            spawn_output_reader(stdout, session.clone());
        }
        if let Some(stderr) = stderr {
            spawn_output_reader(stderr, session.clone());
        }
        spawn_waiter(child, session.clone(), args.timeout_ms);
        self.manager
            .sessions
            .lock()
            .await
            .insert(session_id, session.clone());

        sleep_for(args.yield_time_ms).await;
        let completed = session.exit.lock().await.is_some();
        let snapshot = session
            .snapshot(
                Some(session_id),
                args.max_output_tokens,
                SnapshotMode::SinceLastRead,
            )
            .await;
        if completed {
            self.manager.sessions.lock().await.remove(&session_id);
        }
        Ok(result(
            call,
            snapshot.text,
            snapshot.data,
            snapshot.is_error,
        ))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for WriteStdinTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "write_stdin".to_string(),
            description:
                "Writes characters to an existing unified exec session and returns recent output."
                    .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "integer",
                        "description": "Identifier of the running unified exec session."
                    },
                    "chars": {
                        "type": "string",
                        "description": "Bytes to write to stdin. Omit or pass an empty string to poll."
                    },
                    "yield_time_ms": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "How long to wait for output before yielding."
                    },
                    "max_output_tokens": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Maximum output tokens to return. Excess output is truncated."
                    }
                },
                "required": ["session_id"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        ctx.require_process_runner()?;
        let args = parse::<WriteStdinArgs>(&call)?;
        let Some(session) = self
            .manager
            .sessions
            .lock()
            .await
            .get(&args.session_id)
            .cloned()
        else {
            return Ok(result(
                call,
                format!("exec session {} not found", args.session_id),
                json!({
                    "session_id": args.session_id,
                    "status": "not_found"
                }),
                true,
            ));
        };

        if let Some(chars) = args.chars.as_deref()
            && !chars.is_empty()
        {
            let mut stdin = session.stdin.lock().await;
            if let Some(stdin) = stdin.as_mut() {
                stdin.write_all(chars.as_bytes()).await?;
                stdin.flush().await?;
            }
        }

        sleep_for(args.yield_time_ms).await;
        let completed = session.exit.lock().await.is_some();
        let snapshot = session
            .snapshot(
                Some(args.session_id),
                args.max_output_tokens,
                SnapshotMode::SinceLastRead,
            )
            .await;
        if completed {
            self.manager.sessions.lock().await.remove(&args.session_id);
        }
        Ok(result(
            call,
            snapshot.text,
            snapshot.data,
            snapshot.is_error,
        ))
    }
}

#[derive(Deserialize)]
struct ExecCommandArgs {
    cmd: String,
    workdir: Option<String>,
    shell: Option<String>,
    tty: Option<bool>,
    yield_time_ms: Option<u64>,
    max_output_tokens: Option<usize>,
    login: Option<bool>,
    timeout_ms: Option<u64>,
}

#[derive(Deserialize)]
struct WriteStdinArgs {
    session_id: u64,
    chars: Option<String>,
    yield_time_ms: Option<u64>,
    max_output_tokens: Option<usize>,
}

#[derive(Debug, Clone, Copy)]
enum SnapshotMode {
    SinceLastRead,
}

struct ExecSnapshot {
    text: String,
    data: serde_json::Value,
    is_error: bool,
}

impl ExecSessionManager {
    fn next_session_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed) + 1
    }
}

impl ExecSession {
    async fn snapshot(
        &self,
        session_id: Option<u64>,
        max_output_tokens: Option<usize>,
        mode: SnapshotMode,
    ) -> ExecSnapshot {
        let exit = *self.exit.lock().await;
        let status = status_for(exit);
        let is_error = exit.is_some_and(|exit| exit.exit_code != 0 || exit.timed_out);
        let full_output = self.output.lock().await.clone();
        let output = match mode {
            SnapshotMode::SinceLastRead => {
                let mut cursor = self.cursor.lock().await;
                let start = (*cursor).min(full_output.len());
                let chunk = full_output[start..].to_string();
                *cursor = full_output.len();
                chunk
            }
        };
        let output = if output.is_empty() && exit.is_some() {
            "(no output)".to_string()
        } else {
            truncate_output(&output, max_output_tokens)
        };
        let elapsed_ms = self.started.elapsed().as_millis() as u64;
        let exit_code = exit.map(|exit| exit.exit_code);
        let timed_out = exit.is_some_and(|exit| exit.timed_out);
        let text = format_exec_output(exit_code, status, elapsed_ms, session_id, &output);
        ExecSnapshot {
            text,
            data: json!({
                "command": self.command,
                "cwd": self.cwd,
                "session_id": session_id,
                "aggregated_output": output,
                "exit_code": exit_code,
                "duration_ms": elapsed_ms,
                "status": status,
                "timed_out": timed_out,
                "tty": self.tty,
            }),
            is_error,
        }
    }
}

fn build_command(shell: &str, command: &str, login: bool, tty: bool) -> Command {
    if tty && cfg!(target_os = "macos") {
        let mut cmd = Command::new("script");
        cmd.arg("-q")
            .arg("/dev/null")
            .arg(shell)
            .arg(shell_arg(login))
            .arg(command);
        return cmd;
    }

    let mut cmd = Command::new(shell);
    cmd.arg(shell_arg(login)).arg(command);
    cmd
}

fn shell_arg(login: bool) -> &'static str {
    if login { "-lc" } else { "-c" }
}

fn default_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
}

fn spawn_output_reader<R>(mut reader: R, session: Arc<ExecSession>)
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut buf = [0_u8; 8192];
        loop {
            match reader.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let mut output = session.output.lock().await;
                    output.push_str(&String::from_utf8_lossy(&buf[..n]));
                    if output.len() > MAX_BUFFER_BYTES {
                        let keep_from = output.len() - MAX_BUFFER_BYTES;
                        output.replace_range(..keep_from, "");
                        let mut cursor = session.cursor.lock().await;
                        *cursor = cursor.saturating_sub(keep_from);
                    }
                }
            }
        }
    });
}

fn spawn_waiter(
    mut child: tokio::process::Child,
    session: Arc<ExecSession>,
    timeout_ms: Option<u64>,
) {
    tokio::spawn(async move {
        let exit = if let Some(timeout_ms) = timeout_ms {
            tokio::select! {
                status = child.wait() => exit_from_status(status, false),
                _ = tokio::time::sleep(Duration::from_millis(timeout_ms)) => {
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                    ExecExit { exit_code: -1, timed_out: true }
                }
            }
        } else {
            exit_from_status(child.wait().await, false)
        };
        *session.exit.lock().await = Some(exit);
    });
}

fn exit_from_status(
    status: std::io::Result<std::process::ExitStatus>,
    timed_out: bool,
) -> ExecExit {
    let exit_code = status
        .ok()
        .and_then(|status| status.code())
        .unwrap_or(if timed_out { -1 } else { 1 });
    ExecExit {
        exit_code,
        timed_out,
    }
}

fn status_for(exit: Option<ExecExit>) -> &'static str {
    match exit {
        None => "running",
        Some(exit) if exit.timed_out => "timed_out",
        Some(exit) if exit.exit_code == 0 => "completed",
        Some(_) => "failed",
    }
}

async fn sleep_for(yield_time_ms: Option<u64>) {
    let millis = yield_time_ms.unwrap_or(DEFAULT_YIELD_MS);
    if millis > 0 {
        tokio::time::sleep(Duration::from_millis(millis)).await;
    }
}

fn truncate_output(output: &str, max_output_tokens: Option<usize>) -> String {
    let max_chars = max_output_tokens
        .unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS)
        .saturating_mul(4);
    if output.len() <= max_chars {
        return output.trim_end().to_string();
    }
    let omitted = output.len() - max_chars;
    format!(
        "[{} bytes omitted]\n{}",
        omitted,
        output[output.len() - max_chars..].trim_end()
    )
}

fn format_exec_output(
    exit_code: Option<i32>,
    status: &str,
    duration_ms: u64,
    session_id: Option<u64>,
    output: &str,
) -> String {
    let exit = exit_code
        .map(|code| code.to_string())
        .unwrap_or_else(|| "still running".to_string());
    let mut text = format!(
        "Exit code: {exit}\nStatus: {status}\nWall time: {:.3} seconds",
        duration_ms as f64 / 1000.0
    );
    if status == "running"
        && let Some(session_id) = session_id
    {
        text.push_str(&format!("\nSession id: {session_id}"));
    }
    text.push_str(&format!("\nOutput:\n{output}"));
    text
}

#[cfg(test)]
mod tests {
    use roder_api::events::{ThreadId, TurnId};
    use roder_api::policy_mode::PolicyMode;
    use roder_api::tools::{LocalProcessRunnerHandle, LocalWorkspaceHandle};

    use super::*;

    #[tokio::test]
    async fn exec_command_returns_completed_output_without_session_polling() {
        let root = temp_workspace("roder-exec");
        std::fs::create_dir_all(&root).unwrap();
        let manager = Arc::new(ExecSessionManager::default());
        let tool = ExecCommandTool {
            workspace: Workspace::new(root.clone()).unwrap(),
            manager,
        };

        let result = tool
            .execute(
                context(),
                call(
                    "exec_command",
                    json!({ "cmd": "printf hi", "yield_time_ms": 50 }),
                ),
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert_eq!(result.data["status"], "completed");
        assert_eq!(result.data["aggregated_output"], "hi");

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn write_stdin_polls_running_session() {
        let root = temp_workspace("roder-exec-stdin");
        std::fs::create_dir_all(&root).unwrap();
        let manager = Arc::new(ExecSessionManager::default());
        let exec = ExecCommandTool {
            workspace: Workspace::new(root.clone()).unwrap(),
            manager: manager.clone(),
        };
        let stdin = WriteStdinTool { manager };

        let started = exec
            .execute(
                context(),
                call(
                    "exec_command",
                    json!({
                        "cmd": "read line; printf got:$line",
                        "yield_time_ms": 10
                    }),
                ),
            )
            .await
            .unwrap();
        assert_eq!(started.data["status"], "running");
        let session_id = started.data["session_id"].as_u64().unwrap();

        let polled = stdin
            .execute(
                context(),
                call(
                    "write_stdin",
                    json!({
                        "session_id": session_id,
                        "chars": "hello\n",
                        "yield_time_ms": 100
                    }),
                ),
            )
            .await
            .unwrap();

        assert!(!polled.is_error);
        assert_eq!(polled.data["status"], "completed");
        assert_eq!(polled.data["aggregated_output"], "got:hello");

        let _ = std::fs::remove_dir_all(root);
    }

    fn call(name: &str, arguments: serde_json::Value) -> ToolCall {
        ToolCall {
            id: format!("call-{name}"),
            name: name.to_string(),
            arguments,
            raw_arguments: "{}".to_string(),
            thread_id: "thread-exec".to_string(),
            turn_id: "turn-exec".to_string(),
        }
    }

    fn context() -> ToolExecutionContext {
        ToolExecutionContext::new(
            ThreadId::from("thread-exec"),
            TurnId::from("turn-exec"),
            PolicyMode::Default,
        )
        .with_workspace_handle(Arc::new(LocalWorkspaceHandle::new(".")))
        .with_process_runner(Arc::new(LocalProcessRunnerHandle))
    }

    fn temp_workspace(prefix: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}"))
    }
}
