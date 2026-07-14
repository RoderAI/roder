use std::collections::BTreeMap;
use std::path::PathBuf;
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
use tokio::sync::{Mutex, Notify};

use crate::backend::WorkspaceBackendHandle;
use crate::command_shell::{command_args_for_shell, shell_for_context};
use crate::exec_output::{format_exec_output, trim_output_buffer_to_max_bytes, truncate_output};
use crate::files::{parse, require_nonempty, result};
use crate::workspace::Workspace;

const DEFAULT_YIELD_MS: u64 = 1000;
const DEFAULT_MAX_OUTPUT_TOKENS: usize = 6000;
const MAX_BUFFER_BYTES: usize = 1024 * 1024;
const DEADLINE_TIMEOUT_RESERVE_MS: u64 = 30_000;
const MIN_DEADLINE_TIMEOUT_MS: u64 = 1_000;

pub(crate) fn register(
    registry: &mut ToolRegistry,
    workspace: Workspace,
    command_shell: String,
    backend: Option<WorkspaceBackendHandle>,
) -> anyhow::Result<()> {
    let manager = Arc::new(ExecSessionManager::default());
    registry.register(Arc::new(ExecCommandTool {
        workspace: workspace.clone(),
        manager: manager.clone(),
        command_shell: command_shell.clone(),
        backend: backend.clone(),
    }))?;
    registry.register(Arc::new(WriteStdinTool {
        manager: manager.clone(),
        backend: backend.clone(),
    }))?;
    registry.register(Arc::new(UnifiedExecTool {
        workspace,
        manager,
        command_shell,
        backend,
    }))
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
    shell: String,
    started: Instant,
    effective_timeout_ms: Option<u64>,
    stdin: Mutex<Option<ChildStdin>>,
    output: Mutex<String>,
    cursor: Mutex<usize>,
    exit: Mutex<Option<ExecExit>>,
    exit_notify: Notify,
    tty: bool,
}

#[derive(Debug, Clone, Copy)]
struct ExecExit {
    exit_code: i32,
    timed_out: bool,
}

struct ExecCommandTool {
    workspace: Workspace,
    manager: Arc<ExecSessionManager>,
    command_shell: String,
    /// Exec commands can change files behind the search index's back; the
    /// backend is notified after each interaction so the next grep rebuilds.
    backend: Option<WorkspaceBackendHandle>,
}

struct WriteStdinTool {
    manager: Arc<ExecSessionManager>,
    backend: Option<WorkspaceBackendHandle>,
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
                        "description": "Shell binary to launch. Defaults to the configured command shell."
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
        // No interactive PTY/stdin transport exists over a remote runner session.
        let workspace =
            Workspace::local_from_context_or_fallback(&ctx, &self.workspace, "exec_command")?;
        let args = parse::<ExecCommandArgs>(&call)?;
        let command = args.cmd.trim().to_string();
        require_nonempty(&command, "cmd")?;
        let cwd_path = match args.workdir.as_deref() {
            Some(workdir) => workspace.resolve_existing_workdir(workdir)?,
            _ => workspace.root().to_path_buf(),
        };
        let shell = args
            .shell
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| shell_for_context(&ctx, &self.command_shell));
        let login = args.login.unwrap_or(true);
        let tty = args.tty.unwrap_or(false);
        let timeout_ms = effective_timeout_ms(args.timeout_ms, ctx.deadline_remaining_seconds);
        let cwd = workspace.display(&cwd_path);
        let (session_id, session) = spawn_exec_session(
            &self.manager,
            SpawnOptions {
                command,
                cwd_path,
                cwd,
                shell,
                login,
                tty,
                timeout_ms,
            },
        )
        .await?;

        let snapshot = settle_and_snapshot(
            &self.manager,
            session_id,
            &session,
            args.yield_time_ms,
            args.max_output_tokens,
        )
        .await;
        if let Some(backend) = self.backend.as_ref() {
            backend.note_external_change();
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

        write_session_stdin(&session, args.chars.as_deref()).await?;

        let snapshot = settle_and_snapshot(
            &self.manager,
            args.session_id,
            &session,
            args.yield_time_ms,
            args.max_output_tokens,
        )
        .await;
        if let Some(backend) = self.backend.as_ref() {
            backend.note_external_change();
        }
        Ok(result(
            call,
            snapshot.text,
            snapshot.data,
            snapshot.is_error,
        ))
    }
}

