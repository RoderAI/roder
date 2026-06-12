use std::sync::Arc;
use std::time::Instant;

use roder_api::remote_runner::RunnerCommandRequest;
use roder_api::tools::{
    ToolCall, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult, ToolSpec,
};
use serde::Deserialize;
use serde_json::json;
use tokio::process::Command;

use crate::backend::WorkspaceBackendHandle;
use crate::command_shell::{command_args_for_shell, shell_for_context};
use crate::files::{parse, require_nonempty, result};
use crate::workspace::Workspace;

const DEFAULT_TIMEOUT_SECONDS: u64 = 120;
const MAX_TIMEOUT_SECONDS: u64 = 600;
const DEADLINE_TIMEOUT_RESERVE_SECONDS: u64 = 30;
const MIN_DEADLINE_TIMEOUT_SECONDS: u64 = 1;

pub(crate) fn register(
    registry: &mut ToolRegistry,
    workspace: Workspace,
    command_shell: String,
    backend: Option<WorkspaceBackendHandle>,
) -> anyhow::Result<()> {
    registry.register(Arc::new(ShellTool {
        workspace,
        command_shell,
        backend,
    }))
}

struct ShellTool {
    workspace: Workspace,
    command_shell: String,
    /// Shell commands can create or modify files behind the search index's
    /// back; the backend is notified after every command so the next grep
    /// rebuilds instead of missing new files.
    backend: Option<WorkspaceBackendHandle>,
}

#[async_trait::async_trait]
impl ToolExecutor for ShellTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "shell".to_string(),
            description: "Run a shell command in the workspace and return aggregated output."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Shell command string evaluated by the user's configured shell."
                    },
                    "workdir": {
                        "type": "string",
                        "description": "Optional working directory. Relative paths resolve from the workspace root. Defaults to the workspace root."
                    },
                    "timeout_seconds": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 600,
                        "default": DEFAULT_TIMEOUT_SECONDS
                    }
                },
                "required": ["command"],
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
        let workspace = Workspace::from_context_or_fallback(&ctx, &self.workspace)?;
        let args = parse::<ShellArgs>(&call)?;
        let command = args.command.trim().to_string();
        require_nonempty(&command, "command")?;

        let cwd = match args.workdir.as_deref() {
            Some(workdir) => workspace.resolve_existing_workdir(workdir)?,
            _ => workspace.root().to_path_buf(),
        };
        let requested_timeout = args
            .timeout_seconds
            .unwrap_or(DEFAULT_TIMEOUT_SECONDS)
            .clamp(1, MAX_TIMEOUT_SECONDS);
        let timeout = effective_timeout_seconds(requested_timeout, ctx.deadline_remaining_seconds);
        /*
         * Runner-bound threads execute through the remote session; `sh -lc` is
         * used instead of the locally configured shell because the local shell
         * need not exist on the runner.
         */
        let shell = match ctx.handles.remote_workspace.as_ref() {
            Some(_) => "sh".to_string(),
            None => shell_for_context(&ctx, &self.command_shell),
        };
        let started = Instant::now();
        let (exit_code, aggregated_output, timed_out) =
            if let Some(remote) = ctx.handles.remote_workspace.as_ref() {
                let request = RunnerCommandRequest {
                    command_id: call.id.clone(),
                    program: shell.clone(),
                    args: vec!["-lc".to_string(), command.clone()],
                    cwd: Some(cwd.clone()),
                    env: Vec::new(),
                };
                let output = tokio::time::timeout(
                    std::time::Duration::from_secs(timeout),
                    remote.session.run_command(request),
                )
                .await;
                match output {
                    Ok(Ok(output)) => (
                        output.exit_code.unwrap_or(-1),
                        aggregate_output(output.stdout.as_bytes(), output.stderr.as_bytes()),
                        false,
                    ),
                    Ok(Err(err)) => (-1, format!("execution error: {err:?}"), false),
                    Err(_) => (
                        -1,
                        format!("command timed out after {timeout} seconds"),
                        true,
                    ),
                }
            } else {
                let mut process = Command::new(&shell);
                process.args(command_args_for_shell(&shell, &command, true));
                process.current_dir(&cwd).kill_on_drop(true);
                let output =
                    tokio::time::timeout(std::time::Duration::from_secs(timeout), process.output())
                        .await;
                match output {
                    Ok(Ok(output)) => (
                        output
                            .status
                            .code()
                            .unwrap_or_else(|| if output.status.success() { 0 } else { -1 }),
                        aggregate_output(&output.stdout, &output.stderr),
                        false,
                    ),
                    Ok(Err(err)) => (-1, format!("execution error: {err:?}"), false),
                    Err(_) => (
                        -1,
                        format!("command timed out after {timeout} seconds"),
                        true,
                    ),
                }
            };
        if let Some(backend) = self.backend.as_ref() {
            backend.note_external_change();
        }
        let duration_ms = started.elapsed().as_millis() as u64;
        let status = if timed_out {
            "timed_out"
        } else if exit_code == 0 {
            "completed"
        } else {
            "failed"
        };
        let text = format_shell_output(exit_code, duration_ms, &aggregated_output);
        let data = json!({
            "command": command,
            "cwd": self.workspace.display(&cwd),
            "shell": shell,
            "aggregated_output": aggregated_output,
            "exit_code": exit_code,
            "duration_ms": duration_ms,
            "requested_timeout_seconds": requested_timeout,
            "effective_timeout_seconds": timeout,
            "status": status,
            "timed_out": timed_out,
        });

        Ok(result(call, text, data, status != "completed"))
    }
}