/// Codex-shaped single-tool wrapper over [`ExecSessionManager`]: omitting
/// `session_id` starts a new session, passing one writes to an existing
/// session's stdin. Reuses the same session bookkeeping as `exec_command` and
/// `write_stdin` so all three tools observe and drive the same sessions.
struct UnifiedExecTool {
    workspace: Workspace,
    manager: Arc<ExecSessionManager>,
    command_shell: String,
    backend: Option<WorkspaceBackendHandle>,
}

#[async_trait::async_trait]
impl ToolExecutor for UnifiedExecTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "unified_exec".to_string(),
            description: "Runs shell commands in persistent sessions. Omit session_id to start a \
                new session running `input` as a shell command; pass the session_id returned by an \
                earlier call to send more input (e.g. a reply, or Ctrl-C as \"\\u0003\") to that \
                still-running command. Returns output collected up to timeout_ms, plus the session \
                ID if the command is still running."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "input": {
                        "type": "string",
                        "description": "Shell command to start a new session, or characters to write to stdin of an existing session."
                    },
                    "session_id": {
                        "type": ["string", "integer"],
                        "description": "Identifier of a running session to write to, as returned by an earlier call. Omit to start a new session."
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "How long to wait for output before returning. Defaults to 1000ms. The session keeps running past the timeout if it has not exited."
                    }
                },
                "required": ["input"],
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
        let args = parse::<UnifiedExecArgs>(&call)?;
        require_nonempty(&args.input, "input")?;
        let session_id = match parse_session_id(args.session_id.as_ref()) {
            Ok(session_id) => session_id,
            Err(message) => {
                return Ok(result(
                    call,
                    message,
                    json!({ "status": "invalid_session_id" }),
                    true,
                ));
            }
        };

        let (session_id, session) = match session_id {
            Some(session_id) => {
                let Some(session) = self.manager.sessions.lock().await.get(&session_id).cloned()
                else {
                    return Ok(result(
                        call,
                        format!("unified exec session {session_id} not found"),
                        json!({
                            "session_id": session_id.to_string(),
                            "status": "not_found"
                        }),
                        true,
                    ));
                };
                write_session_stdin(&session, Some(args.input.as_str())).await?;
                (session_id, session)
            }
            None => {
                ctx.require_workspace()?;
                let workspace = Workspace::local_from_context_or_fallback(
                    &ctx,
                    &self.workspace,
                    "unified_exec",
                )?;
                let command = args.input.trim().to_string();
                require_nonempty(&command, "input")?;
                let cwd_path = workspace.root().to_path_buf();
                let cwd = workspace.display(&cwd_path);
                let shell = shell_for_context(&ctx, &self.command_shell);
                // unified_exec's timeout_ms bounds how long we wait for output,
                // not the process lifetime (Codex leaves the session running past
                // it); only the turn deadline can force-kill the process here.
                let timeout_ms = effective_timeout_ms(None, ctx.deadline_remaining_seconds);
                spawn_exec_session(
                    &self.manager,
                    SpawnOptions {
                        command,
                        cwd_path,
                        cwd,
                        shell,
                        login: true,
                        tty: false,
                        timeout_ms,
                    },
                )
                .await?
            }
        };

        let snapshot =
            settle_and_snapshot(&self.manager, session_id, &session, args.timeout_ms, None).await;
        if let Some(backend) = self.backend.as_ref() {
            backend.note_external_change();
        }
        let mut data = snapshot.data;
        // Codex's unified_exec returns session_id as a string on the wire, and
        // gpt-5.5 is RL-trained to echo that shape back in the next call.
        if let Some(session_id) = data.get_mut("session_id")
            && !session_id.is_null()
        {
            *session_id = json!(session_id.to_string().trim_matches('"'));
        }
        Ok(result(call, snapshot.text, data, snapshot.is_error))
    }
}