#[derive(Deserialize)]
struct ShellArgs {
    command: String,
    workdir: Option<String>,
    timeout_seconds: Option<u64>,
}

fn aggregate_output(stdout: &[u8], stderr: &[u8]) -> String {
    let mut text = String::new();
    if !stdout.is_empty() {
        text.push_str(&String::from_utf8_lossy(stdout));
    }
    if !stderr.is_empty() {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&String::from_utf8_lossy(stderr));
    }
    let text = text.trim_end();
    if text.is_empty() {
        "(no output)".to_string()
    } else {
        text.to_string()
    }
}

fn format_shell_output(exit_code: i32, duration_ms: u64, output: &str) -> String {
    format!(
        "Exit code: {exit_code}\nWall time: {:.3} seconds\nOutput:\n{output}",
        duration_ms as f64 / 1000.0
    )
}

fn effective_timeout_seconds(
    requested_timeout_seconds: u64,
    deadline_remaining_seconds: Option<u64>,
) -> u64 {
    let deadline_timeout = deadline_remaining_seconds.map(|seconds| {
        seconds
            .saturating_sub(DEADLINE_TIMEOUT_RESERVE_SECONDS)
            .max(MIN_DEADLINE_TIMEOUT_SECONDS)
    });
    match deadline_timeout {
        Some(deadline) => requested_timeout_seconds.min(deadline),
        None => requested_timeout_seconds,
    }
}