/// Accepts `session_id` as either a JSON string or integer, matching Codex's
/// wire shape (string) while staying tolerant of a plain integer. Returns a
/// clear error message for anything that isn't a valid non-negative integer.
fn parse_session_id(value: Option<&serde_json::Value>) -> Result<Option<u64>, String> {
    match value {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::Number(number)) => number
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("session_id must be a non-negative integer, got {number}")),
        Some(serde_json::Value::String(text)) => {
            let trimmed = text.trim();
            trimmed
                .parse::<u64>()
                .map(Some)
                .map_err(|_| format!("session_id must be numeric, got {text:?}"))
        }
        Some(other) => Err(format!(
            "session_id must be a string or integer, got {other}"
        )),
    }
}

/// Options for spawning a new [`ExecSession`], shared by `exec_command` and
/// `unified_exec`.
struct SpawnOptions {
    command: String,
    cwd_path: PathBuf,
    cwd: String,
    shell: String,
    login: bool,
    tty: bool,
    timeout_ms: Option<u64>,
}

/// Spawns a command as a new tracked session and registers it with `manager`.
async fn spawn_exec_session(
    manager: &ExecSessionManager,
    opts: SpawnOptions,
) -> anyhow::Result<(u64, Arc<ExecSession>)> {
    let session_id = manager.next_session_id();
    let mut child = build_command(&opts.shell, &opts.command, opts.login, opts.tty)
        .current_dir(&opts.cwd_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let session = Arc::new(ExecSession {
        command: opts.command,
        cwd: opts.cwd,
        shell: opts.shell,
        started: Instant::now(),
        effective_timeout_ms: opts.timeout_ms,
        stdin: Mutex::new(stdin),
        output: Mutex::new(String::new()),
        cursor: Mutex::new(0),
        exit: Mutex::new(None),
        exit_notify: Notify::new(),
        tty: opts.tty,
    });

    if let Some(stdout) = stdout {
        spawn_output_reader(stdout, session.clone());
    }
    if let Some(stderr) = stderr {
        spawn_output_reader(stderr, session.clone());
    }
    spawn_waiter(child, session.clone(), opts.timeout_ms);
    manager
        .sessions
        .lock()
        .await
        .insert(session_id, session.clone());

    Ok((session_id, session))
}

/// Writes `chars` to a session's stdin, if any non-empty input was given.
/// Shared by `write_stdin` and `unified_exec`.
async fn write_session_stdin(session: &ExecSession, chars: Option<&str>) -> anyhow::Result<()> {
    if let Some(chars) = chars
        && !chars.is_empty()
    {
        let mut stdin = session.stdin.lock().await;
        if let Some(stdin) = stdin.as_mut() {
            stdin.write_all(chars.as_bytes()).await?;
            stdin.flush().await?;
        }
    }
    Ok(())
}

/// Waits for output, takes a snapshot since the last read, and evicts the
/// session from `manager` if it has completed. Shared by all three exec
/// tools.
async fn settle_and_snapshot(
    manager: &ExecSessionManager,
    session_id: u64,
    session: &ExecSession,
    yield_time_ms: Option<u64>,
    max_output_tokens: Option<usize>,
) -> ExecSnapshot {
    sleep_for(yield_time_ms).await;
    settle_completed_exit(session).await;
    let completed = session.exit.lock().await.is_some();
    let snapshot = session
        .snapshot(
            Some(session_id),
            max_output_tokens,
            SnapshotMode::SinceLastRead,
        )
        .await;
    if completed {
        manager.sessions.lock().await.remove(&session_id);
    }
    snapshot
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

#[derive(Deserialize)]
struct UnifiedExecArgs {
    input: String,
    session_id: Option<serde_json::Value>,
    timeout_ms: Option<u64>,
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
            truncate_output(&output, max_output_tokens, DEFAULT_MAX_OUTPUT_TOKENS)
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
                "shell": self.shell,
                "session_id": session_id,
                "aggregated_output": output,
                "exit_code": exit_code,
                "duration_ms": elapsed_ms,
                "effective_timeout_ms": self.effective_timeout_ms,
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
        cmd.arg("-q").arg("/dev/null").arg(shell);
        cmd.args(command_args_for_shell(shell, command, login));
        return cmd;
    }

    let mut cmd = Command::new(shell);
    cmd.args(command_args_for_shell(shell, command, login));
    cmd
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
                    let mut cursor = session.cursor.lock().await;
                    trim_output_buffer_to_max_bytes(&mut output, &mut cursor, MAX_BUFFER_BYTES);
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
        session.exit_notify.notify_waiters();
    });
}

fn effective_timeout_ms(
    requested_timeout_ms: Option<u64>,
    deadline_remaining_seconds: Option<u64>,
) -> Option<u64> {
    let deadline_timeout = deadline_remaining_seconds.map(|seconds| {
        seconds
            .saturating_mul(1000)
            .saturating_sub(DEADLINE_TIMEOUT_RESERVE_MS)
            .max(MIN_DEADLINE_TIMEOUT_MS)
    });
    match (requested_timeout_ms, deadline_timeout) {
        (Some(requested), Some(deadline)) => Some(requested.min(deadline)),
        (Some(requested), None) => Some(requested),
        (None, Some(deadline)) => Some(deadline),
        (None, None) => None,
    }
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

async fn settle_completed_exit(session: &ExecSession) {
    if session.exit.lock().await.is_some() {
        tokio::task::yield_now().await;
        return;
    }
    let _ = tokio::time::timeout(Duration::from_millis(250), session.exit_notify.notified()).await;
    tokio::task::yield_now().await;
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
            command_shell: test_command_shell(),
            backend: None,
        };
        let cmd = if cfg!(windows) {
            "[Console]::Out.Write('hi')"
        } else {
            "printf hi"
        };

        let result = tool
            .execute(
                context(&root),
                call(
                    "exec_command",
                    json!({ "cmd": cmd, "yield_time_ms": 1000, "login": false }),
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
    async fn exec_command_does_not_snapshot_workspace_changes_as_hunks() {
        let root = temp_workspace("roder-exec-hunks");
        std::fs::create_dir_all(root.join("src/routes")).unwrap();
        std::fs::write(root.join("src/routes/index.tsx"), "old title\n").unwrap();
        let manager = Arc::new(ExecSessionManager::default());
        let tool = ExecCommandTool {
            workspace: Workspace::new(root.clone()).unwrap(),
            manager,
            command_shell: roder_api::command_shell::default_command_shell(),
            backend: None,
        };

        let result = tool
            .execute(
                context(&root),
                call(
                    "exec_command",
                    json!({
                        "cmd": "node -e \"require('fs').writeFileSync('src/routes/index.tsx', 'new title\\n')\"",
                        "yield_time_ms": 100,
                        "login": false
                    }),
                ),
            )
            .await
            .unwrap();

        assert!(result.data.get("hunks").is_none());

        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn write_stdin_polls_running_session() {
        let root = temp_workspace("roder-exec-stdin");
        std::fs::create_dir_all(&root).unwrap();
        let manager = Arc::new(ExecSessionManager::default());
        let exec = ExecCommandTool {
            workspace: Workspace::new(root.clone()).unwrap(),
            manager: manager.clone(),
            command_shell: test_command_shell(),
            backend: None,
        };
        let stdin = WriteStdinTool {
            manager,
            backend: None,
        };
        let cmd = if cfg!(windows) {
            "[Console]::Out.Write('got:' + [Console]::In.ReadLine())"
        } else {
            "read line; printf got:$line"
        };

        let started = exec
            .execute(
                context(&root),
                call(
                    "exec_command",
                    json!({
                        "cmd": cmd,
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
                context(&root),
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

    fn test_command_shell() -> String {
        if cfg!(windows) {
            "powershell".to_string()
        } else {
            roder_api::command_shell::default_command_shell()
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn exec_command_uses_context_command_shell_by_default() {
        let root = temp_workspace("roder-exec-context-shell");
        std::fs::create_dir_all(&root).unwrap();
        let shell = root.join("record-shell.sh");
        std::fs::write(
            &shell,
            "#!/bin/sh\nprintf '%s\\n' \"$0\" > used-shell.txt\nexec /bin/sh \"$@\"\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&shell).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&shell, permissions).unwrap();
        let manager = Arc::new(ExecSessionManager::default());
        let tool = ExecCommandTool {
            workspace: Workspace::new(root.clone()).unwrap(),
            manager,
            command_shell: "bash".to_string(),
            backend: None,
        };

        let result = tool
            .execute(
                context(&root).with_command_shell(shell.display().to_string()),
                call(
                    "exec_command",
                    json!({ "cmd": "printf hi", "yield_time_ms": 1500 }),
                ),
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert_eq!(result.data["status"], "completed");
        assert_eq!(result.data["shell"], shell.display().to_string());
        assert_eq!(
            std::fs::read_to_string(root.join("used-shell.txt"))
                .unwrap()
                .trim(),
            shell.display().to_string()
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn effective_timeout_reserves_deadline_finalization_window() {
        assert_eq!(effective_timeout_ms(None, None), None);
        assert_eq!(effective_timeout_ms(Some(5_000), None), Some(5_000));
        assert_eq!(effective_timeout_ms(None, Some(90)), Some(60_000));
        assert_eq!(effective_timeout_ms(Some(120_000), Some(90)), Some(60_000));
        assert_eq!(effective_timeout_ms(Some(5_000), Some(90)), Some(5_000));
        assert_eq!(effective_timeout_ms(None, Some(5)), Some(1_000));
    }

    #[tokio::test]
    async fn exec_command_clamps_missing_timeout_to_deadline_remaining() {
        let root = temp_workspace("roder-exec-deadline");
        std::fs::create_dir_all(&root).unwrap();
        let manager = Arc::new(ExecSessionManager::default());
        let tool = ExecCommandTool {
            workspace: Workspace::new(root.clone()).unwrap(),
            manager,
            command_shell: roder_api::command_shell::default_command_shell(),
            backend: None,
        };

        let result = tool
            .execute(
                context(&root).with_deadline_remaining_seconds(1),
                call(
                    "exec_command",
                    json!({ "cmd": sleep_command(2), "yield_time_ms": 1200, "login": false }),
                ),
            )
            .await
            .unwrap();

        assert!(result.is_error);
        assert_eq!(result.data["status"], "timed_out");
        assert_eq!(result.data["timed_out"], true);
        assert_eq!(result.data["effective_timeout_ms"], 1000);

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn exec_command_fails_clearly_on_a_remote_runner_workspace() {
        let root = temp_workspace("roder-exec-remote");
        std::fs::create_dir_all(&root).unwrap();
        let tool = ExecCommandTool {
            workspace: Workspace::new(root.clone()).unwrap(),
            manager: Arc::new(ExecSessionManager::default()),
            command_shell: roder_api::command_shell::default_command_shell(),
            backend: None,
        };
        let state = Arc::new(crate::remote_test_support::RecordingRunnerState::default());
        let ctx = context(&root).with_remote_workspace(Arc::new(
            roder_api::remote_runner::RemoteWorkspace {
                session: Arc::new(crate::remote_test_support::RecordingRunnerSession {
                    state: state.clone(),
                }),
                root: "/sandbox/workspace".into(),
                read_roots: Vec::new(),
            },
        ));

        let error = tool
            .execute(ctx, call("exec_command", json!({ "cmd": "echo hi" })))
            .await
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("exec_command is not supported on a remote runner workspace"),
            "unexpected error: {error}"
        );
        // Nothing may run locally or remotely.
        assert!(state.commands.lock().unwrap().is_empty());

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

    fn context(workspace: &std::path::Path) -> ToolExecutionContext {
        ToolExecutionContext::new(
            ThreadId::from("thread-exec"),
            TurnId::from("turn-exec"),
            PolicyMode::Default,
        )
        .with_workspace_handle(Arc::new(LocalWorkspaceHandle::new(workspace)))
        .with_process_runner(Arc::new(LocalProcessRunnerHandle))
    }

    fn temp_workspace(prefix: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}"))
    }

    fn sleep_command(seconds: u64) -> String {
        if cfg!(windows) {
            format!("Start-Sleep -Seconds {seconds}")
        } else {
            format!("sleep {seconds}")
        }
    }

    #[tokio::test]
    async fn unified_exec_starts_a_new_session_without_session_id() {
        let root = temp_workspace("roder-unified-exec-new");
        std::fs::create_dir_all(&root).unwrap();
        let manager = Arc::new(ExecSessionManager::default());
        let tool = UnifiedExecTool {
            workspace: Workspace::new(root.clone()).unwrap(),
            manager,
            command_shell: test_command_shell(),
            backend: None,
        };
        let cmd = if cfg!(windows) {
            "[Console]::Out.Write('hi')"
        } else {
            "printf hi"
        };

        let result = tool
            .execute(
                context(&root),
                call("unified_exec", json!({ "input": cmd, "timeout_ms": 1000 })),
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert_eq!(result.data["status"], "completed");
        assert_eq!(result.data["aggregated_output"], "hi");

        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unified_exec_reuses_a_session_via_string_session_id() {
        let root = temp_workspace("roder-unified-exec-reuse");
        std::fs::create_dir_all(&root).unwrap();
        let manager = Arc::new(ExecSessionManager::default());
        let tool = UnifiedExecTool {
            workspace: Workspace::new(root.clone()).unwrap(),
            manager,
            command_shell: test_command_shell(),
            backend: None,
        };

        let started = tool
            .execute(
                context(&root),
                call("unified_exec", json!({ "input": "cat", "timeout_ms": 200 })),
            )
            .await
            .unwrap();
        assert_eq!(started.data["status"], "running");
        // Codex's wire shape returns session_id as a string; the model is
        // expected to echo that same string back on the next call.
        let session_id = started.data["session_id"].as_str().unwrap().to_string();

        let echoed = tool
            .execute(
                context(&root),
                call(
                    "unified_exec",
                    json!({
                        "input": "hello\n",
                        "session_id": session_id,
                        "timeout_ms": 200
                    }),
                ),
            )
            .await
            .unwrap();

        assert!(!echoed.is_error);
        assert_eq!(echoed.data["session_id"], json!(session_id));
        assert_eq!(echoed.data["aggregated_output"], "hello");

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn unified_exec_returns_running_session_id_when_timeout_elapses() {
        let root = temp_workspace("roder-unified-exec-timeout");
        std::fs::create_dir_all(&root).unwrap();
        let manager = Arc::new(ExecSessionManager::default());
        let tool = UnifiedExecTool {
            workspace: Workspace::new(root.clone()).unwrap(),
            manager,
            command_shell: roder_api::command_shell::default_command_shell(),
            backend: None,
        };

        let result = tool
            .execute(
                context(&root),
                call(
                    "unified_exec",
                    json!({ "input": sleep_command(5), "timeout_ms": 200 }),
                ),
            )
            .await
            .unwrap();

        // The process is still running past timeout_ms: unified_exec only
        // bounds how long it waits for output, it does not kill the session.
        assert_eq!(result.data["status"], "running");
        assert!(result.data["session_id"].as_str().is_some());

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn unified_exec_reports_unknown_session_id() {
        let root = temp_workspace("roder-unified-exec-missing");
        std::fs::create_dir_all(&root).unwrap();
        let manager = Arc::new(ExecSessionManager::default());
        let tool = UnifiedExecTool {
            workspace: Workspace::new(root.clone()).unwrap(),
            manager,
            command_shell: test_command_shell(),
            backend: None,
        };

        let result = tool
            .execute(
                context(&root),
                call(
                    "unified_exec",
                    // A string session_id (Codex's wire shape) must resolve the
                    // same as an integer one.
                    json!({ "input": "echo hi", "session_id": "9999" }),
                ),
            )
            .await
            .unwrap();

        assert!(result.is_error);
        assert_eq!(result.data["status"], "not_found");

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn unified_exec_rejects_non_numeric_session_id() {
        let root = temp_workspace("roder-unified-exec-bad-id");
        std::fs::create_dir_all(&root).unwrap();
        let manager = Arc::new(ExecSessionManager::default());
        let tool = UnifiedExecTool {
            workspace: Workspace::new(root.clone()).unwrap(),
            manager,
            command_shell: test_command_shell(),
            backend: None,
        };

        let result = tool
            .execute(
                context(&root),
                call(
                    "unified_exec",
                    json!({ "input": "echo hi", "session_id": "not-a-number" }),
                ),
            )
            .await
            .unwrap();

        assert!(result.is_error);
        assert_eq!(result.data["status"], "invalid_session_id");

        let _ = std::fs::remove_dir_all(root);
    }
}