#[cfg(test)]
mod tests {
    use roder_api::events::{ThreadId, TurnId};
    use roder_api::policy_mode::PolicyMode;
    use roder_api::tools::{
        LocalProcessRunnerHandle, LocalWorkspaceHandle, ToolCall, ToolExecutionContext,
        ToolExecutor,
    };
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn shell_tool_returns_codex_style_aggregated_output() {
        let root = temp_workspace("roder-shell");
        std::fs::create_dir_all(&root).unwrap();
        let tool = ShellTool {
            workspace: Workspace::new(root.clone()).unwrap(),
            command_shell: roder_api::command_shell::default_command_shell(),
            backend: None,
        };

        let command = success_command("hi");
        let result = tool
            .execute(context(&root), call(json!({ "command": command })))
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.text.contains("Exit code: 0"));
        assert!(result.text.contains("Output:\nhi"));
        assert_eq!(result.data["command"], command);
        assert_eq!(result.data["aggregated_output"], "hi");
        assert_eq!(result.data["status"], "completed");

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn shell_tool_does_not_snapshot_workspace_changes_as_hunks() {
        let root = temp_workspace("roder-shell-hunks");
        std::fs::create_dir_all(root.join("src/routes")).unwrap();
        std::fs::write(root.join("src/routes/index.tsx"), "old title\n").unwrap();
        let tool = ShellTool {
            workspace: Workspace::new(root.clone()).unwrap(),
            command_shell: roder_api::command_shell::default_command_shell(),
            backend: None,
        };

        let result = tool
            .execute(
                context(&root),
                call(json!({
                    "command": "node -e \"require('fs').writeFileSync('src/routes/index.tsx', 'new title\\n')\""
                })),
            )
            .await
            .unwrap();

        assert!(result.data.get("hunks").is_none());

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn shell_tool_marks_nonzero_exit_as_error() {
        let root = temp_workspace("roder-shell-fail");
        std::fs::create_dir_all(&root).unwrap();
        let tool = ShellTool {
            workspace: Workspace::new(root.clone()).unwrap(),
            command_shell: roder_api::command_shell::default_command_shell(),
            backend: None,
        };

        let result = tool
            .execute(
                context(&root),
                call(json!({ "command": failure_command("nope", 7) })),
            )
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.text.contains("Exit code: 7"));
        assert!(result.text.contains("nope"));
        assert_eq!(result.data["exit_code"], 7);
        assert_eq!(result.data["status"], "failed");

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn shell_tool_accepts_workspace_root_workdir_variants() {
        let root = temp_workspace("roder-shell-workdir");
        std::fs::create_dir_all(&root).unwrap();
        let tool = ShellTool {
            workspace: Workspace::new(root.clone()).unwrap(),
            command_shell: roder_api::command_shell::default_command_shell(),
            backend: None,
        };

        for workdir in [".", "./", " . /", "'.'", "` . / `"] {
            let result = tool
                .execute(
                    context(&root),
                    call(json!({ "command": success_command("ok"), "workdir": workdir })),
                )
                .await
                .unwrap();
            assert!(!result.is_error, "workdir {workdir:?}: {}", result.text);
            assert_eq!(result.data["status"], "completed");
        }

        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shell_tool_uses_context_command_shell() {
        let root = temp_workspace("roder-shell-context-shell");
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
        let tool = ShellTool {
            workspace: Workspace::new(root.clone()).unwrap(),
            command_shell: "bash".to_string(),
            backend: None,
        };

        let result = tool
            .execute(
                context(&root).with_command_shell(shell.display().to_string()),
                call(json!({ "command": "printf ok" })),
            )
            .await
            .unwrap();

        assert!(!result.is_error);
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
        assert_eq!(effective_timeout_seconds(120, None), 120);
        assert_eq!(effective_timeout_seconds(120, Some(90)), 60);
        assert_eq!(effective_timeout_seconds(5, Some(90)), 5);
        assert_eq!(effective_timeout_seconds(120, Some(5)), 1);
    }

    #[tokio::test]
    async fn shell_tool_clamps_timeout_to_deadline_remaining() {
        let root = temp_workspace("roder-shell-deadline");
        std::fs::create_dir_all(&root).unwrap();
        let tool = ShellTool {
            workspace: Workspace::new(root.clone()).unwrap(),
            command_shell: roder_api::command_shell::default_command_shell(),
            backend: None,
        };

        let result = tool
            .execute(
                context(&root).with_deadline_remaining_seconds(1),
                call(json!({ "command": sleep_command(2) })),
            )
            .await
            .unwrap();

        assert!(result.is_error);
        assert_eq!(result.data["status"], "timed_out");
        assert_eq!(result.data["timed_out"], true);
        assert_eq!(result.data["effective_timeout_seconds"], 1);

        let _ = std::fs::remove_dir_all(root);
    }

    fn call(arguments: serde_json::Value) -> ToolCall {
        ToolCall {
            id: "call-shell".to_string(),
            name: "shell".to_string(),
            arguments,
            raw_arguments: "{}".to_string(),
            thread_id: "thread-shell".to_string(),
            turn_id: "turn-shell".to_string(),
        }
    }

    fn context(workspace: &std::path::Path) -> ToolExecutionContext {
        ToolExecutionContext::new(
            ThreadId::from("thread-shell"),
            TurnId::from("turn-shell"),
            PolicyMode::Default,
        )
        .with_workspace_handle(Arc::new(LocalWorkspaceHandle::new(workspace)))
        .with_process_runner(Arc::new(LocalProcessRunnerHandle))
    }

    #[tokio::test]
    async fn shell_tool_routes_remote_commands_through_runner_session() {
        let root = temp_workspace("roder-shell-remote");
        std::fs::create_dir_all(&root).unwrap();
        let state =
            std::sync::Arc::new(crate::remote_test_support::RecordingRunnerState::default());
        let ctx = context(&root).with_remote_workspace(Arc::new(
            roder_api::remote_runner::RemoteWorkspace {
                session: Arc::new(crate::remote_test_support::RecordingRunnerSession {
                    state: state.clone(),
                }),
                root: "/sandbox/workspace".into(),
                read_roots: Vec::new(),
            },
        ));
        let tool = ShellTool {
            workspace: Workspace::new(root.clone()).unwrap(),
            command_shell: roder_api::command_shell::default_command_shell(),
            backend: None,
        };

        let result = tool
            .execute(
                ctx,
                call(json!({ "command": "echo hi", "workdir": "apps/web" })),
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.text.contains("remote ok"));
        let commands = state.commands.lock().unwrap();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].program, "sh");
        assert_eq!(
            commands[0].args,
            vec!["-lc".to_string(), "echo hi".to_string()]
        );
        // cwd resolves against the runner workspace root, not the local cwd.
        assert_eq!(
            commands[0].cwd.as_deref(),
            Some(std::path::Path::new("/sandbox/workspace/apps/web"))
        );

        let _ = std::fs::remove_dir_all(root);
    }

    fn temp_workspace(prefix: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}"))
    }

    fn success_command(output: &str) -> String {
        if cfg!(windows) {
            format!("[Console]::Out.Write('{output}')")
        } else {
            format!("printf {output}")
        }
    }

    fn failure_command(output: &str, exit_code: i32) -> String {
        if cfg!(windows) {
            format!("[Console]::Error.Write('{output}'); exit {exit_code}")
        } else {
            format!("echo {output} >&2; exit {exit_code}")
        }
    }

    fn sleep_command(seconds: u64) -> String {
        if cfg!(windows) {
            format!("Start-Sleep -Seconds {seconds}")
        } else {
            format!("sleep {seconds}")
        }
    }
}
